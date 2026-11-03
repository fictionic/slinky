use std::{
    cell::{Cell, OnceCell},
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use fs_err as fs;
use regex::bytes::Regex;
use walkdir::{DirEntry, WalkDir};

use crate::{cli::SlinkyCli, fs::dereference_symlink, path::symlink_parent};

pub struct Symlink {
    // path to the link origin as given by the fs walk
    pub origin_path: PathBuf,
    // raw target path within the symlink
    pub target_path: PathBuf,
    // target path, resolvable to the correct destination relative to our process CWD
    pub target_path_resolvable: PathBuf,
    pub is_absolute: bool,
    // this requires a syscall, so don't compute unless asked
    is_dangling: OnceCell<bool>,
}

impl Symlink {
    /// Makes a link from its origin path and the target path that `read_link` gives for it.
    pub fn new(origin_path: PathBuf, target_path: PathBuf) -> Self {
        let is_absolute = target_path.is_absolute();
        // resolve relative targets against the parent dir.
        // (...except for the empty path, which we leave as-is. it must dangle.)
        let target_path_resolvable = if is_absolute || target_path.as_os_str().is_empty() {
            target_path.clone()
        } else {
            symlink_parent(&origin_path).join(&target_path)
        };
        Self {
            origin_path,
            target_path,
            target_path_resolvable,
            is_absolute,
            is_dangling: OnceCell::new(),
        }
    }

    pub fn resolve(&self) -> Result<PathBuf> {
        dereference_symlink(&self.target_path_resolvable)
    }

    pub fn is_dangling(&self) -> Result<bool> {
        if let Some(&dangling) = self.is_dangling.get() {
            return Ok(dangling);
        }
        let dangling =
            check_is_dangling(&self.target_path_resolvable).context("cannot access target")?;
        let _ = self.is_dangling.set(dangling);
        Ok(dangling)
    }
}

fn check_is_dangling(target_path_resolvable: &Path) -> io::Result<bool> {
    match target_path_resolvable.try_exists() {
        // the file definitely exists.
        Ok(true) => Ok(false),
        // the file definitely does not exist.
        Ok(false) => Ok(true),
        // despite its docstring, some of try_exists()'s errors are determinate indications that a
        // symlink does not resolve. ELOOP means the kernel had to dereference too many symlinks
        // while resolving the path, and it gave up. ENOTDIR means that a directory component of the
        // path resolves to a non-directory (such as a trailing slash on a path to a regular file).
        // in either case, we know the symlink dangles.
        Err(e) if matches!(e.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => Ok(true),
        // something else went wrong; we don't know whether the symlink dangles or not. could be
        // EACCES.
        Err(e) => Err(e),
    }
}

pub struct SlinkyCtx {
    iter: Cell<Option<SymlinkIter>>,
    pub walker_root_path: PathBuf,
    pub cmd_name: String,
    pub verbose: bool,
    pub dry_run: bool,
    // set when an operation on any link fails, so the process can exit nonzero after it has moved
    // on to the remaining links
    pub(crate) failed: Cell<bool>,
}

pub enum SymlinkSkipCondition {
    /// Symlinks that do not resolve. Some commands need to act on the linked file.
    Dangling,
    /// Symlinks whose target string is empty. Not typically possible to create, but you never know.
    ///
    /// Since the empty string is not a valid path, empty symlinks always dangle, but some commands
    /// that can handle dangling symlinks can't handle empty symlinks.
    Empty,
}

impl SymlinkSkipCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Empty => "skipping empty symlink",
            Self::Dangling => "skipping dangling symlink",
        }
    }

    pub fn matches(&self, link: &Symlink) -> Result<bool> {
        match self {
            Self::Empty => Ok(link.target_path.as_os_str().is_empty()),
            Self::Dangling => link.is_dangling(),
        }
    }
}

impl SlinkyCtx {
    pub fn new(cli: &SlinkyCli, iter: SymlinkIter) -> Self {
        Self {
            iter: Cell::new(Some(iter)),
            walker_root_path: cli.path.clone(),
            cmd_name: cli.command.to_string(),
            verbose: cli.verbose,
            dry_run: cli.dry_run,
            failed: Cell::new(false),
        }
    }

    pub fn failed(&self) -> bool {
        self.failed.get()
    }

    // runs op on each link the walk yields. an error, from the walk or from op, is reported with
    // the path it came from, and the loop moves on to the next item
    pub fn walk<F>(&self, skips: &[SymlinkSkipCondition], mut op: F)
    where
        F: FnMut(&Symlink) -> Result<()>,
    {
        let iter = self.iter.take().expect("walk only runs once per ctx");
        for item in iter {
            match item {
                Ok(link) => {
                    let link_run_result = self.run_unless_skipped(skips, &link, &mut op);
                    if let Err(e) = link_run_result {
                        self.error_on_link(
                            &format!("{e:#}"),
                            &link.origin_path,
                            Some(&link.target_path),
                        );
                    }
                }
                Err(e) => self.error_on_link(
                    &format!("{:#}", e.source),
                    &e.origin_path,
                    e.target_path.as_deref(),
                ),
            }
        }
    }

    fn run_unless_skipped<F>(
        &self,
        skips: &[SymlinkSkipCondition],
        link: &Symlink,
        op: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&Symlink) -> Result<()>,
    {
        for c in skips {
            if c.matches(link)? {
                self.warn_on_link(c.as_str(), &link.origin_path, &link.target_path);
                return Ok(());
            }
        }
        op(link)
    }
}

#[cfg(test)]
impl SlinkyCtx {
    // a ctx whose walk yields `items`, so unit tests can run a command without a real walk
    pub(crate) fn for_test(items: Vec<Result<Symlink, WalkError>>, dry_run: bool) -> Self {
        Self {
            iter: Cell::new(Some(SymlinkIter(Box::new(items.into_iter())))),
            walker_root_path: PathBuf::from("."),
            cmd_name: "test".to_string(),
            verbose: false,
            dry_run,
            failed: Cell::new(false),
        }
    }
}

// a failure in the walk itself, before any command sees the link
pub struct WalkError {
    // the link, or the directory the walk could not read
    pub origin_path: PathBuf,
    // None when the failure came before the target was read
    pub target_path: Option<PathBuf>,
    pub source: anyhow::Error,
}

impl WalkError {
    fn from_walkdir(e: walkdir::Error, walker_root_path: &Path) -> Self {
        // walkdir gives no path when readdir fails partway through a directory
        // (for example, EIO). it does not say which directory, so name the root
        let origin_path = e.path().unwrap_or(walker_root_path).to_path_buf();
        // into_io_error is None only for loop errors, which need
        // follow_links(true), so this does not happen today
        let source = match e.into_io_error() {
            Some(io_err) => io_err,
            None => io::Error::other("filesystem loop"),
        };
        Self {
            origin_path,
            target_path: None,
            source: source.into(),
        }
    }
}

pub struct SymlinkIter(Box<dyn Iterator<Item = Result<Symlink, WalkError>>>);

impl Iterator for SymlinkIter {
    type Item = Result<Symlink, WalkError>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl SymlinkIter {
    pub fn new(cli: &SlinkyCli) -> Result<Self> {
        let origin_filter_re = cli
            .filter_origin
            .as_ref()
            .map(|p| Regex::new(p))
            .transpose()?;
        let target_filter_re = cli
            .filter_target
            .as_ref()
            .map(|p| Regex::new(p))
            .transpose()?;

        let mut walker = WalkDir::new(&cli.path)
            .follow_root_links(false)
            .follow_links(false);
        if let Some(depth) = cli.max_depth {
            walker = walker.max_depth(depth);
        }

        // don't move cli into the closure
        let only_dangling = cli.only_dangling;
        let only_attached = cli.only_attached;
        let only_absolute = cli.only_absolute;
        let only_relative = cli.only_relative;
        let walker_root_path = cli.path.clone();

        // Ok(None) means the entry is not a symlink, or a filter excluded it
        let build_link = move |entry: walkdir::Result<DirEntry>| -> Result<_, WalkError> {
            let entry = entry.map_err(|e| WalkError::from_walkdir(e, &walker_root_path))?;

            if !entry.file_type().is_symlink() {
                return Ok(None);
            }

            let origin_path = entry.into_path();
            // TODO: theoretically we could make this syscall lazier by putting it below filters
            // that don't need it, but probably not worth it
            let target_path = fs::read_link(&origin_path).map_err(|source| WalkError {
                origin_path: origin_path.clone(),
                target_path: None,
                source: source.into(),
            })?;

            // the link struct costs no syscall. its OnceCell holds the lazy is_dangling
            let link = Symlink::new(origin_path, target_path);

            // now filter based on is_absolute (costs nothing; do it as early as possible)
            if only_absolute && !link.is_absolute {
                return Ok(None);
            }
            if only_relative && link.is_absolute {
                return Ok(None);
            }

            // then filter on regexes. they match the raw bytes, so a path that is
            // not valid UTF-8 can still match
            let Symlink {
                origin_path,
                target_path,
                ..
            } = &link;

            if let Some(re) = &origin_filter_re
                && !re.is_match(origin_path.as_os_str().as_bytes())
            {
                return Ok(None);
            }

            if let Some(re) = &target_filter_re
                && !re.is_match(target_path.as_os_str().as_bytes())
            {
                return Ok(None);
            }

            // filter on dangling/attached; it requires a syscall so we do it last
            if only_dangling || only_attached {
                // a target that cannot be checked matches neither filter. it is an
                // error, not a silent skip, and -x rm must never reach it
                let dangling = link.is_dangling().map_err(|source| WalkError {
                    origin_path: link.origin_path.clone(),
                    target_path: Some(link.target_path.clone()),
                    source,
                })?;
                if (only_dangling && !dangling) || (only_attached && dangling) {
                    return Ok(None);
                }
            }

            Ok(Some(link))
        };

        let iter = walker
            .into_iter()
            .filter_map(move |entry| match build_link(entry) {
                // the walk is lazy, so commands change the tree while it runs. for
                // example, replace-with-target removes the intermediate links of a
                // chain, and moves a directory target away. an entry that is gone by
                // the time the walk reads it gives NotFound. there is nothing left to
                // act on, so skip it silently
                Err(e) if is_not_found(&e.source) => None,
                result => result.transpose(),
            });
        Ok(SymlinkIter(Box::new(iter)))
    }
}

fn is_not_found(e: &anyhow::Error) -> bool {
    e.downcast_ref::<io::Error>()
        .is_some_and(|io| io.kind() == io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn link_to(target_path: &Path) -> Symlink {
        Symlink::new(PathBuf::from("link"), target_path.to_path_buf())
    }

    // Symlink::new

    #[test]
    fn symlink_new_relative_target_resolves_from_link_dir() {
        let link = Symlink::new(PathBuf::from("a/b/link"), PathBuf::from("../f"));
        assert_eq!(link.target_path_resolvable, Path::new("a/b/../f"));
        assert!(!link.is_absolute);
    }

    #[test]
    fn symlink_new_absolute_target_is_used_as_is() {
        let link = Symlink::new(PathBuf::from("a/link"), PathBuf::from("/x/f"));
        assert_eq!(link.target_path_resolvable, Path::new("/x/f"));
        assert!(link.is_absolute);
    }

    #[test]
    fn symlink_new_empty_target_stays_empty() {
        // joining "" to the link dir would name the link dir, which exists. the empty target
        // must dangle
        let link = Symlink::new(PathBuf::from("a/link"), PathBuf::new());
        assert_eq!(link.target_path_resolvable, Path::new(""));
        assert!(!link.is_absolute);
    }

    // check_is_dangling

    #[test]
    fn check_is_dangling_empty_path_is_dangling() {
        // linux cannot make an empty link, but readlink can return one from another filesystem
        assert!(check_is_dangling(Path::new("")).unwrap());
    }

    #[test]
    fn check_is_dangling_loop_is_dangling() {
        // try_exists gives ELOOP as an error, not as Ok(false)
        let dir = tempdir().unwrap();
        std::os::unix::fs::symlink("b", dir.path().join("a")).unwrap();
        std::os::unix::fs::symlink("a", dir.path().join("b")).unwrap();
        assert!(check_is_dangling(&dir.path().join("a")).unwrap());
    }

    #[test]
    fn check_is_dangling_path_through_regular_file_is_dangling() {
        // try_exists gives ENOTDIR as an error, not as Ok(false)
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file"), "").unwrap();
        assert!(check_is_dangling(&dir.path().join("file/x")).unwrap());
    }

    // SymlinkSkipCondition

    #[test]
    fn skip_empty_matches_only_empty_target() {
        let empty = link_to(Path::new(""));
        let non_empty = link_to(Path::new("x"));
        assert!(SymlinkSkipCondition::Empty.matches(&empty).unwrap());
        assert!(!SymlinkSkipCondition::Empty.matches(&non_empty).unwrap());
    }

    #[test]
    fn skip_dangling_matches_missing_target() {
        let dir = tempdir().unwrap();
        let link = link_to(&dir.path().join("missing"));
        assert!(SymlinkSkipCondition::Dangling.matches(&link).unwrap());
    }

    #[test]
    fn skip_dangling_does_not_match_existing_target() {
        let dir = tempdir().unwrap();
        let link = link_to(dir.path());
        assert!(!SymlinkSkipCondition::Dangling.matches(&link).unwrap());
    }

    // SlinkyCtx::walk

    fn link(origin_path: &str, target_path: &str) -> Symlink {
        Symlink::new(PathBuf::from(origin_path), PathBuf::from(target_path))
    }

    #[test]
    fn walk_runs_op_on_each_link() {
        let ctx = SlinkyCtx::for_test(vec![Ok(link("a", "x")), Ok(link("b", "x"))], false);
        let mut seen = Vec::new();
        ctx.walk(&[], |link| {
            seen.push(link.origin_path.clone());
            Ok(())
        });
        assert_eq!(seen, [Path::new("a"), Path::new("b")]);
        assert!(!ctx.failed());
    }

    #[test]
    fn walk_does_not_run_op_on_skipped_link() {
        let ctx = SlinkyCtx::for_test(vec![Ok(link("a", "")), Ok(link("b", "x"))], false);
        let mut seen = Vec::new();
        ctx.walk(&[SymlinkSkipCondition::Empty], |link| {
            seen.push(link.origin_path.clone());
            Ok(())
        });
        assert_eq!(seen, [Path::new("b")]);
        // a skip is a correct outcome, not a failure
        assert!(!ctx.failed());
    }

    #[test]
    fn walk_op_error_continues_and_sets_failed() {
        let ctx = SlinkyCtx::for_test(vec![Ok(link("a", "x")), Ok(link("b", "x"))], false);
        let mut seen = Vec::new();
        ctx.walk(&[], |link| {
            seen.push(link.origin_path.clone());
            if link.origin_path == Path::new("a") {
                anyhow::bail!("op failed");
            }
            Ok(())
        });
        assert_eq!(seen, [Path::new("a"), Path::new("b")]);
        assert!(ctx.failed());
    }

    #[test]
    fn walk_error_item_continues_and_sets_failed() {
        let walk_error = WalkError {
            origin_path: PathBuf::from("unreadable"),
            target_path: None,
            source: anyhow::anyhow!("walk failed"),
        };
        let ctx = SlinkyCtx::for_test(vec![Err(walk_error), Ok(link("b", "x"))], false);
        let mut seen = Vec::new();
        ctx.walk(&[], |link| {
            seen.push(link.origin_path.clone());
            Ok(())
        });
        assert_eq!(seen, [Path::new("b")]);
        assert!(ctx.failed());
    }
}
