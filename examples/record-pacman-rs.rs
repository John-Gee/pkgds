//! Record a `--with-deps` ground-truth fixture from a real Arch system.
//!
//!     cargo run --example record-pacman-rs > tests/fixtures/pacman_rs.tsv
//!
//! Needs pacman and a populated local database, so it is run by hand (not in
//! CI). The output is replayed hermetically by
//! `test_with_deps_matches_recorded_pacman_rs`.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use pkgds::pacman::{Filter, load_local_db};

fn main() {
    let dbpath = Path::new("/var/lib/pacman");
    let (entries, _skipped, _errors) = load_local_db(
        dbpath,
        &Filter {
            targets: vec![],
            search: None,
        },
    )
    .unwrap();

    let mut out = String::new();
    writeln!(
        out,
        "# pkgds --with-deps ground truth, recorded from `pacman -Rs --print`."
    )
    .unwrap();
    for e in &entries {
        writeln!(
            out,
            "entry\t{}\t{}\t{}\t{}",
            e.name,
            if e.explicit { 1 } else { 0 },
            e.depends.join(" "),
            e.provides.join(" "),
        )
        .unwrap();
    }

    let mut multi = 0usize;
    let mut single = 0usize;
    for (scanned, e) in entries.iter().enumerate() {
        if (multi >= 20 && single >= 10) || scanned >= 1200 {
            break;
        }
        let o = Command::new("pacman")
            .args(["-Rs", "--print", &e.name])
            .output()
            .expect("run pacman");
        if !o.status.success() {
            continue; // pacman refuses removals that break other packages
        }
        let expected: Vec<String> = String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|line| {
                let mut parts = line.trim().rsplitn(3, '-');
                parts.next()?; // release
                parts.next()?; // version
                parts.next().map(str::to_string)
            })
            .collect();
        if expected.is_empty() {
            continue;
        }
        // Prefer cases with exclusive dependencies (the interesting ones), but
        // keep a few single-package cases too.
        if expected.len() > 1 {
            if multi >= 20 {
                continue;
            }
            multi += 1;
        } else {
            if single >= 10 {
                continue;
            }
            single += 1;
        }
        writeln!(out, "target\t{}", e.name).unwrap();
        writeln!(out, "expect\t{}", expected.join(" ")).unwrap();
    }

    print!("{out}");
}
