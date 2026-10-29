use std::collections::HashSet;
use std::fs;
use std::os::unix;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::path::dirpath;
use crate::semantic_path::SemanticPath;

/// Like `fs::canonicalize`, but does not follow a final symlink. In other words, it returns the
/// canonical path to the directory entry that the path indicates.
pub fn canonicalize_nofollow(path: &Path) -> Result<PathBuf> {
    match SemanticPath::try_from(path)?.split_canonical_entry_name() {
        Some((parent_dir_path, entry_name)) => {
            let parent = parent_dir_path.to_path_buf().canonicalize()?;
            Ok(parent.join(entry_name))
        }
        // if there's no canonical entry name, it's safe to canonicalize the whole path, because we
        // know it cannot refer to a symlink.
        _ => Ok(path.canonicalize()?),
    }
}

pub fn is_cross_device(a: &Path, b: &Path) -> Result<bool> {
    Ok(device_of(a)? != device_of(b)?)
}

fn device_of(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path)
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                let parent = dirpath(path);
                fs::symlink_metadata(parent)
            } else {
                Err(e)
            }
        })
        .with_context(|| format!("could not determine device for {}", path.display()))?;
    Ok(meta.dev())
}

pub fn create_symlink(original: &Path, link: &Path) -> Result<()> {
    Ok(unix::fs::symlink(original, link)?)
}

pub fn create_hard_link(original: &Path, link: &Path) -> Result<()> {
    if original.is_dir() {
        anyhow::bail!("Refusing to hardlink a directory");
    }
    fs::hard_link(original, link)?;
    Ok(())
}

/// Follows a chain of symlinks to its end, even if it ultimately dangles.
pub fn dereference_symlink(path: &Path) -> Result<PathBuf> {
    if !path.is_symlink() {
        return Ok(path.to_path_buf());
    }
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut chain = symlink_chain(path);
            let last = chain
                .pop()
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

// Creates a 'hard link tree' to mirror the contents of a directory.
//
// A hard link tree is a directory tree that only contains directories and hard links. A hard link
// tree that mirrors a given directory will have an identical directory structure and, for each
// file, a hard link at a corresponding position to the same inode.
//
// The `tree` path will be a hard link tree to the directory specified by the `original` path.
// A symlink within `original` is not followed; the tree gets a hard link to the symlink itself, so
// a relative symlink will likely dangle at its new position.
//
// (This signature is written to resemble std::fs::hard_link.)
pub fn create_hard_link_tree(original: &Path, tree: &Path) -> Result<()> {
    if original.is_dir() {
        fs::create_dir(tree)?;
        for entry in WalkDir::new(original) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(original)?;
            if rel.as_os_str().is_empty() {
                continue;
            }
            let dest = tree.join(rel);
            if entry.file_type().is_dir() {
                fs::create_dir_all(dest)?;
            } else {
                fs::hard_link(entry.path(), dest)?;
            }
        }
    } else {
        fs::hard_link(original, tree)?;
    }
    Ok(())
}

// Creates a 'symlink tree' to mirror the contents of a directory.
//
// A symlink tree is a directory tree that only contains directories and symlinks. A symlink tree
// that mirrors a given directory will have an identical directory structure and, for each file, a
// symlink at a corresponding position that points to it.
//
// The `tree` path will be a symlink tree to the directory specified by the `original` path.
// If `original` is a relative path, then it is resolved relative to the parent of `tree`, not to
// the process CWD. All symlinks created within the tree will point through `original`.
//
// If `original` does not resolve to a directory, then a single symlink is created to point to it,
// which will dangle if the path does not resolve at all.
//
// (This signature is written to resemble unix::fs::symlink.)
pub fn create_symlink_tree(original: &Path, tree: &Path) -> Result<()> {
    // resolve a relative `original` path relative to the parent of `tree`
    let original_resolvable = match tree.parent() {
        Some(parent) if original.is_relative() => parent.join(original),
        _ => original.to_path_buf(),
    };
    if original_resolvable.is_dir() {
        fs::create_dir(tree)?;
        for entry in WalkDir::new(&original_resolvable) {
            // TODO: walkdir fails when it cannot read a directory or stat an entry (EACCES, EIO, or
            // an entry removed during the walk). it does not follow links, so it finds no loops. a
            // failure here leaves a partial tree; see BUGS.md, "Commands destroy the symlink before
            // an operation that can fail"
            let entry = entry?;
            let rel = entry.path().strip_prefix(&original_resolvable)?;
            if rel.as_os_str().is_empty() {
                continue;
            }
            let mirrored_entry_path = tree.join(rel);
            if entry.file_type().is_dir() {
                fs::create_dir_all(mirrored_entry_path)?;
            } else {
                let climb_out_prefix = if original.is_relative() {
                    let climb_out_depth = rel.components().count();
                    std::iter::repeat_n("..", climb_out_depth).collect()
                } else {
                    PathBuf::new()
                };
                let symlink_target = climb_out_prefix.join(original).join(rel);
                unix::fs::symlink(symlink_target, mirrored_entry_path)?;
            }
        }
    } else {
        unix::fs::symlink(original, tree)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    // canonicalize_nofollow

    // base/d/f, and base/dlink -> d
    fn nofollow_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(base.join("d")).unwrap();
        fs::write(base.join("d/f"), "").unwrap();
        symlink("d", base.join("dlink")).unwrap();
        (dir, base)
    }

    #[test]
    fn canonicalize_nofollow_keeps_final_symlink() {
        let (_dir, base) = nofollow_fixture();
        assert_eq!(
            canonicalize_nofollow(&base.join("dlink")).unwrap(),
            base.join("dlink")
        );
    }

    #[test]
    fn canonicalize_nofollow_resolves_parent() {
        let (_dir, base) = nofollow_fixture();
        assert_eq!(
            canonicalize_nofollow(&base.join("dlink/f")).unwrap(),
            base.join("d/f")
        );
    }

    #[test]
    fn canonicalize_nofollow_trailing_slash_follows_final_symlink() {
        let (_dir, base) = nofollow_fixture();
        assert_eq!(
            canonicalize_nofollow(&base.join("dlink/")).unwrap(),
            base.join("d")
        );
    }

    #[test]
    fn canonicalize_nofollow_trailing_dot_follows_final_symlink() {
        let (_dir, base) = nofollow_fixture();
        assert_eq!(
            canonicalize_nofollow(&base.join("dlink/.")).unwrap(),
            base.join("d")
        );
    }

    #[test]
    fn canonicalize_nofollow_trailing_dotdot_resolves_parent_of_target() {
        let (_dir, base) = nofollow_fixture();
        // dlink/.. is the parent of d, not of dlink
        assert_eq!(canonicalize_nofollow(&base.join("dlink/..")).unwrap(), base);
    }

    #[test]
    fn canonicalize_nofollow_missing_parent_fails() {
        let (_dir, base) = nofollow_fixture();
        assert!(canonicalize_nofollow(&base.join("nope/f")).is_err());
    }

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

    // store/dir/file1, store/dir/sub/file2, and an empty links/. returns the
    // temp dir, the source directory, and the tree path links/link
    fn symlink_tree_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempdir().unwrap();
        let src = dir.path().join("store/dir");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("file1"), "one").unwrap();
        fs::write(src.join("sub/file2"), "two").unwrap();
        fs::create_dir(dir.path().join("links")).unwrap();
        let dest = dir.path().join("links/link");
        (dir, src, dest)
    }

    #[test]
    fn symlink_tree_absolute_target_children_resolve() {
        let (_dir, src, dest) = symlink_tree_fixture();
        create_symlink_tree(&src, &dest).unwrap();
        assert_eq!(
            fs::read_link(dest.join("file1")).unwrap(),
            src.join("file1")
        );
        assert_eq!(fs::read_to_string(dest.join("file1")).unwrap(), "one");
        assert_eq!(fs::read_to_string(dest.join("sub/file2")).unwrap(), "two");
    }

    #[test]
    fn symlink_tree_relative_target_top_level_child_resolves() {
        // links/link -> ../store/dir would resolve from links/
        let (_dir, _src, dest) = symlink_tree_fixture();
        create_symlink_tree(Path::new("../store/dir"), &dest).unwrap();
        assert_eq!(fs::read_to_string(dest.join("file1")).unwrap(), "one");
    }

    #[test]
    fn symlink_tree_relative_target_nested_child_resolves() {
        let (_dir, _src, dest) = symlink_tree_fixture();
        create_symlink_tree(Path::new("../store/dir"), &dest).unwrap();
        assert_eq!(fs::read_to_string(dest.join("sub/file2")).unwrap(), "two");
    }

    #[test]
    fn symlink_tree_relative_target_climbs_once_per_component() {
        // each child resolves from its own directory, one level deeper than
        // the link for each component of its path under the tree
        let (_dir, _src, dest) = symlink_tree_fixture();
        create_symlink_tree(Path::new("../store/dir"), &dest).unwrap();
        assert_eq!(
            fs::read_link(dest.join("file1")).unwrap(),
            Path::new("../../store/dir/file1")
        );
        assert_eq!(
            fs::read_link(dest.join("sub/file2")).unwrap(),
            Path::new("../../../store/dir/sub/file2")
        );
    }

    #[test]
    fn symlink_tree_non_directory_writes_target_as_is() {
        let (_dir, _src, dest) = symlink_tree_fixture();
        create_symlink_tree(Path::new("../store/dir/file1"), &dest).unwrap();
        assert_eq!(
            fs::read_link(&dest).unwrap(),
            Path::new("../store/dir/file1")
        );
        assert_eq!(fs::read_to_string(&dest).unwrap(), "one");
    }
}
