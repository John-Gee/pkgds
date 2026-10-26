//! `--with-deps`: which dependencies removing the target package(s) would free.
//!
//! Models the disk removing the target(s) would free: the targets plus every
//! dependency that is not explicitly installed and is no longer required by any
//! remaining package. This matches `pacman -Rs` when the removal is permitted;
//! pacman refuses when something outside the set still needs the target, and we
//! report the footprint anyway (the estimation is the useful number). Shared
//! dependencies (glibc and friends) are therefore *not* counted, and neither
//! are dependencies other installed packages still need.

use std::collections::{HashMap, HashSet};

use crate::pacman::PackageEntry;

/// Bare dependency name: strip a version constraint, so `glibc>=2.34` becomes
/// `glibc` and `libalpm.so=15-64` becomes `libalpm.so`.
pub fn bare_dep_name(spec: &str) -> &str {
    let end = spec.find(['<', '>', '=']).unwrap_or(spec.len());
    spec[..end].trim()
}

/// Index of the installed package satisfying a dependency spec: an exact name
/// match first, then a package that provides the name.
fn resolve(
    spec: &str,
    by_name: &HashMap<String, usize>,
    by_provides: &HashMap<String, usize>,
) -> Option<usize> {
    let name = bare_dep_name(spec);
    if let Some(&i) = by_name.get(name) {
        return Some(i);
    }
    by_provides.get(name).copied()
}

/// Shared dependency graph built once from the installed package list.
///
/// Holds the resolved dependency graph, so repeated queries (one per package
/// in bare ranking mode) do not rebuild it. Built from `entries`; indices
/// refer to positions in that slice.
pub struct DepsIndex {
    /// Resolved direct dependencies per package (unknown specs dropped).
    adjacency: Vec<Vec<usize>>,
    /// Direct requirers per package (reverse of adjacency, via provides too).
    requirers: Vec<Vec<usize>>,
}

impl DepsIndex {
    /// Build the index once from the installed package list.
    pub fn build(entries: &[PackageEntry]) -> Self {
        let mut by_name: HashMap<String, usize> = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_name.insert(e.name.clone(), i);
        }
        let mut by_provides: HashMap<String, usize> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            for p in &e.provides {
                let key = bare_dep_name(p).to_string();
                match by_provides.get(&key) {
                    // Keep the alphabetically-first provider, whatever the order.
                    Some(&cur) if entries[cur].name <= e.name => {}
                    _ => {
                        by_provides.insert(key, i);
                    }
                }
            }
        }

        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
        for (i, e) in entries.iter().enumerate() {
            for dep in &e.depends {
                if let Some(y) = resolve(dep, &by_name, &by_provides) {
                    adjacency[i].push(y);
                }
            }
        }

        let mut requirers: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
        for (y, deps) in adjacency.iter().enumerate() {
            for &x in deps {
                requirers[x].push(y);
            }
        }

        DepsIndex {
            adjacency,
            requirers,
        }
    }

    /// Return the indices that removing `selected` would free, using the
    /// prebuilt graph. Same result as `removable_with_deps`.
    pub fn removable(&self, entries: &[PackageEntry], selected: &[usize]) -> Vec<usize> {
        // Transitive dependency closure of the selected set.
        let mut closure: HashSet<usize> = HashSet::new();
        let mut stack: Vec<usize> = selected.to_vec();
        while let Some(x) = stack.pop() {
            if !closure.insert(x) {
                continue;
            }
            for &y in &self.adjacency[x] {
                if !closure.contains(&y) {
                    stack.push(y);
                }
            }
        }

        // Remove the targets, then repeatedly remove any non-explicit
        // dependency whose requirers have all been removed, to a fixpoint.
        let mut removed: HashSet<usize> = selected.iter().copied().collect();
        loop {
            let mut changed = false;
            for &x in &closure {
                if removed.contains(&x) || entries[x].explicit {
                    continue;
                }
                if self.requirers[x].iter().all(|y| removed.contains(y)) {
                    removed.insert(x);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut out: Vec<usize> = removed.into_iter().collect();
        out.sort_unstable();
        out
    }

    /// Solo query for reuse: what removing only `idx` would free.
    pub fn removable_single(&self, entries: &[PackageEntry], idx: usize) -> Vec<usize> {
        self.removable(entries, &[idx])
    }

    /// Solo footprint for ranking: the sum of `base` over what removing only
    /// `idx` would free, or zero when something still installed requires
    /// `idx` (removal would be refused, so nothing is achievable). `base`
    /// must align with `entries` (one value per package).
    pub fn solo_footprint(&self, entries: &[PackageEntry], idx: usize, base: &[u64]) -> u64 {
        let freed = self.removable_single(entries, idx);
        if self.requirers[idx]
            .iter()
            .any(|&y| y != idx && !freed.contains(&y))
        {
            return 0;
        }
        freed.iter().map(|&j| base[j]).sum()
    }
}

/// Given every installed package and the selected target indices, return the
/// indices that removing the targets would free: the targets plus each
/// dependency that is not explicitly installed and whose every requirer is also
/// removed.
///
/// Version constraints are ignored, the system is installed, so pacman has
/// already satisfied them. When several packages provide a dependency the
/// alphabetically-first provider is used, so the result does not depend on
/// directory order (a deliberate simplification).
pub fn removable_with_deps(entries: &[PackageEntry], selected: &[usize]) -> Vec<usize> {
    DepsIndex::build(entries).removable(entries, selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, explicit: bool, depends: &[&str], provides: &[&str]) -> PackageEntry {
        PackageEntry {
            name: name.to_string(),
            version: "1.0-1".to_string(),
            metadata_size: 0,
            files: Vec::new(),
            depends: depends.iter().map(|s| s.to_string()).collect(),
            provides: provides.iter().map(|s| s.to_string()).collect(),
            explicit,
        }
    }

    fn names(entries: &[PackageEntry], idx: &[usize]) -> Vec<String> {
        idx.iter().map(|&i| entries[i].name.clone()).collect()
    }

    #[test]
    fn test_bare_dep_name() {
        assert_eq!(bare_dep_name("glibc>=2.34"), "glibc");
        assert_eq!(bare_dep_name("sh"), "sh");
        assert_eq!(bare_dep_name("libalpm.so=15-64"), "libalpm.so");
        assert_eq!(bare_dep_name("foo<2"), "foo");
    }

    #[test]
    fn test_exclusive_chain_is_removed() {
        let entries = vec![
            pkg("app", true, &["lib"], &[]),
            pkg("lib", false, &["core"], &[]),
            pkg("core", false, &[], &[]),
        ];
        let out = removable_with_deps(&entries, &[0]);
        assert_eq!(names(&entries, &out), vec!["app", "lib", "core"]);
    }

    #[test]
    fn test_shared_dep_is_kept() {
        // `core` is also required by `other`, so removing `app` does not free it.
        let entries = vec![
            pkg("app", true, &["lib"], &[]),
            pkg("lib", false, &["core"], &[]),
            pkg("core", false, &[], &[]),
            pkg("other", true, &["core"], &[]),
        ];
        let out = removable_with_deps(&entries, &[0]);
        assert_eq!(names(&entries, &out), vec!["app", "lib"]);
    }

    #[test]
    fn test_explicit_dep_is_kept() {
        let entries = vec![
            pkg("app", true, &["python"], &[]),
            pkg("python", true, &[], &[]),
        ];
        let out = removable_with_deps(&entries, &[0]);
        assert_eq!(names(&entries, &out), vec!["app"]);
    }

    #[test]
    fn test_provides_satisfies_dependency() {
        let entries = vec![
            pkg("app", true, &["sh"], &[]),
            pkg("bash", false, &[], &["sh"]),
        ];
        let out = removable_with_deps(&entries, &[0]);
        assert_eq!(names(&entries, &out), vec!["app", "bash"]);
    }

    #[test]
    fn test_provider_choice_is_deterministic() {
        // Two providers for `sh`; the alphabetically-first must be chosen
        // regardless of the order they appear in.
        let a = vec![
            pkg("app", true, &["sh"], &[]),
            pkg("zsh", false, &[], &["sh"]),
            pkg("bash", false, &[], &["sh"]),
        ];
        let b = vec![
            pkg("app", true, &["sh"], &[]),
            pkg("bash", false, &[], &["sh"]),
            pkg("zsh", false, &[], &["sh"]),
        ];
        assert_eq!(
            names(&a, &removable_with_deps(&a, &[0])),
            vec!["app", "bash"]
        );
        assert_eq!(
            names(&b, &removable_with_deps(&b, &[0])),
            vec!["app", "bash"]
        );
    }

    #[test]
    fn test_overlapping_requirers_and_multiple_providers() {
        // app1 and app2 both need lib; lib needs `sh` (provided by bash and
        // dash) and core; core is also needed by `other`, so it is kept.
        let entries = vec![
            pkg("app1", true, &["lib"], &[]),
            pkg("app2", true, &["lib"], &[]),
            pkg("lib", false, &["sh", "core"], &[]),
            pkg("bash", false, &[], &["sh"]),
            pkg("dash", false, &[], &["sh"]),
            pkg("core", false, &[], &[]),
            pkg("other", true, &["core"], &[]),
        ];
        // Removing both apps frees lib (both requirers gone) and bash (the
        // alphabetically-first `sh` provider), but not core (other needs it).
        let out = removable_with_deps(&entries, &[0, 1]);
        assert_eq!(names(&entries, &out), vec!["app1", "app2", "lib", "bash"]);
    }

    #[test]
    fn test_cycle_terminates() {
        let entries = vec![pkg("a", true, &["b"], &[]), pkg("b", false, &["a"], &[])];
        let out = removable_with_deps(&entries, &[0]);
        assert_eq!(names(&entries, &out), vec!["a", "b"]);
    }

    #[test]
    fn test_unknown_dependency_is_ignored() {
        let entries = vec![pkg("app", true, &["ghost>=1"], &[])];
        let out = removable_with_deps(&entries, &[0]);
        assert_eq!(names(&entries, &out), vec!["app"]);
    }

    /// Independent reference: resolves each spec by linear scan (exact name,
    /// else alphabetically-first provider) and runs the same removal rule.
    /// Shares only `bare_dep_name` and the fixture shape with the code under
    /// test, so it checks the shared index wiring rather than repeating it.
    fn naive_removable(entries: &[PackageEntry], selected: &[usize]) -> Vec<usize> {
        fn resolve_all(entries: &[PackageEntry], spec: &str) -> Option<usize> {
            let name = bare_dep_name(spec);
            if let Some(i) = entries.iter().position(|e| e.name == name) {
                return Some(i);
            }
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.provides.iter().any(|p| bare_dep_name(p) == name))
                .min_by_key(|(_, e)| &e.name)
                .map(|(i, _)| i)
        }
        let adj: Vec<Vec<usize>> = entries
            .iter()
            .map(|e| {
                e.depends
                    .iter()
                    .filter_map(|d| resolve_all(entries, d))
                    .collect()
            })
            .collect();
        let mut requirers: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
        for (y, deps) in adj.iter().enumerate() {
            for &x in deps {
                requirers[x].push(y);
            }
        }
        let mut closure = std::collections::HashSet::new();
        let mut stack = selected.to_vec();
        while let Some(x) = stack.pop() {
            if closure.insert(x) {
                stack.extend(adj[x].iter().copied().filter(|y| !closure.contains(y)));
            }
        }
        let mut removed: std::collections::HashSet<usize> = selected.iter().copied().collect();
        loop {
            let mut changed = false;
            for &x in &closure {
                if removed.contains(&x) || entries[x].explicit {
                    continue;
                }
                if requirers[x].iter().all(|y| removed.contains(y)) {
                    removed.insert(x);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut out: Vec<usize> = removed.into_iter().collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn test_index_matches_naive_reference_for_every_selection() {
        // Contract: the shared index is only a speed-up. An independent
        // reimplementation must agree with it for every selection, and the
        // result must uphold the removal invariants.
        let entries = vec![
            pkg("app1", true, &["lib"], &[]),
            pkg("app2", true, &["lib"], &[]),
            pkg("lib", false, &["sh", "core"], &[]),
            pkg("bash", false, &[], &["sh"]),
            pkg("dash", false, &[], &["sh"]),
            pkg("core", false, &[], &[]),
            pkg("other", true, &["core"], &[]),
        ];
        let index = DepsIndex::build(&entries);
        for mask in 0..(1usize << entries.len()) {
            let selected: Vec<usize> = (0..entries.len())
                .filter(|i| mask & (1 << i) != 0)
                .collect();
            let out = index.removable(&entries, &selected);
            assert_eq!(
                out,
                naive_removable(&entries, &selected),
                "mismatch for selection {selected:?}"
            );
            for &t in &selected {
                assert!(
                    out.contains(&t),
                    "target {t} must be freed for selection {selected:?}"
                );
            }
            for (i, e) in entries.iter().enumerate() {
                if !selected.contains(&i) && e.explicit {
                    assert!(
                        !out.contains(&i),
                        "explicit dep {} must be kept for selection {selected:?}",
                        e.name
                    );
                }
            }
        }
    }

    #[test]
    fn test_solo_query_matches_single_selection() {
        // Contract: the indexed solo query is the single-target case, so it
        // must match the wrapper and free the exclusive chain.
        let entries = vec![
            pkg("app", true, &["lib"], &[]),
            pkg("lib", false, &["core"], &[]),
            pkg("core", false, &[], &[]),
            pkg("other", true, &["core"], &[]),
        ];
        let index = DepsIndex::build(&entries);
        assert_eq!(
            names(&entries, &index.removable_single(&entries, 0)),
            vec!["app", "lib"]
        );
        assert_eq!(
            index.removable_single(&entries, 0),
            naive_removable(&entries, &[0])
        );
        // A shared dependency is kept by the solo query too.
        assert_eq!(
            names(&entries, &index.removable_single(&entries, 3)),
            vec!["other"]
        );
    }

    #[test]
    fn test_solo_footprint_is_zero_when_still_required() {
        // `core` is required by `other`, which stays: removing only `core`
        // would be refused, so its achievable footprint is zero. `app`'s
        // chain is exclusive, so it keeps its footprint.
        let entries = vec![
            pkg("app", true, &["lib"], &[]),
            pkg("lib", false, &["core"], &[]),
            pkg("core", false, &[], &[]),
            pkg("other", true, &["core"], &[]),
        ];
        let index = DepsIndex::build(&entries);
        let base = vec![10, 20, 40, 80];
        assert_eq!(index.solo_footprint(&entries, 0, &base), 30);
        assert_eq!(index.solo_footprint(&entries, 1, &base), 0);
        assert_eq!(index.solo_footprint(&entries, 2, &base), 0);
        assert_eq!(index.solo_footprint(&entries, 3, &base), 80);
    }
}
