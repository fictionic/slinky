use colored::*;
use std::path::Path;

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
    cmd_name: Option<&str>,
    err_msg: Option<&str>,
    origin_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) {
    if let Some(c) = cmd_name {
        eprint!("{}: ", c.bold());
    }
    if let Some(p) = err_msg {
        eprint!("{}: ", p.red());
    }
    eprintln!(
        "{} -> {}",
        origin_path.as_ref().display().to_string().cyan(),
        target_path.as_ref().display().to_string().yellow()
    );
}

pub fn log_dangling_link(
    cmd_name: &str,
    origin_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) {
    log_link_err(
        Some(cmd_name),
        Some("skipping dangling symlink"),
        origin_path,
        target_path,
    );
}

pub fn log_link_from_cmd(
    cmd_name: &str,
    origin_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) {
    log_link_with_prefix(
        Some(cmd_name.bold()),
        origin_path,
        target_path,
    );
}

pub fn log_link_with_prefix(
    prefix: Option<ColoredString>,
    origin_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) {
    if let Some(p) = prefix {
        print!("{}: ", p);
    }
    println!(
        "{} -> {}",
        origin_path.as_ref().display().to_string().cyan(),
        target_path.as_ref().display().to_string().yellow()
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
