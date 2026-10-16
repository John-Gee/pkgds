//! Regenerate the man page and shell completions from the CLI definition.
//!
//! Run with `cargo run --example gen-assets`; the generated files are committed
//! under `man/` and `completions/` and installed by the PKGBUILD.

use std::fs;

use clap::CommandFactory;

fn main() -> std::io::Result<()> {
    let cmd = pkgds::config::RawArgs::command();
    let name = cmd.get_name().to_string();

    fs::create_dir_all("man")?;
    let mut man = Vec::new();
    clap_mangen::Man::new(cmd.clone()).render(&mut man)?;
    fs::write(format!("man/{name}.1"), man)?;

    fs::create_dir_all("completions")?;
    for (shell, path) in [
        (
            clap_complete::Shell::Bash,
            format!("completions/{name}.bash"),
        ),
        (clap_complete::Shell::Zsh, format!("completions/_{name}")),
        (
            clap_complete::Shell::Fish,
            format!("completions/{name}.fish"),
        ),
    ] {
        let mut buf = Vec::new();
        clap_complete::generate(shell, &mut cmd.clone(), &name, &mut buf);
        fs::write(path, buf)?;
    }

    Ok(())
}
