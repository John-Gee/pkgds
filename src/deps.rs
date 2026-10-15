//! `--with-deps`: which dependencies removing the target package(s) would free.
//!
//! This mirrors `pacman -Rs`: the targets plus every dependency that is not
//! explicitly installed and is no longer required by any remaining package.
//! Shared dependencies (glibc and friends) are therefore *not* counted, and
//! neither are dependencies other installed packages still need.

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
    by_name: &HashMap<&str, usize>,
    by_provides: &HashMap<String, usize>,
) -> Option<usize> {
    let name = bare_dep_name(spec);
    if let Some(&i) = by_name.get(name) {
        return Some(i);
    }
    by_provides.get(name).copied()
}

/// Given every installed package and the selected target indices, return the
/// indices that `pacman -Rs` would remove: the targets plus each dependency
/// that is not explicitly installed and whose every requirer is also removed.
///
/// Version constraints are ignored — the system is installed, so pacman has
/// already satisfied them. When several packages provide a dependency the first
/// is used (a rare case, and a deliberate simplification).
pub fn removable_with_deps(entries: &[PackageEntry], selected: &[usize]) -> Vec<usize> {
    let mut by_name: HashMap<&str, usize> = HashMap::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        by_name.insert(e.name.as_str(), i);
    }
    let mut by_provides: HashMap<String, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        for p in &e.provides {
            by_provides.entry(bare_dep_name(p).to_string()).or_insert(i);
        }
    }

    // Transitive dependency closure of the selected set.
    let mut closure: HashSet<usize> = HashSet::new();
    let mut stack: Vec<usize> = selected.to_vec();
    while let Some(x) = stack.pop() {
        if !closure.insert(x) {
            continue;
        }
        for dep in &entries[x].depends {
            if let Some(y) = resolve(dep, &by_name, &by_provides) {
                if !closure.contains(&y) {
                    stack.push(y);
                }
            }
        }
    }

    // Who directly requires each package (including via `provides`).
    let mut requirers: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    for (y, e) in entries.iter().enumerate() {
        for dep in &e.depends {
            if let Some(x) = resolve(dep, &by_name, &by_provides) {
                requirers[x].push(y);
            }
        }
    }

    // Remove the targets, then repeatedly remove any non-explicit dependency
    // whose requirers have all been removed, to a fixpoint.
    let mut removed: HashSet<usize> = selected.iter().copied().collect();
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
}
