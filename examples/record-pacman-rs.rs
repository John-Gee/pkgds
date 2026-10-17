//! Record a `--with-deps` ground-truth fixture from a real Arch system.
//!
//!     cargo run --example record-pacman-rs
//!
//! Writes `tests/fixtures/pacman_rs.tsv`. Needs pacman and a populated local
//! database, so it is run by hand (not in CI). It writes the file only after
//! every assertion passes, so a failed run never clobbers a good fixture.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use pkgds::pacman::{Filter, load_local_db};

fn main() {
    let dbpath = Path::new("/var/lib/pacman");
    let (mut entries, skipped, errors) = load_local_db(
        dbpath,
        &Filter {
            targets: vec![],
            search: None,
        },
    )
    .unwrap();

    // The expectations come from pacman's view of the database; if we read a
    // different set, the fixture would describe a world pkgds never sees.
    assert!(
        skipped == 0 && errors.is_empty(),
        "the local database was not read cleanly (skipped {skipped}, {} error(s))",
        errors.len()
    );

    // Deterministic output: read_dir order is unspecified, and it drives entry
    // order, which packages get scanned, and so which cases are picked.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = String::new();
    writeln!(
        out,
        "# pkgds --with-deps ground truth recorded from `pacman -Rs --print`."
    )
    .unwrap();
    writeln!(out, "# Tab-separated lines:").unwrap();
    writeln!(
        out,
        "#   entry<TAB>name<TAB>explicit(1|0)<TAB>depends(space-separated)<TAB>provides(space-separated)"
    )
    .unwrap();
    writeln!(out, "#   target<TAB>package name").unwrap();
    writeln!(
        out,
        "#   expect<TAB>packages `pacman -Rs` would remove (space-separated)"
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

    assert!(
        multi + single > 0,
        "no comparable packages found; refusing to write an empty fixture"
    );

    std::fs::write("tests/fixtures/pacman_rs.tsv", out).expect("write fixture");
    eprintln!(
        "wrote tests/fixtures/pacman_rs.tsv ({multi} multi-package, {single} single-package cases)"
    );
}
