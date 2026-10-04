use std::collections::HashSet;
use std::fs;
use std::os::unix;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

pub fn is_cross_device(a: &Path, b: &Path) -> Result<bool> {
    Ok(device_of(a)? != device_of(b)?)
}

fn device_of(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path)
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                fs::symlink_metadata(parent)
            } else {
                Err(e)
            }
        })
        .with_context(|| format!("could not determine device for {}", path.display()))?;
    Ok(meta.dev())
}

pub fn create_hard_link(target: &Path, origin: &Path) -> Result<()> {
    if target.is_dir() {
        anyhow::bail!("Refusing to hardlink a directory");
    }
    fs::hard_link(target, origin)?;
    Ok(())
}

// follows a chain of symlinks to its end, even if it ultimately dangles
pub fn dereference_symlink(path: &Path) -> Result<PathBuf> {
    if !path.is_symlink() {
        return Ok(path.to_path_buf());
    }
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut chain = symlink_chain(path);
            let last = chain.pop()
                .expect("symlink_chain should always return at least one thing");
            Ok(last)
        }
        Err(e) => {
            Err(anyhow::Error::new(e)
                .context(format!("Could not resolve target: {}", path.display(),)))
        }
    }
}

// resolves chains of symlinks pointing to one another by producing
// a Vec of the chain of paths, starting from the origin and ending at the final target
pub fn symlink_chain(path: &Path) -> Vec<PathBuf> {
    let mut current = path.to_path_buf();
    let mut chain: Vec<PathBuf> = vec![current.clone()];
    let mut visited: HashSet<(u64, u64)> = HashSet::new();
    loop {
        let meta = match fs::symlink_metadata(&current) {
            Err(_) => break,
            Ok(m) if !m.is_symlink() => break,
            Ok(m) => m,
        };
        if !visited.insert((meta.dev(), meta.ino())) {
            // symlink cycle
            // TODO: should we handle this with an Err?
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
                chain.push(current.clone());
            }
            Err(_) => break,
        }
    }
    chain
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn symlink_chain_regular_file_is_just_itself() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "").unwrap();
        assert_eq!(symlink_chain(&file), vec![file]);
    }

    #[test]
    fn symlink_chain_missing_path_is_just_itself() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(symlink_chain(&missing), vec![missing]);
    }

    #[test]
    fn symlink_chain_relative_target_joins_parent() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("real"), "").unwrap();
        let link = dir.path().join("link");
        symlink("real", &link).unwrap();
        assert_eq!(symlink_chain(&link), vec![link, dir.path().join("real")]);
    }

    #[test]
    fn symlink_chain_absolute_target_is_used_as_is() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real");
        fs::write(&real, "").unwrap();
        let link = dir.path().join("link");
        symlink(&real, &link).unwrap();
        assert_eq!(symlink_chain(&link), vec![link, real]);
    }

    #[test]
    fn symlink_chain_follows_every_hop() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("real"), "").unwrap();
        symlink("real", dir.path().join("b")).unwrap();
        symlink("b", dir.path().join("a")).unwrap();
        let link = dir.path().join("link");
        symlink("a", &link).unwrap();
        assert_eq!(
            symlink_chain(&link),
            vec![
                link,
                dir.path().join("a"),
                dir.path().join("b"),
                dir.path().join("real"),
            ]
        );
    }

    #[test]
    fn symlink_chain_dangling_ends_at_missing_path() {
        let dir = tempdir().unwrap();
        symlink("missing", dir.path().join("mid")).unwrap();
        let link = dir.path().join("link");
        symlink("mid", &link).unwrap();
        assert_eq!(
            symlink_chain(&link),
            vec![link, dir.path().join("mid"), dir.path().join("missing")]
        );
    }

    #[test]
    fn symlink_chain_cycle_terminates() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        symlink("b", &a).unwrap();
        symlink("a", &b).unwrap();
        // the path that closes the cycle is pushed before the cycle is detected
        assert_eq!(symlink_chain(&a), vec![a.clone(), b, a]);
    }

    #[test]
    fn symlink_chain_excludes_symlinked_parent_dirs() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("realdir")).unwrap();
        fs::write(dir.path().join("realdir/file"), "").unwrap();
        symlink("realdir", dir.path().join("linkdir")).unwrap();
        let link = dir.path().join("link");
        symlink("linkdir/file", &link).unwrap();
        // linkdir is a parent component, not a hop, so it must not appear.
        // replace-with-target unlinks the chain, and linkdir is not debris
        assert_eq!(
            symlink_chain(&link),
            vec![link, dir.path().join("linkdir/file")]
        );
    }
}
