use std::path::{Path, PathBuf, absolute};

pub fn get_symlink_parent(origin_path: &Path) -> &Path {
    // note that this can return the empty path when the origin path consists
    // of a single path segment
    origin_path
        .parent()
        .expect("symlink should not be the filesystem root")
}

pub fn get_absolute_origin_path(origin_path: &Path) -> PathBuf {
    absolute(origin_path).expect("origin path should not be empty")
}
