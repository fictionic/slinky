use anyhow::Result;
use colored::*;
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub mod cli;

pub fn tidy_path(path: &Path) -> PathBuf {
    let mut cleaned = PathBuf::new();
    let mut components = path.components().peekable();

    // Handle absolute paths / prefixes
    if let Some(c @ std::path::Component::Prefix(..)) = components.peek() {
        cleaned.push(c);
        components.next();
    }
    if let Some(c @ std::path::Component::RootDir) = components.peek() {
        cleaned.push(c);
        components.next();
    }

    for component in components {
        match component {
            std::path::Component::Normal(c) => cleaned.push(c),
            std::path::Component::CurDir => {} // Ignore .
            std::path::Component::ParentDir => {
                if let Some(std::path::Component::Normal(..)) = cleaned.components().next_back() {
                    cleaned.pop();
                } else if cleaned.as_os_str().is_empty()
                    || cleaned.components().next_back() == Some(std::path::Component::ParentDir)
                {
                    // Keep leading .. in relative paths or append to existing ..
                    cleaned.push(component);
                }
                // If at RootDir, .. is a no-op
            }
            _ => {} // Ignore other component types like Prefix, RootDir
        }
    }
    cleaned
}

pub fn create_hard_link(target: &Path, origin: &Path) -> Result<()> {
    if target.is_dir() {
        anyhow::bail!("Refusing to hardlink a directory");
    }
    fs::hard_link(target, origin)?;
    Ok(())
}

pub fn dereference_symlink(path: &Path) -> PathBuf {
    if !path.is_symlink() {
        return path.to_path_buf();
    }
    match fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(_) => {
            // TODO: there are other failure modes for canonicalize() besides
            // 'symlink loop'. probably this function should return a Result
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
            current
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tidy_path_basics() {
        assert_eq!(tidy_path(Path::new("foo/bar")), PathBuf::from("foo/bar"));
        assert_eq!(tidy_path(Path::new("foo/./bar")), PathBuf::from("foo/bar"));
        assert_eq!(tidy_path(Path::new("./foo/bar")), PathBuf::from("foo/bar"));
        assert_eq!(tidy_path(Path::new("foo/bar/.")), PathBuf::from("foo/bar"));
    }

    #[test]
    fn test_tidy_path_parent_traversal() {
        assert_eq!(tidy_path(Path::new("foo/../bar")), PathBuf::from("bar"));
        assert_eq!(tidy_path(Path::new("foo/bar/..")), PathBuf::from("foo"));
        assert_eq!(tidy_path(Path::new("foo/bar/../baz")), PathBuf::from("foo/baz"));
        assert_eq!(tidy_path(Path::new("a/b/../../c")), PathBuf::from("c"));
    }

    #[test]
    fn test_tidy_path_leading_parent() {
        assert_eq!(tidy_path(Path::new("../foo")), PathBuf::from("../foo"));
        assert_eq!(tidy_path(Path::new("../../foo")), PathBuf::from("../../foo"));
        assert_eq!(tidy_path(Path::new("../foo/../bar")), PathBuf::from("../bar"));
    }

    #[test]
    fn test_tidy_path_mixed() {
        assert_eq!(tidy_path(Path::new("a/../../b")), PathBuf::from("../b"));
        assert_eq!(tidy_path(Path::new("a/./../b")), PathBuf::from("b"));
    }

    #[test]
    fn test_tidy_path_absolute() {
        assert_eq!(tidy_path(Path::new("/foo/bar")), PathBuf::from("/foo/bar"));
        assert_eq!(tidy_path(Path::new("/foo/../bar")), PathBuf::from("/bar"));
        assert_eq!(tidy_path(Path::new("/../foo")), PathBuf::from("/foo"));
        assert_eq!(tidy_path(Path::new("/../../foo")), PathBuf::from("/foo"));
    }

    #[test]
    fn test_dereference_symlink_cycle_terminates() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        symlink("b", &a).unwrap();
        symlink("a", &b).unwrap();

        let (tx, rx) = mpsc::channel();
        let a_for_thread = a.clone();
        thread::spawn(move || {
            let _ = tx.send(dereference_symlink(&a_for_thread));
        });

        if rx.recv_timeout(Duration::from_secs(5)).is_err() {
            panic!("dereference_symlink did not terminate on a symlink cycle");
        }
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

pub fn handle_operation<F>(op: F)
where
    F: FnOnce() -> Result<()>,
{
    if let Err(e) = op() {
        eprintln!("{}: {}", "Error".red(), e);
    }
}

pub fn log_dangling_link(cmd: &str, link: &str, target: &str) {
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
    link: &str,
    target: &str,
) {
    if let Some(c) = cmd {
        eprint!("{}: ", c);
    }
    if let Some(p) = err_msg {
        eprint!("{}: ", p);
    }
    eprintln!("{} -> {}", link.cyan(), target.yellow());
}

pub fn log_link(prefix: Option<ColoredString>, link: &str, target: &str) {
    if let Some(p) = prefix {
        print!("{}: ", p);
    }
    println!("{} -> {}", link.cyan(), target.yellow());
}

pub fn log_transformation(cmd_name: &str, link: &str, old: &str, new: &str) {
    println!(
        "{}: {} -> ({} {} {})",
        cmd_name.bold(),
        link.cyan(),
        old.dimmed(),
        "=>".bright_white(),
        new.yellow()
    );
}
