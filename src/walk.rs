use std::{fs, path::{Path, PathBuf}};
use colored::*;

use anyhow::Result;
use regex::Regex;
use walkdir::WalkDir;

use crate::{cli::SlinkyCli, logging::log_link_err, path::get_symlink_parent};

pub struct Symlink {
    pub origin_path: PathBuf,
    pub target_path: PathBuf,
    pub target_path_resolved: PathBuf,
    pub is_dangling: bool,
    pub is_absolute: bool,
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
    fn next(&mut self) -> Option<Symlink> { self.0.next() }
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

        let iter = walker.into_iter()
            .filter_map(|e| e.ok())
            .map(|f| f.into_path())
            .filter(|f| f.is_symlink())
            .filter_map(move |origin_path| {
                let target_path = fs::read_link(&origin_path).ok()?;

                let origin_parent_dir_path = get_symlink_parent(&origin_path);

                // resolve relative targets against the parent dir
                let target_path_resolved = if target_path.is_absolute() {
                    target_path.clone()
                } else {
                    origin_parent_dir_path.join(&target_path)
                };

                // TODO: only check for existence if a filter asks for it.
                // slight performance boost?
                let is_dangling = !target_path_resolved.exists();
                let is_absolute = target_path.is_absolute();

                // boolean filters
                if only_dangling && !is_dangling {
                    return None;
                }
                if only_attached && is_dangling {
                    return None;
                }
                if only_absolute && !is_absolute {
                    return None;
                }
                if only_relative && is_absolute {
                    return None;
                }

                fn matches_filter(
                    re: &Regex,
                    value: &Path,
                    kind: &str,
                    origin: &Path,
                    target: &Path
                ) -> bool {
                    match value.to_str() {
                        Some(s) => re.is_match(s),
                        None => {
                            log_link_err(
                                None,
                                Some(format!("cannot filter on {} path because it contains invalid unicode",
                                        kind).red()),
                                        origin,
                                        target,
                            );
                            false
                        }
                    }
                }

                if let Some(re) = &origin_filter_re {
                    if !matches_filter(
                        re,
                        &origin_path,
                        "origin",
                        &origin_path,
                        &target_path
                    ) {
                        return None;
                    }
                }

                if let Some(re) = &target_filter_re {
                    if !matches_filter(
                        re,
                        &target_path,
                        "target",
                        &origin_path,
                        &target_path
                    ) {
                        return None;
                    }
                }

                Some(Symlink {
                    origin_path,
                    target_path,
                    target_path_resolved,
                    is_dangling,
                    is_absolute,
                })
            });
        Ok(SymlinkIter(Box::new(iter)))
    }
}

