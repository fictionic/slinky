use std::path::{Path, PathBuf};

/// Gets the parent path of the given symlink, which is known to exist.
/// Just a helper for a runtime invariant.
// TODO: get rid of this function?
pub fn symlink_parent(origin_path: &Path) -> &Path {
    // note that this can return the empty path when the origin path consists
    // of a single path segment
    origin_path
        .parent()
        .expect("symlink should not be the filesystem root")
}

/// Like `Path::parent`, but returns "." instead of "" when `path` is a single-component relative path.
pub fn dirpath(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Like `pathdiff::diff_paths`, but returns "." instead of "" when there is no diff.
pub fn diff_paths(path: &Path, base: &Path) -> Option<PathBuf> {
    let diff = pathdiff::diff_paths(path, base)?;
    Some(if diff.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        diff
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_paths_same_path_is_dot() {
        // a symlink with an empty text cannot be created
        assert_eq!(
            diff_paths(Path::new("/a/b"), Path::new("/a/b")),
            Some(PathBuf::from("."))
        );
    }
}
