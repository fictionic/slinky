use std::path::Path;
use colored::*;

use anyhow::Result;
use colored::ColoredString;

pub fn run_with_error_logger<F>(op: F)
where
    F: FnOnce() -> Result<()>,
{
    if let Err(e) = op() {
        eprintln!("{}: {}", "Error".red(), e);
    }
}

pub fn log_link_err(
    cmd: Option<ColoredString>,
    err_msg: Option<ColoredString>,
    link: impl AsRef<Path>,
    target: impl AsRef<Path>,
) {
    if let Some(c) = cmd {
        eprint!("{}: ", c);
    }
    if let Some(p) = err_msg {
        eprint!("{}: ", p);
    }
    eprintln!(
        "{} -> {}",
        link.as_ref().display().to_string().cyan(),
        target.as_ref().display().to_string().yellow()
    );
}

pub fn log_dangling_link(cmd: &str, link: impl AsRef<Path>, target: impl AsRef<Path>) {
    log_link_err(
        Some(cmd.bold()),
        Some("skipping dangling symlink".red()),
        link,
        target,
    );
}

pub fn log_link(prefix: Option<ColoredString>, link: impl AsRef<Path>, target: impl AsRef<Path>) {
    if let Some(p) = prefix {
        print!("{}: ", p);
    }
    println!(
        "{} -> {}",
        link.as_ref().display().to_string().cyan(),
        target.as_ref().display().to_string().yellow()
    );
}

pub fn log_transformation(
    cmd_name: &str,
    link: impl AsRef<Path>,
    old: impl AsRef<Path>,
    new: impl AsRef<Path>,
) {
    println!(
        "{}: {} -> ({} {} {})",
        cmd_name.bold(),
        link.as_ref().display().to_string().cyan(),
        old.as_ref().display().to_string().dimmed(),
        "=>".bright_white(),
        new.as_ref().display().to_string().yellow()
    );
}
