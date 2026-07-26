use anyhow::Result;
use colored::*;
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub mod cli;
pub mod tidy;
pub mod util;

pub fn handle_operation<F>(op: F)
where
    F: FnOnce() -> Result<()>,
{
    if let Err(e) = op() {
        eprintln!("{}: {}", "Error".red(), e);
    }
}

pub fn edit_symlink_target(origin_path: &Path, new_target_str: impl AsRef<Path>) -> Result<()> {
    fs::remove_file(origin_path)?;
    symlink(new_target_str.as_ref(), origin_path)?;
    Ok(())
}


pub fn create_hard_link(target: &Path, origin: &Path) -> Result<()> {
    if target.is_dir() {
        anyhow::bail!("Refusing to hardlink a directory");
    }
    fs::hard_link(target, origin)?;
    Ok(())
}

pub fn dereference_symlink(path: &Path) -> Result<PathBuf> {
    if !path.is_symlink() {
        return Ok(path.to_path_buf());
    }
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut current = path.to_path_buf();
            let mut visited: HashSet<(u64, u64)> = HashSet::new();
            loop {
                let meta = match fs::symlink_metadata(&current) {
                    Err(_) => break,
                    Ok(m) if !m.is_symlink() => break,
                    Ok(m) => m,
                };
                if !visited.insert((meta.dev(), meta.ino())) {
                    // symlink cycle
                    break;
                }
                match fs::read_link(&current) {
                    Ok(next) => {
                        current = if next.is_absolute() {
                            next
                        } else if let Some(parent) = current.parent() {
                            parent.join(next)
                        } else {
                            next
                        };
                    },
                    Err(_) => break,
                }
            }
            Ok(current)
        },
        Err(e) => Err(anyhow::Error::new(e).context(format!(
                    "Could not resolve target: {}",
                    path.display(),
        ))),
    }
}

pub fn create_hard_link_tree(target: &Path, origin: &Path) -> Result<()> {
    if target.is_dir() {
        fs::create_dir_all(origin)?;
        for entry in WalkDir::new(target) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(target)?;
            if rel.as_os_str().is_empty() {
                continue;
            }
            let dest = origin.join(rel);
            if entry.path().is_dir() {
                fs::create_dir_all(dest)?;
            } else {
                fs::hard_link(entry.path(), dest)?;
            }
        }
    } else {
        fs::hard_link(target, origin)?;
    }
    Ok(())
}

pub fn create_symlink_tree(target: &Path, origin: &Path) -> Result<()> {
    if target.is_dir() {
        fs::create_dir_all(origin)?;
        for entry in WalkDir::new(target) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(target)?;
            if rel.as_os_str().is_empty() {
                continue;
            }
            let dest = origin.join(rel);
            if entry.path().is_dir() {
                fs::create_dir_all(dest)?;
            } else {
                let abs_target = fs::canonicalize(entry.path())?;
                symlink(abs_target, dest)?;
            }
        }
    } else {
        let abs_target = fs::canonicalize(target)?;
        symlink(abs_target, origin)?;
    }
    Ok(())
}

pub fn log_dangling_link(cmd: &str, link: impl AsRef<Path>, target: impl AsRef<Path>) {
    log_link_err(
        Some(cmd.bold()),
        Some("skipping dangling symlink".red()),
        link,
        target,
    );
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
