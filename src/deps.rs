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

/// Shared dependency lookup built once from the installed package list.
///
/// Holds the name and provides maps plus the resolved dependency graph, so
/// repeated queries (one per package in bare ranking mode) do not rebuild
/// them. Built from `entries`; indices refer to positions in that slice.
pub struct DepsIndex {
    by_name: HashMap<String, usize>,
    by_provides: HashMap<String, usize>,
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
            by_name,
            by_provides,
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

    /// Look up the installed package satisfying a dependency spec, using the
    /// prebuilt maps. Kept so the shared index owns the full lookup.
    pub fn lookup(&self, spec: &str) -> Option<usize> {
        resolve(spec, &self.by_name, &self.by_provides)
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

    #[test]
    fn test_index_matches_wrapper_for_every_selection() {
        // Contract: the shared index is only a speed-up; it must give the
        // same answer as the one-shot wrapper for every selection.
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
            assert_eq!(
                index.removable(&entries, &selected),
                removable_with_deps(&entries, &selected),
                "mismatch for selection {selected:?}"
            );
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
            removable_with_deps(&entries, &[0])
        );
        // A shared dependency is kept by the solo query too.
        assert_eq!(
            names(&entries, &index.removable_single(&entries, 3)),
            vec!["other"]
        );
    }
}
