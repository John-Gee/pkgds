//! Regenerate the man page and shell completions from the CLI definition.
//!
//! Run with `cargo run --example gen-assets`; the generated files are committed
//! under `man/` and `completions/` and installed by the PKGBUILD.

use std::fs;

use clap::CommandFactory;

fn main() -> std::io::Result<()> {
    let cmd = pkgds::config::RawArgs::command();

    fs::create_dir_all("man")?;
    let mut man = Vec::new();
    clap_mangen::Man::new(cmd.clone()).render(&mut man)?;
    fs::write("man/pkgds.1", man)?;

    fs::create_dir_all("completions")?;
    for (shell, path) in [
        (clap_complete::Shell::Bash, "completions/pkgds.bash"),
        (clap_complete::Shell::Zsh, "completions/_pkgds"),
        (clap_complete::Shell::Fish, "completions/pkgds.fish"),
    ] {
        let mut buf = Vec::new();
        clap_complete::generate(shell, &mut cmd.clone(), "pkgds", &mut buf);
        fs::write(path, buf)?;
    }

    Ok(())
}
