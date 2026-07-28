use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use std::os::unix;
use std::fs;

use anyhow::Result;
use walkdir::WalkDir;

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
                unix::fs::symlink(abs_target, dest)?;
            }
        }
    } else {
        let abs_target = fs::canonicalize(target)?;
        unix::fs::symlink(abs_target, origin)?;
    }
    Ok(())
}

