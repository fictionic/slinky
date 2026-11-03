use std::collections::HashSet;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use fs_err as fs;
use fs_err::os::unix;
use walkdir::WalkDir;

use crate::path::dirpath;
use crate::semantic_path::SemanticPath;

/// Like `fs::canonicalize`, but does not follow a final symlink. In other words, it returns the
/// canonical path to the directory entry that the path indicates.
pub fn canonicalize_nofollow(path: &Path) -> Result<PathBuf> {
    match SemanticPath::try_from(path)?.split_canonical_entry_name() {
        Some((parent_dir_path, entry_name)) => {
            let parent = fs::canonicalize(parent_dir_path.to_path_buf())?;
            Ok(parent.join(entry_name))
        }
        // if there's no canonical entry name, it's safe to canonicalize the whole path, because we
        // know it cannot refer to a symlink.
        _ => Ok(fs::canonicalize(path)?),
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

/// Expands a symlink into a chain of paths, starting from its origin, landing on each intermediate
/// symlink target, and ending on the final target to which it resolves.
///
/// If there is a symlink cycle, returns the chain up until the loop restarts.
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
            // symlink cycle.
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

/// Replaces the non-directory file at `file` with the result of `build_replacement`.
///
/// If `build_replacement` fails, or the built replacement fails to be moved into place, the
/// (maybe partially-built) replacement is removed, and the file is kept as it was.
///
/// Fails when `file` is a directory, or when it does not exist.
pub fn replace_file_with(
    file: &Path,
    build_replacement: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let temp_path = temp_sibling_path(file, temp_counter())?;
    let result = build_replacement(&temp_path).and_then(|()| replace_file(file, &temp_path));
    if let Err(replace_err) = result {
        // clean up
        let do_cleanup = || -> Result<()> {
            match fs::symlink_metadata(&temp_path) {
                Ok(m) if m.is_dir() => fs::remove_dir_all(&temp_path)?,
                Ok(_) => fs::remove_file(&temp_path)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            Ok(())
        };
        if let Err(cleanup_err) = do_cleanup() {
            crate::logging::warn(&format!(
                "Failed to clean up temporary file {}: {cleanup_err:#}",
                &temp_path.display(),
            ));
        }
        return Err(replace_err);
    }
    Ok(())
}

/// Replaces the non-directory file at `file` with the file at `replacement`.
///
/// If `replacement` is not a directory, then the replacement is atomic. Otherwise, the file is
/// moved to a temporary path and deleted once `replacement` is moved into place.
///
/// Fails when `file` is a directory, or when it does not exist.
pub fn replace_file(file: &Path, replacement: &Path) -> Result<()> {
    if fs::symlink_metadata(file)?.is_dir() {
        // we could support this case, but it would add complexity that we don't need in this
        // program.
        anyhow::bail!("File is a directory: {}", file.display());
    }
    if !fs::symlink_metadata(replacement)?.is_dir() {
        // easy case: when `replacement` is not a directory, rename(2) will clobber an existing
        // non-directory file; otherwise it will fail with EISDIR.
        fs::rename(replacement, file)?;
    } else {
        // hard case: when `replacement` is a directory, we have to use a temp file.
        let aside_path = temp_sibling_path(file, temp_counter())?;
        // set the file aside...
        fs::rename(file, &aside_path)?;
        // ...then move the replacement into place...
        if let Err(op_err) = fs::rename(replacement, file) {
            // (if the replacement failed, restore the original file)
            if let Err(restore_err) = fs::rename(&aside_path, file) {
                return Err(anyhow::Error::new(op_err).context(format!(
                    "Failed to move file back from {}: {restore_err:#}",
                    aside_path.display(),
                )));
            }
            return Err(op_err.into());
        } else {
            // ...then remove the temporary file.
            if let Err(cleanup_err) = fs::remove_file(&aside_path) {
                crate::logging::warn(&format!(
                    "Failed to clean up temporary file {}: {cleanup_err:#}",
                    &aside_path.display()
                ));
            }
        }
    }
    Ok(())
}

static TEMP_SIBLING_COUNTER: AtomicU64 = AtomicU64::new(0);
fn temp_counter() -> u64 {
    TEMP_SIBLING_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn temp_sibling_path(path: &Path, count: u64) -> Result<PathBuf> {
    let pid = std::process::id();
    let sibling = path.with_added_extension(format!("slinky-tmp-{}-{}", pid, count));
    match fs::symlink_metadata(&sibling) {
        Ok(_) => Err(anyhow::anyhow!("File exists: {}", sibling.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(sibling),
        Err(e) => Err(e.into()),
    }
    .context("Failed to find available temporary path")
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

    // replace_file

    // the sorted names in a directory. a leftover temp file shows up here
    fn entry_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // base/link -> x, where x does not exist
    fn replace_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let link = dir.path().join("link");
        symlink("x", &link).unwrap();
        (dir, link)
    }

    #[test]
    fn replace_file_puts_file_over_symlink() {
        let (dir, link) = replace_fixture();
        let replacement = dir.path().join("new");
        fs::write(&replacement, "content").unwrap();
        replace_file(&link, &replacement).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_file());
        assert_eq!(fs::read_to_string(&link).unwrap(), "content");
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_puts_directory_over_symlink() {
        // rename(2) cannot put a directory over a non-directory, so the link is set aside first
        let (dir, link) = replace_fixture();
        let replacement = dir.path().join("new");
        fs::create_dir(&replacement).unwrap();
        fs::write(replacement.join("f"), "content").unwrap();
        replace_file(&link, &replacement).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_dir());
        assert_eq!(fs::read_to_string(link.join("f")).unwrap(), "content");
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_puts_file_over_regular_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "old").unwrap();
        let replacement = dir.path().join("new");
        fs::write(&replacement, "content").unwrap();
        replace_file(&file, &replacement).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "content");
        assert_eq!(entry_names(dir.path()), ["file"]);
    }

    #[test]
    fn replace_file_puts_directory_over_regular_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "old").unwrap();
        let replacement = dir.path().join("new");
        fs::create_dir(&replacement).unwrap();
        fs::write(replacement.join("f"), "content").unwrap();
        replace_file(&file, &replacement).unwrap();
        assert!(fs::symlink_metadata(&file).unwrap().is_dir());
        assert_eq!(fs::read_to_string(file.join("f")).unwrap(), "content");
        assert_eq!(entry_names(dir.path()), ["file"]);
    }

    #[test]
    fn replace_file_refuses_directory() {
        // the set-aside path could only remove an empty directory, so a full one would be left
        // behind at a hidden name
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::create_dir(&origin).unwrap();
        fs::write(origin.join("f"), "keep").unwrap();
        let replacement = dir.path().join("new");
        fs::write(&replacement, "content").unwrap();
        assert!(replace_file(&origin, &replacement).is_err());
        assert_eq!(fs::read_to_string(origin.join("f")).unwrap(), "keep");
        assert_eq!(fs::read_to_string(&replacement).unwrap(), "content");
    }

    #[test]
    fn replace_file_missing_replacement_keeps_file() {
        let (dir, link) = replace_fixture();
        assert!(replace_file(&link, &dir.path().join("missing")).is_err());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("x"));
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_directory_move_failure_restores_file() {
        // the link is inside the replacement directory, so moving the directory onto the link
        // fails (EINVAL) after the link is set aside
        let dir = tempdir().unwrap();
        let replacement = dir.path().join("new");
        fs::create_dir(&replacement).unwrap();
        let link = replacement.join("link");
        symlink("x", &link).unwrap();
        assert!(replace_file(&link, &replacement).is_err());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("x"));
        assert_eq!(entry_names(&replacement), ["link"]);
    }

    // replace_file_with

    #[test]
    fn replace_file_with_puts_built_file_in_place() {
        let (dir, link) = replace_fixture();
        replace_file_with(&link, |temp_path| Ok(fs::write(temp_path, "content")?)).unwrap();
        assert_eq!(fs::read_to_string(&link).unwrap(), "content");
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_with_build_failure_removes_partial_directory() {
        let (dir, link) = replace_fixture();
        let result = replace_file_with(&link, |temp_path| {
            fs::create_dir(temp_path)?;
            fs::write(temp_path.join("f"), "")?;
            anyhow::bail!("build failed")
        });
        assert!(format!("{:#}", result.unwrap_err()).contains("build failed"));
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("x"));
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_with_build_failure_removes_partial_file() {
        let (dir, link) = replace_fixture();
        let result = replace_file_with(&link, |temp_path| {
            fs::write(temp_path, "")?;
            anyhow::bail!("build failed")
        });
        assert!(result.is_err());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("x"));
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_with_build_failure_before_creating_anything() {
        let (dir, link) = replace_fixture();
        let result = replace_file_with(&link, |_| anyhow::bail!("build failed"));
        assert!(result.is_err());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("x"));
        assert_eq!(entry_names(dir.path()), ["link"]);
    }

    #[test]
    fn replace_file_with_replace_failure_removes_built_file() {
        // the origin is a directory, so the replace step fails after the build
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::create_dir(&origin).unwrap();
        let result = replace_file_with(&origin, |temp_path| Ok(fs::write(temp_path, "new")?));
        assert!(result.is_err());
        assert!(fs::symlink_metadata(&origin).unwrap().is_dir());
        assert_eq!(entry_names(dir.path()), ["origin"]);
    }

    // temp_sibling_path

    #[test]
    fn temp_sibling_path_differs_by_count() {
        let (dir, link) = replace_fixture();
        let first = temp_sibling_path(&link, 0).unwrap();
        let second = temp_sibling_path(&link, 1).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(dir.path()));
        assert_eq!(second.parent(), Some(dir.path()));
    }

    #[test]
    fn temp_sibling_path_existing_name_is_error() {
        // a leftover from a crash can hold the user's original link, so it is never reused
        let (_dir, link) = replace_fixture();
        let taken = temp_sibling_path(&link, 0).unwrap();
        fs::write(&taken, "").unwrap();
        assert!(temp_sibling_path(&link, 0).is_err());
    }
}
