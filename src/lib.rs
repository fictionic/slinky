use anyhow::Result;
use colored::*;
use std::fs;
use std::os::unix::fs::symlink;
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
        anyhow::bail!("cannot hard link a directory");
    }
    fs::hard_link(target, origin)?;
    Ok(())
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
