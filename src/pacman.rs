use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Fields extracted from a `%desc%` package metadata file.
#[derive(Debug, Clone)]
pub struct DescFields {
    pub name: String,
    pub version: String,
    pub size: u64,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    /// pacman `%REASON%`: 0 = explicitly installed, 1 = installed as a
    /// dependency. `None` when the field is absent (older databases).
    pub reason: Option<u64>,
}

/// Search/filter configuration — by target name(s) and/or a regex search pattern.
#[derive(Debug, Clone)]
pub struct Filter {
    pub targets: Vec<String>,
    pub search: Option<regex::Regex>,
}

/// A single package entry as loaded from the local database.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub metadata_size: u64,
    pub files: Vec<PathBuf>,
    /// Raw `%DEPENDS%` specs (e.g. `glibc>=2.34`, `sh`, `libalpm.so=15-64`).
    pub depends: Vec<String>,
    /// Raw `%PROVIDES%` specs (e.g. `sh`, `cron`).
    pub provides: Vec<String>,
    /// Whether the package was installed explicitly (`%REASON%` 0). Packages
    /// installed as dependencies are eligible for `--with-deps` removal.
    pub explicit: bool,
}

/// Minimal pacman configuration read from `pacman.conf`.
#[derive(Debug, Clone)]
pub struct PacmanConf {
    pub dbpath: PathBuf,
}

/// Parse a pacman desc file and extract %NAME%, %VERSION%, %SIZE% fields.
///
/// The desc format uses `%KEYWORD%` headers followed by value lines.
/// Empty NAME or VERSION returns an error. Missing/unparseable SIZE defaults to 0.
pub fn parse_desc(text: &str) -> crate::error::Result<DescFields> {
    let mut current_key: Option<String> = None;
    let mut values: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for line in text.lines() {
        if let Some(stripped) = line.strip_prefix('%') {
            if stripped.ends_with('%') {
                let key = stripped.strip_suffix('%').unwrap_or("").to_string();
                if !key.is_empty() {
                    current_key = Some(key);
                } else {
                    // `%` followed by `%` — ignore as malformed header
                }
            } else {
                // `%FOOBAR` without closing `%` — treat as value of current key
                if let Some(k) = &current_key {
                    values.entry(k.clone()).or_default().push(line.to_string());
                }
            }
        } else {
            // Value line for the current key
            if let Some(k) = &current_key {
                values.entry(k.clone()).or_default().push(line.to_string());
            }
        }
    }

    // Extract NAME — must be present and non-empty
    let name_lines = values.get("NAME").cloned().unwrap_or_default();
    let name = last_non_empty(&name_lines);
    if name.is_none() || name.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Err(crate::error::PkgdsError::ParsePacmanConf(
            "empty NAME".to_string(),
        ));
    }

    // Extract VERSION — must be present and non-empty
    let version_lines = values.get("VERSION").cloned().unwrap_or_default();
    let version = last_non_empty(&version_lines);
    if version.is_none()
        || version
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(crate::error::PkgdsError::ParsePacmanConf(
            "empty VERSION".to_string(),
        ));
    }

    // Extract SIZE — missing or unparseable → 0
    let size: u64 = values
        .get("SIZE")
        .map(|lines| {
            last_non_empty(lines)
                .unwrap_or_default()
                .trim()
                .parse()
                .unwrap_or(0)
        })
        .unwrap_or(0);

    // Extract DEPENDS / PROVIDES — multi-line lists of raw dependency specs.
    let depends = list_field(&values, "DEPENDS");
    let provides = list_field(&values, "PROVIDES");

    // Extract REASON — 0 = explicit, 1 = dependency; absent on older databases.
    let reason = values
        .get("REASON")
        .and_then(|lines| last_non_empty(lines))
        .and_then(|s| s.trim().parse().ok());

    Ok(DescFields {
        name: name.unwrap(),
        version: version.unwrap(),
        size,
        depends,
        provides,
        reason,
    })
}

/// Collect a multi-line list field (like `%DEPENDS%`), trimming blank lines.
fn list_field(values: &std::collections::HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    values
        .get(key)
        .map(|lines| {
            lines
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Return the last non-empty (trimmed) line from a Vec<String>, or None if all empty.
fn last_non_empty(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .rev()
        .find(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
}

pub fn parse_files(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut in_files = false;
    let mut seen_files = false;

    for line in text.lines() {
        if let Some(stripped) = line.strip_prefix('%') {
            if stripped.ends_with('%') {
                let header = stripped.strip_suffix('%').unwrap_or("");
                if header == "FILES" {
                    in_files = true;
                    seen_files = true;
                } else if in_files {
                    break;
                }
                continue;
            }
        }

        if !seen_files {
            // No %FILES% header seen yet — treat all non-empty lines as paths
            let path = line.trim();
            if path.is_empty() {
                continue;
            }
            if path.ends_with('/') {
                continue;
            }
            paths.push(PathBuf::from(path));
            continue;
        }

        if in_files {
            let path = line.trim();
            if path.is_empty() {
                continue;
            }
            if path.ends_with('/') {
                continue;
            }
            paths.push(PathBuf::from(path));
        }
    }

    paths
}

/// Parse `pacman.conf` and return a minimal `PacmanConf`.
///
/// Parses `key = value` lines (with `#` comments). Extracts the `DBPath` key;
/// if the file is missing, does not exist, or lacks a valid `DBPath`, returns
/// the default dbpath `/var/lib/pacman` silently.
pub fn parse_pacman_conf(path: &Path) -> crate::error::Result<PacmanConf> {
    let default_db = PathBuf::from("/var/lib/pacman");

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(PacmanConf { dbpath: default_db }),
    };

    let mut dbpath: Option<PathBuf> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments (lines starting with #) and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Check for [section] headers — skip them
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            continue;
        }

        // Parse key = value
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            let value = trim_comment(trimmed[eq_pos + 1..].trim());

            if key.eq_ignore_ascii_case("dbpath") && !value.is_empty() {
                dbpath = Some(PathBuf::from(value));
            }
        }
    }

    Ok(PacmanConf {
        dbpath: dbpath.unwrap_or(default_db),
    })
}

/// Strip a trailing inline comment (starting with `#`) from a config value.
/// Strips if the `#` is preceded by whitespace or appears at position 0.
fn trim_comment(s: &str) -> &str {
    match s.rfind('#') {
        Some(0) => "",
        Some(idx) => {
            if s[..idx].ends_with(char::is_whitespace) {
                s[..idx].trim_end()
            } else {
                s
            }
        }
        None => s,
    }
}

/// Check whether a package should be included per the filter.
pub fn matches_filter(name: &str, filter: &Filter) -> bool {
    let targets_match = if filter.targets.is_empty() {
        true
    } else {
        filter.targets.iter().any(|t| t == name)
    };
    let search_match = match &filter.search {
        None => true,
        Some(re) => re.is_match(name),
    };
    targets_match && search_match
}

/// Load packages from a local pacman database directory.
///
/// Walks `{dbpath}/local`, parses desc and files for each subdirectory,
/// applies the filter, and returns (entries, skipped_count, errors).
pub fn load_local_db(
    dbpath: &Path,
    filter: &Filter,
) -> crate::error::Result<(Vec<PackageEntry>, usize, Vec<String>)> {
    let local_dir = dbpath.join("local");
    let mut entries = Vec::new();
    let mut skipped = 0usize;
    let mut errors = Vec::new();

    let dirs = match std::fs::read_dir(&local_dir) {
        Ok(d) => d,
        Err(e) => return Err(crate::error::PkgdsError::Io(e)),
    };

    for entry_result in dirs {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                skipped += 1;
                errors.push(format!("Failed to read a database entry: {}", e));
                continue;
            }
        };

        let path = entry.path();

        // Skip non-directories. ALPM_DB_VERSION is a file, so this also skips
        // it -- nothing further to do for it.
        if !path.is_dir() {
            continue;
        }

        let basename = path.file_name().map(|s| s.to_string_lossy().into_owned());

        let desc_path = path.join("desc");
        let files_path = path.join("files");

        // Every package directory has a desc; one without is a corrupt entry
        // that must be counted, not silently skipped.
        if !desc_path.is_file() {
            skipped += 1;
            errors.push(format!(
                "No desc file for {}",
                basename.clone().unwrap_or_default()
            ));
            continue;
        }

        // Read and parse desc
        let desc_content = match std::fs::read_to_string(&desc_path) {
            Ok(c) => c,
            Err(_) => {
                skipped += 1;
                errors.push(format!(
                    "Failed to read desc for {}",
                    basename.unwrap_or_default()
                ));
                continue;
            }
        };

        let desc_fields = match parse_desc(&desc_content) {
            Ok(f) => f,
            Err(e) => {
                skipped += 1;
                errors.push(format!(
                    "Malformed desc for {}: {}",
                    basename.unwrap_or_default(),
                    e
                ));
                continue;
            }
        };

        // Read and parse files. A missing manifest is an empty package; an
        // unreadable one is a corrupt entry we must not silently report as 0.
        let files = match std::fs::read_to_string(&files_path) {
            Ok(c) => parse_files(&c),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                skipped += 1;
                errors.push(format!(
                    "Failed to read files for {}: {}",
                    desc_fields.name, e
                ));
                continue;
            }
        };

        // Apply filter
        if !matches_filter(&desc_fields.name, filter) {
            continue;
        }

        entries.push(PackageEntry {
            name: desc_fields.name,
            version: desc_fields.version,
            metadata_size: desc_fields.size,
            files,
            depends: desc_fields.depends,
            provides: desc_fields.provides,
            explicit: desc_fields.reason != Some(1),
        });
    }

    Ok((entries, skipped, errors))
}

/// Name of a package derived from its local DB directory (`{name}-{ver}-{rel}`).
/// pkgver/pkgrel cannot contain hyphens, so the last two fields are stripped.
fn dir_package_name(basename: &str) -> Option<String> {
    let mut parts = basename.rsplitn(3, '-');
    parts.next()?; // pkgrel
    parts.next()?; // pkgver
    parts.next().map(|name| name.to_string())
}

/// Names of every package installed in the local database, independent of any
/// search/target filter and of `desc` parse failures (which fall back to the
/// directory name). Used to tell "not installed" apart from "not matched".
pub fn db_package_names(dbpath: &Path) -> crate::error::Result<HashSet<String>> {
    let local_dir = dbpath.join("local");
    let dirs = match std::fs::read_dir(&local_dir) {
        Ok(d) => d,
        Err(e) => return Err(crate::error::PkgdsError::Io(e)),
    };

    let mut names = HashSet::new();
    for entry in dirs.flatten() {
        let path = entry.path();
        // ALPM_DB_VERSION is a file, so the is_dir() test skips it too.
        if !path.is_dir() || !path.join("desc").is_file() {
            continue;
        }
        let basename = match path.file_name().and_then(|s| s.to_str()) {
            Some(b) => b,
            None => continue,
        };
        let name = std::fs::read_to_string(path.join("desc"))
            .ok()
            .and_then(|c| parse_desc(&c).ok())
            .map(|d| d.name)
            .or_else(|| dir_package_name(basename));
        if let Some(name) = name {
            names.insert(name);
        }
    }

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // D2/D3 — parse_desc tests
    #[test]
    fn test_parse_desc() {
        let text = "%NAME%\nhello_pkg\n%DESCRIPTION%\nA test package.\n%VERSION%\n1.0-1\n%BUILDINFO%\n%SIZE%\n12345678\n";
        let df = parse_desc(text).unwrap();
        assert_eq!(df.name, "hello_pkg");
        assert_eq!(df.version, "1.0-1");
        assert_eq!(df.size, 12_345_678);
    }

    #[test]
    fn test_parse_desc_multiline_value() {
        let text = "%NAME%\nworld_pkg\n%VERSION%\n2.0-beta.1-1\n";
        let df = parse_desc(text).unwrap();
        assert_eq!(df.name, "world_pkg");
        assert_eq!(df.version, "2.0-beta.1-1");
    }

    #[test]
    fn test_parse_desc_missing_size() {
        let text = "%NAME%\ntest\n%VERSION%\n1.0-1\n";
        let df = parse_desc(text).unwrap();
        assert_eq!(df.size, 0);
    }

    #[test]
    fn test_parse_desc_depends_provides_reason() {
        let text = "%NAME%\nfoo\n%VERSION%\n1.0-1\n%REASON%\n1\n\
                    %DEPENDS%\nglibc>=2.34\nsh\n%PROVIDES%\nfoo-bin\n";
        let df = parse_desc(text).unwrap();
        assert_eq!(df.depends, vec!["glibc>=2.34", "sh"]);
        assert_eq!(df.provides, vec!["foo-bin"]);
        assert_eq!(df.reason, Some(1));
    }

    #[test]
    fn test_parse_desc_empty_name_returns_error() {
        let text = "%VERSION%\n1.0-1\n%SIZE%\n100\n";
        assert!(parse_desc(text).is_err());
    }

    // D4/D5 — parse_files tests
    #[test]
    fn test_parse_files_all() {
        let text =
            "%FILES%\nusr/lib/lib.so\nusr/bin/tool\nusr/share/doc/\n%BACKUP%\n/etc/default/\n";
        let paths = parse_files(text);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], Path::new("usr/lib/lib.so"));
    }

    #[test]
    fn test_parse_files_empty_lines() {
        let text = "%FILES%\n\nfile1\n\nfile2\n";
        let paths = parse_files(text);
        assert_eq!(paths.len(), 2);
    }

    // D6/D7 — pacman_conf tests
    #[test]
    fn test_parse_pacman_conf_valid_dbpath() {
        let tmp = tempfile::tempdir().unwrap();
        let conf_path = tmp.path().join("pacman.conf");
        std::fs::write(
            &conf_path,
            "# comment\n[options]\nDBPath = /tmp/my_db\nArchitecture = auto\n",
        )
        .unwrap();
        let conf = parse_pacman_conf(&conf_path).unwrap();
        assert_eq!(conf.dbpath, PathBuf::from("/tmp/my_db"));
    }

    #[test]
    fn test_parse_pacman_conf_missing_file_defaults() {
        let conf = parse_pacman_conf(Path::new("/nonexistent_pkgds_pacman.conf")).unwrap();
        assert_eq!(conf.dbpath, PathBuf::from("/var/lib/pacman"));
    }

    #[test]
    fn test_parse_pacman_conf_empty_dbpath_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let conf_path = tmp.path().join("pacman.conf");
        std::fs::write(&conf_path, "DBPath =\n").unwrap();
        let conf = parse_pacman_conf(&conf_path).unwrap();
        assert_eq!(conf.dbpath, PathBuf::from("/var/lib/pacman"));
    }

    #[test]
    fn test_load_local_db_basic() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let local = dbpath.join("local").join("pkg-a-1.0-1");

        std::fs::create_dir_all(&local).unwrap();
        // Write desc file
        std::fs::write(
            local.join("desc"),
            "%NAME%\npkg-a\n%VERSION%\n2.0-1\n%SIZE%\n500\n",
        )
        .unwrap();
        // Write files file
        std::fs::write(
            local.join("files"),
            "%FILES%\nusr/lib/lib.so\nusr/share/doc/\n",
        )
        .unwrap();

        let pkg_dir = dbpath.join("local").join("pkg-a-1.0-1");
        assert_eq!(
            parse_desc(&std::fs::read_to_string(pkg_dir.join("desc")).unwrap())
                .unwrap()
                .version,
            "2.0-1"
        );
    }

    #[test]
    fn test_load_local_db_version_from_desc() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let local = dbpath.join("local").join("pkg-a-1.0-1");

        std::fs::create_dir_all(&local).unwrap();

        // Dir name says "1.0-1", but desc file says "2.0-1"
        std::fs::write(
            local.join("desc"),
            "%NAME%\npkg-a\n%VERSION%\n2.0-1\n%SIZE%\n500\n",
        )
        .unwrap();

        let (entries, _skipped, _errors) = load_local_db(
            &dbpath,
            &Filter {
                targets: vec![],
                search: None,
            },
        )
        .unwrap();

        assert_eq!(entries[0].version, "2.0-1");
    }

    // D9 — load_local_db filter tests using real temp directories

    #[test]
    fn test_load_local_db_target_filter() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let local_parent = dbpath.join("local");
        std::fs::create_dir_all(&local_parent).unwrap();

        // Create pkg-alpha (matches "pkg-alpha" target)
        let p1 = local_parent.join("pkg-alpha-1.0-1");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::write(
            p1.join("desc"),
            "%NAME%\npkg-alpha\n%VERSION%\n1.0-1\n%SIZE%\n111\n",
        )
        .unwrap();
        std::fs::write(p1.join("files"), "%FILES%\nusr/lib/a.so\n").unwrap();

        // Create pkg-beta (does NOT match "pkg-alpha" target)
        let p2 = local_parent.join("pkg-beta-2.0-1");
        std::fs::create_dir_all(&p2).unwrap();
        std::fs::write(
            p2.join("desc"),
            "%NAME%\npkg-beta\n%VERSION%\n2.0-1\n%SIZE%\n222\n",
        )
        .unwrap();
        std::fs::write(p2.join("files"), "%FILES%\nusr/lib/b.so\n").unwrap();

        let (entries, skipped, _errors) = load_local_db(
            &dbpath,
            &Filter {
                targets: vec!["pkg-alpha".to_string()],
                search: None,
            },
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "pkg-alpha");
        assert_eq!(entries[0].version, "1.0-1");
        assert_eq!(skipped, 0); // filter non-matches are not counted as skipped
    }

    #[test]
    fn test_load_local_db_search_regex() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let local_parent = dbpath.join("local");
        std::fs::create_dir_all(&local_parent).unwrap();

        let p1 = local_parent.join("pkg-libs-1.0-1");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::write(
            p1.join("desc"),
            "%NAME%\npkg-libs\n%VERSION%\n1.0-1\n%SIZE%\n100\n",
        )
        .unwrap();
        std::fs::write(p1.join("files"), "%FILES%\nusr/lib.so\n").unwrap();

        let p2 = local_parent.join("pkg-utils-2.0-1");
        std::fs::create_dir_all(&p2).unwrap();
        std::fs::write(
            p2.join("desc"),
            "%NAME%\npkg-utils\n%VERSION%\n2.0-1\n%SIZE%\n200\n",
        )
        .unwrap();
        std::fs::write(p2.join("files"), "%FILES%\n/usr/bin/tool\n").unwrap();

        let re = regex::Regex::new(r"libs").unwrap();
        let (entries, skipped, _errors) = load_local_db(
            &dbpath,
            &Filter {
                targets: vec![],
                search: Some(re),
            },
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "pkg-libs");
        assert_eq!(skipped, 0); // filter non-matches are not counted as skipped
    }

    #[test]
    fn test_load_local_db_targets_and_search_intersection() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let local_parent = dbpath.join("local");
        std::fs::create_dir_all(&local_parent).unwrap();

        // pkg-foo — matches target "pkg-foo" AND regex "foo"
        let p1 = local_parent.join("pkg-foo-1.0-1");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::write(
            p1.join("desc"),
            "%NAME%\npkg-foo\n%VERSION%\n1.0-1\n%SIZE%\n100\n",
        )
        .unwrap();
        std::fs::write(p1.join("files"), "%FILES%\n/usr/lib.so\n").unwrap();

        // pkg-bar — does NOT match target "pkg-foo" (but would match regex)
        let p2 = local_parent.join("pkg-bar-2.0-1");
        std::fs::create_dir_all(&p2).unwrap();
        std::fs::write(
            p2.join("desc"),
            "%NAME%\npkg-bar\n%VERSION%\n2.0-1\n%SIZE%\n200\n",
        )
        .unwrap();
        std::fs::write(p2.join("files"), "%FILES%\n/usr/bin/tool\n").unwrap();

        let re = regex::Regex::new(r"foo").unwrap();
        let (entries, skipped, _errors) = load_local_db(
            &dbpath,
            &Filter {
                targets: vec!["pkg-foo".to_string()],
                search: Some(re),
            },
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "pkg-foo");
        assert_eq!(skipped, 0); // filter non-matches are not counted as skipped
    }

    #[test]
    fn test_dir_package_name() {
        assert_eq!(dir_package_name("badpkg-1.0-1").as_deref(), Some("badpkg"));
        assert_eq!(
            dir_package_name("foo-bar-1.0.r1-2").as_deref(),
            Some("foo-bar")
        );
        assert_eq!(dir_package_name("nohyphen"), None);
    }

    #[test]
    fn test_db_package_names_includes_all_and_falls_back_to_dir() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let local = dbpath.join("local");
        std::fs::create_dir_all(&local).unwrap();

        // Normal package: name from desc.
        let p1 = local.join("pkg-a-1.0-1");
        std::fs::create_dir_all(&p1).unwrap();
        std::fs::write(p1.join("desc"), "%NAME%\npkg-a\n%VERSION%\n1.0-1\n").unwrap();

        // Malformed desc: name falls back to the directory.
        let p2 = local.join("pkg-b-2.0-1");
        std::fs::create_dir_all(&p2).unwrap();
        std::fs::write(p2.join("desc"), "%NAME%\n%VERSION%\n2.0-1\n").unwrap();

        let names = db_package_names(&dbpath).unwrap();
        assert!(names.contains("pkg-a"), "got: {names:?}");
        assert!(names.contains("pkg-b"), "got: {names:?}");
    }

    #[test]
    fn test_load_local_db_counts_desc_less_directory() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        std::fs::create_dir_all(dbpath.join("local").join("no-desc-1.0-1")).unwrap();

        let (entries, skipped, errors) = load_local_db(
            &dbpath,
            &Filter {
                targets: vec![],
                search: None,
            },
        )
        .unwrap();

        assert!(entries.is_empty());
        assert_eq!(skipped, 1);
        assert!(
            errors.iter().any(|e| e.contains("No desc file")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn test_load_local_db_counts_unreadable_files_manifest() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath = tmpdir.path().join("db");
        let pkg = dbpath.join("local").join("pkg-1.0-1");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("desc"), "%NAME%\npkg\n%VERSION%\n1.0-1\n").unwrap();
        // A directory where the `files` manifest should be: reading it as a
        // file fails with something other than NotFound.
        std::fs::create_dir_all(pkg.join("files")).unwrap();

        let (entries, skipped, errors) = load_local_db(
            &dbpath,
            &Filter {
                targets: vec![],
                search: None,
            },
        )
        .unwrap();

        assert!(entries.is_empty());
        assert_eq!(skipped, 1);
        assert!(
            errors.iter().any(|e| e.contains("Failed to read files")),
            "got: {errors:?}"
        );
    }
}
