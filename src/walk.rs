use std::{
    cell::OnceCell, fs, path::{Path, PathBuf}
};

use anyhow::Result;
use regex::Regex;
use walkdir::WalkDir;

use crate::{cli::SlinkyCli, fs::dereference_symlink, logging::log_link_err, path::get_symlink_parent};

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
        *self.is_dangling
            // TODO: should we be using try_exists()?
            .get_or_init(|| !self.target_path_resolvable.exists())
    }
}

pub struct SlinkyCtx {
    pub walker_root_path: PathBuf,
    pub cmd_name: String,
    pub verbose: bool,
    pub dry_run: bool,
}

pub struct SymlinkIter(Box<dyn Iterator<Item = Symlink>>);

impl Iterator for SymlinkIter {
    type Item = Symlink;
    fn next(&mut self) -> Option<Symlink> {
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

        let iter = walker
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_symlink())
            .map(|e| e.into_path())
            .filter_map(move |origin_path| {
                let target_path = fs::read_link(&origin_path).ok()?;

                let origin_parent_dir_path = get_symlink_parent(&origin_path);

                // resolve relative targets against the parent dir
                let target_path_resolvable = if target_path.is_absolute() {
                    target_path.clone()
                } else {
                    origin_parent_dir_path.join(&target_path)
                };

                let is_absolute = target_path.is_absolute();

                let link = Symlink {
                    origin_path,
                    target_path,
                    target_path_resolvable,
                    is_absolute,
                    is_dangling: OnceCell::new(),
                };

                // apply filters. the ordering here is important for perf.
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
                            log_link_err(
                                None,
                                Some(&format!(
                                    "cannot filter on {} path because it contains invalid unicode",
                                    kind
                                )),
                                origin,
                                target,
                            );
                            false
                        }
                    }
                }

                if let Some(re) = &origin_filter_re
                    && !matches_filter(re, &link.origin_path, "origin", &link.origin_path, &link.target_path)
                {
                    return None;
                }

                if let Some(re) = &target_filter_re
                    && !matches_filter(re, &link.target_path, "target", &link.origin_path, &link.target_path)
                {
                    return None;
                }

                // now the boolean filters
                // first absolute/relative
                if only_absolute && !is_absolute {
                    return None;
                }
                if only_relative && is_absolute {
                    return None;
                }
                // finally is_dangling; it requires a syscall so we do it last
                if only_dangling && !link.is_dangling() {
                    return None;
                }
                if only_attached && link.is_dangling() {
                    return None;
                }

                Some(link)
            });
        Ok(SymlinkIter(Box::new(iter)))
    }
}
