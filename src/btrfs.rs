use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsFd;
use std::path::{Path, PathBuf};

use btrfs_disk::items::{FileExtentBody, FileExtentItem};
use btrfs_uapi::raw::BTRFS_EXTENT_DATA_KEY;
use btrfs_uapi::tree_search::{SearchFilter, tree_search_v2};
use rayon::prelude::*;

use crate::config::Config;
use crate::pacman::PackageEntry;

/// Result of probing whether a path is on btrfs.
pub enum BtrfsStatus {
    Yes,
    No,
    PermissionDenied,
}

/// The key range used to probe for btrfs support.
///
/// `tree_search_v2` walks its whole range item by item, so this range must be
/// empty: objectid 0 is never allocated in a subvolume's fs tree (the first
/// free objectid is 256), so the kernel answers a single ioctl with zero
/// items. A whole-subvolume range here would sweep every extent item on every
/// `--btrfs` run — including the targeted queries whose entire point is to
/// avoid a sweep.
fn probe_filter() -> SearchFilter {
    SearchFilter::for_objectid_range(0, BTRFS_EXTENT_DATA_KEY, 0, 0)
}

/// Probe whether `root` is on a btrfs filesystem by attempting the tree
/// search ioctl on the root directory.
pub fn detect_btrfs(root: &Path) -> BtrfsStatus {
    let file = match File::open(root) {
        Ok(f) => f,
        Err(_) => return BtrfsStatus::No,
    };
    match tree_search_v2(file.as_fd(), probe_filter(), Some(4096), |_, _| Ok(())) {
        Ok(()) => BtrfsStatus::Yes,
        Err(nix::errno::Errno::ENOTTY)
        | Err(nix::errno::Errno::ENOPROTOOPT)
        | Err(nix::errno::Errno::ENOSYS) => BtrfsStatus::No,
        Err(nix::errno::Errno::EPERM) => BtrfsStatus::PermissionDenied,
        Err(_) => BtrfsStatus::Yes,
    }
}

/// Return the btrfs on-disk size (from file extents) for each package entry.
///
/// This is the physical disk usage of every extent the package's files
/// reference — compressed or not — i.e. what `compsize` reports as "Disk
/// Usage".
///
/// Hybrid, because the `BTRFS_IOC_TREE_SEARCH` ioctl is ~1-2 ms per call and
/// serialises in the kernel:
/// - **full scan** (no explicit package names): one bulk sweep of the
///   subvolume's extents, building an inode→bytes table (`compsize`-style);
/// - **targeted query** (package names given): read extents per file, whose
///   cost scales with the (small) number of requested files.
pub(crate) fn compressed_sizes(
    entries: &[PackageEntry],
    config: &Config,
) -> (Vec<Option<u64>>, Option<InodeMap>, Vec<String>) {
    if config.targets.is_empty() {
        match build_inode_map(&config.root) {
            Ok(map) => (sweep_sizes(entries, config, &map), Some(map), Vec::new()),
            Err(e) => (vec![None; entries.len()], None, vec![e]),
        }
    } else {
        let mut sizes = Vec::with_capacity(entries.len());
        let mut errors = Vec::new();
        for entry in entries {
            let (size, mut errs) = package_disk_usage(entry, config);
            sizes.push(size);
            errors.append(&mut errs);
        }
        (sizes, None, errors)
    }
}

/// Extents recorded for one inode during a sweep.
#[derive(Default)]
pub(crate) struct InodeExtents {
    /// Total inline (in-tree) data size.
    inline: u64,
    /// `(disk_bytenr, disk_num_bytes)` for each non-hole regular extent.
    extents: Vec<(u64, u64)>,
}

/// Inode → extents table built by a full-subvolume sweep.
pub(crate) type InodeMap = HashMap<u64, InodeExtents>;

/// Resolve a manifest path (relative paths are rooted at `root`).
fn resolve_path(root: &Path, file_path: &Path) -> PathBuf {
    if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        root.join(file_path)
    }
}

/// pacman's `.FILESYSTEM` marker is not a real file and must be ignored.
fn is_filesystem_marker(path: &Path) -> bool {
    path.file_name()
        .map(|n| n == ".FILESYSTEM")
        .unwrap_or(false)
}

/// One pass over the subvolume's `EXTENT_DATA` items, producing
/// `inode -> extents` (holes recorded as an empty entry, inline counted).
/// `Err` if the sweep cannot be run or fails partway.
fn build_inode_map(root: &Path) -> Result<HashMap<u64, InodeExtents>, String> {
    let file = File::open(root).map_err(|e| format!("open {}: {}", root.display(), e))?;
    let filter = SearchFilter::for_type(0, BTRFS_EXTENT_DATA_KEY);
    let mut map: HashMap<u64, InodeExtents> = HashMap::new();
    tree_search_v2(
        file.as_fd(),
        filter,
        None,
        |hdr, data: &[u8]| -> std::result::Result<(), nix::errno::Errno> {
            // A compound-key search can return other item types; keep only
            // extent-data items.
            if hdr.item_type == BTRFS_EXTENT_DATA_KEY
                && let Some(extent) = FileExtentItem::parse(data)
            {
                let entry = map.entry(hdr.objectid).or_default();
                match &extent.body {
                    FileExtentBody::Regular {
                        disk_bytenr,
                        disk_num_bytes,
                        ..
                    } => {
                        if *disk_bytenr != 0 {
                            entry.extents.push((*disk_bytenr, *disk_num_bytes));
                        }
                    }
                    FileExtentBody::Inline { inline_size } => {
                        entry.inline += *inline_size as u64;
                    }
                }
            }
            Ok(())
        },
    )
    .map_err(|e| format!("extent sweep: {e}"))?;
    Ok(map)
}

/// Sum a package's on-disk bytes from a prebuilt inode map. Hardlinked paths
/// are counted once, and a physical extent shared by two of the package's
/// inodes is counted once. Files not on the swept subvolume are skipped.
/// `None` when none of the package's files were found in the map.
fn sum_package_files(
    files: &[PathBuf],
    root: &Path,
    root_dev: Option<u64>,
    map: &HashMap<u64, InodeExtents>,
) -> Option<u64> {
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut seen_bytenr: HashSet<u64> = HashSet::new();
    let mut total = 0u64;
    let mut found = false;

    for file_path in files {
        if is_filesystem_marker(file_path) {
            continue;
        }
        let full = resolve_path(root, file_path);
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        if root_dev.is_some() && Some(meta.dev()) != root_dev {
            // On a different subvolume than the one we swept.
            continue;
        }
        let ino = meta.ino();
        if seen_inodes.insert((meta.dev(), ino))
            && let Some(extents) = map.get(&ino)
        {
            found = true;
            total += extents.inline;
            for &(disk_bytenr, disk_num_bytes) in &extents.extents {
                if seen_bytenr.insert(disk_bytenr) {
                    total += disk_num_bytes;
                }
            }
        }
    }

    found.then_some(total)
}

/// Sweep-based sizes for a full scan.
///
/// The extent sweep itself is serial (the ioctl serialises), but the
/// per-package summation is just `stat` + map lookups and is parallelised.
fn sweep_sizes(
    entries: &[PackageEntry],
    config: &Config,
    map: &HashMap<u64, InodeExtents>,
) -> Vec<Option<u64>> {
    let root_dev = std::fs::metadata(&config.root).ok().map(|m| m.dev());
    entries
        .par_iter()
        .map(|entry| sum_package_files(&entry.files, &config.root, root_dev, map))
        .collect()
}

/// Raw extent records for one file, before cross-file dedup.
#[derive(Default)]
struct FileExtents {
    /// Device and inode of the file (0 when it could not be read); together
    /// they identify the file for hardlink dedup.
    dev: u64,
    ino: u64,
    /// `(disk_bytenr, disk_num_bytes)` for each regular extent; holes appear as
    /// `disk_bytenr == 0`.
    regular: Vec<(u64, u64)>,
    /// Total inline (in-tree) data size.
    inline_bytes: u64,
    /// Whether any extent item was seen for this file.
    found: bool,
    /// Set when the extent ioctl failed, so the caller can report it rather
    /// than silently treat the file as empty.
    error: Option<String>,
}

/// Read one file's extent records via the btrfs tree-search ioctl.
fn file_extents(file_path: &Path, root: &Path) -> FileExtents {
    let full = resolve_path(root, file_path);
    let mut out = FileExtents::default();

    // Skip symlinks and other non-regular entries, matching the scanner
    // (`stat_package`) and the sweep path. `File::open` below would otherwise
    // follow a symlink and attribute the target's extents to this package.
    match std::fs::symlink_metadata(&full) {
        Ok(meta) if meta.is_file() => {}
        _ => return out,
    }

    let file = match File::open(&full) {
        Ok(f) => f,
        Err(_) => return out,
    };
    let meta = match file.metadata() {
        Ok(m) => m,
        Err(_) => return out,
    };
    if !meta.is_file() {
        return out;
    }

    let ino = meta.ino();
    out.dev = meta.dev();
    out.ino = ino;
    let filter = SearchFilter::for_objectid_range(0, BTRFS_EXTENT_DATA_KEY, ino, ino);

    if let Err(e) = tree_search_v2(
        file.as_fd(),
        filter,
        None,
        |_hdr, data: &[u8]| -> std::result::Result<(), nix::errno::Errno> {
            if let Some(extent) = FileExtentItem::parse(data) {
                out.found = true;
                match &extent.body {
                    FileExtentBody::Regular {
                        disk_bytenr,
                        disk_num_bytes,
                        ..
                    } => out.regular.push((*disk_bytenr, *disk_num_bytes)),
                    FileExtentBody::Inline { inline_size } => {
                        out.inline_bytes += *inline_size as u64;
                    }
                }
            }
            Ok(())
        },
    ) {
        // ENOTTY/ENOSYS/ENOPROTOOPT mean the file is not on btrfs (e.g. another
        // mount such as /boot on vfat): that is "no data", not a failure.
        if !matches!(
            e,
            nix::errno::Errno::ENOTTY | nix::errno::Errno::ENOPROTOOPT | nix::errno::Errno::ENOSYS
        ) {
            out.error = Some(e.to_string());
        }
    }

    out
}

/// Sum a package's on-disk usage from its files' extent records: skip holes
/// (`disk_bytenr == 0`), count each physical extent once, add inline data.
/// `None` when no extent was found (e.g. every file was unreadable).
fn total_from_files(files: &[FileExtents]) -> Option<u64> {
    let mut seen: HashSet<u64> = HashSet::new();
    let mut total = 0u64;
    let mut has_extent = false;

    for file in files {
        has_extent |= file.found;
        total += file.inline_bytes;
        for &(disk_bytenr, disk_num_bytes) in &file.regular {
            if disk_bytenr != 0 && seen.insert(disk_bytenr) {
                total += disk_num_bytes;
            }
        }
    }

    has_extent.then_some(total)
}

/// Total btrfs on-disk usage of one package (targeted path).
///
/// Hardlinked inodes are deduplicated before summing so this agrees with the
/// per-file tree totals from [`package_file_disk_sizes`].
fn package_disk_usage(entry: &PackageEntry, config: &Config) -> (Option<u64>, Vec<String>) {
    let root_dev = std::fs::metadata(&config.root).ok().map(|m| m.dev());
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut errors = Vec::new();
    let files: Vec<FileExtents> = entry
        .files
        .iter()
        .filter(|file_path| !is_filesystem_marker(file_path))
        .map(|file_path| {
            let ext = file_extents(file_path, &config.root);
            if let Some(err) = &ext.error {
                errors.push(format!("{}: {}", file_path.display(), err));
            }
            ext
        })
        .filter(|ext| {
            // Only files on the root's subvolume count (as in the sweep path).
            ext.ino != 0
                && root_dev.is_none_or(|dev| ext.dev == dev)
                && seen_inodes.insert((ext.dev, ext.ino))
        })
        .collect();
    (total_from_files(&files), errors)
}

/// Per-file btrfs on-disk bytes for one package, aligned with `entry.files`.
///
/// Uses the subvolume-wide inode `map` when available (full scan), otherwise
/// reads each file's extents directly (targeted query). Hardlinked inodes and
/// physical extents shared between the package's files are attributed to their
/// first occurrence, so the returned vector sums to the package total. The
/// second value is that total (`None` when no extent was found).
pub(crate) fn package_file_disk_sizes(
    entry: &PackageEntry,
    config: &Config,
    map: Option<&InodeMap>,
) -> (Vec<u64>, Option<u64>, Vec<String>) {
    let root_dev = std::fs::metadata(&config.root).ok().map(|m| m.dev());
    let mut sizes = vec![0u64; entry.files.len()];
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut seen_bytenr: HashSet<u64> = HashSet::new();
    let mut total = 0u64;
    let mut found = false;
    let mut errors = Vec::new();

    for (i, file_path) in entry.files.iter().enumerate() {
        if is_filesystem_marker(file_path) {
            continue;
        }
        let full = resolve_path(&config.root, file_path);
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        if let Some(dev) = root_dev {
            // The map only covers the swept subvolume.
            if meta.dev() != dev {
                continue;
            }
        }
        let dev = meta.dev();
        let ino = meta.ino();
        if !seen_inodes.insert((dev, ino)) {
            continue;
        }

        let mut bytes = 0u64;
        let mut this_found = false;
        match map {
            Some(map) => {
                if let Some(ext) = map.get(&ino) {
                    this_found = true;
                    bytes += ext.inline;
                    for &(disk_bytenr, disk_num_bytes) in &ext.extents {
                        if disk_bytenr != 0 && seen_bytenr.insert(disk_bytenr) {
                            bytes += disk_num_bytes;
                        }
                    }
                }
            }
            None => {
                let ext = file_extents(file_path, &config.root);
                if let Some(err) = &ext.error {
                    errors.push(format!("{}: {}", file_path.display(), err));
                }
                if ext.found {
                    this_found = true;
                    bytes += ext.inline_bytes;
                    for &(disk_bytenr, disk_num_bytes) in &ext.regular {
                        if disk_bytenr != 0 && seen_bytenr.insert(disk_bytenr) {
                            bytes += disk_num_bytes;
                        }
                    }
                }
            }
        }

        if this_found {
            found = true;
            sizes[i] = bytes;
            total += bytes;
        }
    }

    (sizes, found.then_some(total), errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(regular: &[(u64, u64)], inline_bytes: u64, found: bool) -> FileExtents {
        FileExtents {
            dev: 0,
            ino: 0,
            regular: regular.to_vec(),
            inline_bytes,
            found,
            error: None,
        }
    }

    #[test]
    fn test_probe_filter_is_an_empty_objectid_range() {
        // Contract: the btrfs probe must not sweep the subvolume. The crate
        // walks the entire key range, so a probe built from `for_type`
        // (objectid 0..u64::MAX) reads every extent item in the filesystem
        // before the scan does anything useful.
        let filter = probe_filter();
        assert_eq!(filter.tree_id, 0);
        assert_eq!(filter.start.objectid, 0);
        assert_eq!(filter.end.objectid, 0, "probe must cover a single objectid");
        assert_eq!(filter.start.item_type, filter.end.item_type);
    }

    #[test]
    fn test_total_from_files_counts_and_dedups() {
        // Two extents, with the first repeated across files (a shared/reflinked
        // physical extent) -> counted once.
        let files = vec![
            file(&[(100, 4096), (200, 8192)], 0, true),
            file(&[(100, 4096)], 0, true),
        ];
        assert_eq!(total_from_files(&files), Some(4096 + 8192));
    }

    #[test]
    fn test_total_from_files_skips_holes_and_counts_inline() {
        // disk_bytenr == 0 is a hole/sparse range: no disk usage; inline is counted.
        let files = vec![file(&[(0, 65536)], 50, true)];
        assert_eq!(total_from_files(&files), Some(50));
    }

    #[test]
    fn test_total_from_files_none_when_no_extent() {
        assert_eq!(total_from_files(&[file(&[], 0, false)]), None);
    }

    #[test]
    fn test_sum_package_files_dedups_hardlinks_and_shared_extents() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = dir.join("a");
        let b = dir.join("b");
        let c = dir.join("c_link");
        std::fs::write(&a, b"aaaa").unwrap();
        std::fs::write(&b, b"bbbb").unwrap();
        std::fs::hard_link(&a, &c).unwrap();

        let dev = std::fs::metadata(dir).unwrap().dev();
        let ino_a = std::fs::metadata(&a).unwrap().ino();
        let ino_b = std::fs::metadata(&b).unwrap().ino();
        let mut map: HashMap<u64, InodeExtents> = HashMap::new();
        map.insert(
            ino_a,
            InodeExtents {
                // Non-zero inline so the hardlink (same inode) must be deduped
                // by inode, not merely by shared bytenr.
                inline: 7,
                extents: vec![(500, 4096), (501, 8192)],
            },
        );
        map.insert(
            ino_b,
            InodeExtents {
                inline: 10,
                extents: vec![(500, 4096)], // shares bytenr 500 with `a`
            },
        );

        // `a` and its hardlink share an inode (counted once); bytenr 500 is
        // shared between `a` and `b` (counted once); inline 10 is added.
        let files = vec![a.clone(), b.clone(), c.clone()];
        assert_eq!(
            sum_package_files(&files, dir, Some(dev), &map),
            Some(4096 + 8192 + 7 + 10)
        );

        // Nothing in the map -> None.
        let empty: HashMap<u64, InodeExtents> = HashMap::new();
        assert_eq!(sum_package_files(&files, dir, Some(dev), &empty), None);
    }

    fn test_config(root: &Path) -> Config {
        Config {
            root: root.to_path_buf(),
            dbpath: PathBuf::from("/var/lib/pacman"),
            targets: vec![],
            search: None,
            sort: crate::config::SortField::Real,
            limit: None,
            btrfs: true,
            verbose: false,
            format: None,
            humansize: None,
            delim: "\n".to_string(),
            no_color: true,
            apparent_size: false,
            total: false,
            files: false,
            depth: None,
            breadth: 5,
            min_percent: 0.0,
            with_deps: false,
        }
    }

    #[test]
    fn test_package_file_disk_sizes_from_map() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = dir.join("a");
        let b = dir.join("b");
        let link = dir.join("a_link");
        std::fs::write(&a, b"aaaa").unwrap();
        std::fs::write(&b, b"bbbb").unwrap();
        std::fs::hard_link(&a, &link).unwrap();

        let ino_a = std::fs::metadata(&a).unwrap().ino();
        let ino_b = std::fs::metadata(&b).unwrap().ino();
        let mut map: InodeMap = HashMap::new();
        map.insert(
            ino_a,
            InodeExtents {
                inline: 7,
                extents: vec![(500, 4096)],
            },
        );
        map.insert(
            ino_b,
            InodeExtents {
                inline: 10,
                // bytenr 500 is shared with `a` -> counted once.
                extents: vec![(500, 4096), (501, 8192)],
            },
        );

        let entry = PackageEntry {
            name: "pkg".to_string(),
            version: "1.0-1".to_string(),
            metadata_size: 0,
            files: vec![a.clone(), b.clone(), link.clone()],
            depends: Vec::new(),
            provides: Vec::new(),
            explicit: false,
        };
        let config = test_config(dir);
        let (sizes, total, _errors) = package_file_disk_sizes(&entry, &config, Some(&map));

        // a: inline 7 + extent 500 (4096); b: inline 10 + extent 501 (8192)
        // (500 already seen); the hardlink shares a's inode -> 0.
        assert_eq!(sizes, vec![4103, 8202, 0]);
        assert_eq!(total, Some(4103 + 8202));
    }

    #[test]
    fn test_file_extents_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let target = dir.join("target");
        std::fs::write(&target, b"data").unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink("target", &link).unwrap();

        // The symlink is skipped (ino stays 0) rather than following to `target`.
        let ext = file_extents(&link, dir);
        assert_eq!(ext.ino, 0, "symlinks must be skipped");
        assert!(!ext.found);

        // A regular file is still read (ino is set even without btrfs extents).
        assert_ne!(file_extents(&target, dir).ino, 0);
    }

    #[test]
    fn test_file_extents_not_on_btrfs_is_not_an_error() {
        // A regular file on a non-btrfs filesystem (e.g. /boot on vfat): the
        // ioctl fails with ENOTTY, which means "no data here", not a failure.
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        std::fs::write(&f, b"data").unwrap();

        let ext = file_extents(&f, tmp.path());
        // On btrfs the ioctl succeeds; otherwise the file is skipped silently.
        if !ext.found {
            assert!(
                ext.error.is_none(),
                "a non-btrfs file must not be reported as an error: {:?}",
                ext.error
            );
        }
    }
}
