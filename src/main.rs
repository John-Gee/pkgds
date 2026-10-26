use std::io::Write;

use pkgds::config::RawArgs;
use pkgds::output;
use pkgds::scan;

fn eprint_error(msg: &str, color: bool) {
    if color {
        use owo_colors::OwoColorize;
        eprintln!("{}", msg.red());
    } else {
        eprintln!("{}", msg);
    }
}

fn main() {
    let raw_args = {
        use clap::Parser;
        RawArgs::parse()
    };

    let cfg = match raw_args.into_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let use_color = output::stderr_color_enabled(&cfg);

    let report = match scan::scan_packages(&cfg) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // Print diagnostics to stderr
    for err in &report.errors {
        eprint_error(err, use_color);
    }
    for target in &report.missing_targets {
        eprint_error(
            &format!(
                "no such package: {} (use -s/--search for substring/regex search)",
                target
            ),
            use_color,
        );
    }
    let unreadable = report.permission_errors + report.read_errors;
    if cfg.verbose {
        for warn in &report.warnings {
            eprintln!("{}", warn);
        }
    } else if unreadable > 0 {
        // The exit code is 2, but without -v nothing has said why.
        eprintln!(
            "{} file(s) could not be read; re-run with -v for details",
            unreadable
        );
    }

    // Render output to stdout. Nothing matched means nothing to print, so avoid
    // emitting a bare header.
    if !report.packages.is_empty() {
        let result = if let Some(ref fmt) = cfg.format {
            report
                .packages
                .iter()
                .map(|pkg| fmt.render(pkg, &cfg))
                .collect::<Vec<_>>()
                .join(&cfg.delim)
        } else if cfg.depth.is_some() {
            // The tree keeps every row in one unit, so packages the view
            // cannot measure are left out. Say so, rather than let them look
            // like they vanished.
            if report.trees_omitted > 0 {
                eprint_error(
                    &format!(
                        "{} package(s) have no btrfs on-disk data and are not shown in the tree",
                        report.trees_omitted
                    ),
                    use_color,
                );
            }
            output::render_tree(&report, &cfg)
        } else {
            output::render_table(&report, &cfg)
        };
        // A broken pipe (`pkgds | head`) is normal; anything else is a real
        // write error. Using `writeln!` rather than `println!` avoids the
        // panic that would otherwise turn a closed pipe into exit 101.
        if let Err(e) = writeln!(std::io::stdout(), "{}", result)
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            eprintln!("error writing to stdout: {}", e);
            std::process::exit(1);
        }
    }

    // Exit codes: 1 when nothing matched at all (unknown targets, an empty
    // search, an empty database); 2 when the result is partial: a file or
    // package could not be read, btrfs data was unavailable, or some requested
    // targets were unknown.
    if report.packages.is_empty() {
        std::process::exit(1);
    }
    if report.skipped_packages + report.permission_errors + report.read_errors > 0
        || !report.errors.is_empty()
        || !report.missing_targets.is_empty()
    {
        std::process::exit(2);
    }
}
