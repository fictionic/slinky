use std::{
    cell::{Cell, OnceCell},
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::Result;
use regex::Regex;
use walkdir::WalkDir;

use crate::{
    cli::SlinkyCli, fs::dereference_symlink, logging::warn_on_link, path::get_symlink_parent,
};

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
    pub fn resolve(&self) -> Result<PathBuf> {
        dereference_symlink(&self.target_path_resolvable)
    }
    pub fn is_dangling(&self) -> bool {
        *self
            .is_dangling
            // TODO: should we be using try_exists()?
            .get_or_init(|| !self.target_path_resolvable.exists())
    }
}

pub struct SlinkyCtx {
    pub walker_root_path: PathBuf,
    pub cmd_name: String,
    pub verbose: bool,
    pub dry_run: bool,
    // set when an operation on any link fails, so the process can exit nonzero
    // after it has moved on to the remaining links
    pub(crate) failed: Cell<bool>,
}

impl SlinkyCtx {
    pub fn new(cli: &SlinkyCli) -> Self {
        Self {
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
}

// a failure in the walk itself, before any command sees the link
pub struct WalkError {
    // the link, or the directory the walk could not read
    pub origin_path: PathBuf,
    // None when the failure came before the target was read
    pub target_path: Option<PathBuf>,
    pub source: io::Error,
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
            source,
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

        // the walk is lazy, so commands change the tree while it runs. for example,
        // replace-with-target removes the intermediate links of a chain, and moves
        // a directory target away. an entry that is gone by the time the walk reads
        // it gives NotFound. there is nothing left to act on, so skip it silently
        let is_gone = |e: &io::Error| e.kind() == io::ErrorKind::NotFound;

        let iter = walker.into_iter().filter_map(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) if e.io_error().is_some_and(is_gone) => return None,
                Err(e) => return Some(Err(WalkError::from_walkdir(e, &walker_root_path))),
            };

            if !entry.file_type().is_symlink() {
                return None;
            }

            let origin_path = entry.into_path();
            // TODO: theoretically we could make this syscall lazier by putting it below filters
            // that don't need it, but probably not worth it
            let target_path = match fs::read_link(&origin_path) {
                Ok(target_path) => target_path,
                Err(source) if is_gone(&source) => return None,
                Err(source) => {
                    return Some(Err(WalkError {
                        origin_path,
                        target_path: None,
                        source,
                    }));
                }
            };

            let origin_parent_dir_path = get_symlink_parent(&origin_path);

            // resolve relative targets against the parent dir
            let target_path_resolvable = if target_path.is_absolute() {
                target_path.clone()
            } else {
                origin_parent_dir_path.join(&target_path)
            };

            let is_absolute = target_path.is_absolute();

            // now filter based on is_absolute (costs nothing; do it as early as possible)
            if only_absolute && !is_absolute {
                return None;
            }
            if only_relative && is_absolute {
                return None;
            }

            // first filter on regexes
            fn matches_filter(
                re: &Regex,
                value: &Path,
                kind: &str,
                origin: &Path,
                target: &Path,
            ) -> bool {
                match value.to_str() {
                    Some(s) => re.is_match(s),
                    None => {
                        warn_on_link(
                            &format!(
                                "cannot filter on {} path because it contains invalid unicode",
                                kind
                            ),
                            origin,
                            target,
                        );
                        false
                    }
                }
            }

            if let Some(re) = &origin_filter_re
                && !matches_filter(re, &origin_path, "origin", &origin_path, &target_path)
            {
                return None;
            }

            if let Some(re) = &target_filter_re
                && !matches_filter(re, &target_path, "target", &origin_path, &target_path)
            {
                return None;
            }

            // assemble the link struct now, since we need the OnceCell for the lazy is_dangling
            let link = Symlink {
                origin_path,
                target_path,
                target_path_resolvable,
                is_absolute,
                is_dangling: OnceCell::new(),
            };

            // filter on is_dangling; it requires a syscall so we do it last
            if only_dangling && !link.is_dangling() {
                return None;
            }
            if only_attached && link.is_dangling() {
                return None;
            }

            Some(Ok(link))
        });
        Ok(SymlinkIter(Box::new(iter)))
    }
}
