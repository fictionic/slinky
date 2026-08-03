use std::fs;
use std::os::unix;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use colored::*;
use regex::Regex;

use crate::cli::{
    CanonicalizeOpts, EditTargetOpts, ExecOpts, ListOpts, RemoveOpts, ReplaceWithTargetOpts,
    TidyTargetOpts, ToAbsoluteOpts, ToHardlinkOpts, ToRelativeOpts, ToTreeOpts,
};
use crate::fs::{create_hard_link, create_hard_link_tree, create_symlink_tree, is_cross_device};
use crate::logging::{
    log_dangling_link, log_link_err, log_link_from_cmd, log_link_with_prefix, log_transformation, run_with_error_logger
};
use crate::path::get_symlink_parent;
use crate::tidy::PathTidier;
use crate::walk::{SlinkyCtx, Symlink, SymlinkIter};

pub trait SymlinkCommand {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()>;
}

// commands that only rewrite the target string of each link. returning None leaves the link alone.
trait SetSymlinkTargets {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>>;

    fn set_symlink_targets(&self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            run_with_error_logger(|| {
                let Some(new_target_path) = self.get_new_target(&link)? else {
                    return Ok(());
                };
                if new_target_path != link.target_path {
                    if ctx.verbose {
                        log_transformation(
                            &ctx.cmd_name,
                            &link.origin_path,
                            &link.target_path,
                            &new_target_path,
                        );
                    }
                    if !ctx.dry_run {
                        fs::remove_file(&link.origin_path)?;
                        unix::fs::symlink(new_target_path, &link.origin_path)?;
                    }
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl SymlinkCommand for ListOpts {
    fn run(self, _ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            if self.origin_only {
                println!("{}", link.origin_path.display());
            } else {
                let prefix = if self.status {
                    Some(if link.is_dangling {
                        "dangling".red()
                    } else {
                        "attached".green()
                    })
                } else {
                    None
                };
                log_link_with_prefix(prefix, &link.origin_path, &link.target_path);
            }
        }
        Ok(())
    }
}

// the tidier holds per-run state, so it is built once and paired with the opts
struct TidyTarget {
    opts: TidyTargetOpts,
    tidier: PathTidier,
}

impl SetSymlinkTargets for TidyTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        let tidied = self
            .tidier
            .tidy(&link.target_path, &link.origin_path, &self.opts);
        Ok(Some(PathBuf::from(tidied)))
    }
}

impl SymlinkCommand for TidyTargetOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        TidyTarget {
            opts: self,
            tidier: PathTidier::new(ctx.walker_root_path.clone()),
        }
        .set_symlink_targets(ctx, iter)
    }
}

impl SetSymlinkTargets for CanonicalizeOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        Ok(Some(fs::canonicalize(&link.target_path_resolvable)?))
    }
}

impl SymlinkCommand for CanonicalizeOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        self.set_symlink_targets(ctx, iter)
    }
}

impl SetSymlinkTargets for ToRelativeOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        if !link.is_absolute {
            return Ok(None);
        }
        let target_abs = std::path::absolute(&link.target_path_resolvable)?;
        let origin_parent_dir_path = get_symlink_parent(&link.origin_path);
        let origin_parent_abs = if self.lexical {
            std::path::absolute(origin_parent_dir_path)?
        } else {
            fs::canonicalize(origin_parent_dir_path)?
        };
        Ok(pathdiff::diff_paths(&target_abs, &origin_parent_abs))
    }
}

impl SymlinkCommand for ToRelativeOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        self.set_symlink_targets(ctx, iter)
    }
}

impl SetSymlinkTargets for ToAbsoluteOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        if link.is_absolute {
            return Ok(None);
        }
        // if target_path_resolvable is relative, it's because the walk root is relative; in this
        // case, we prepend the process cwd (this is what absolute() does)
        Ok(Some(std::path::absolute(&link.target_path_resolvable)?))
    }
}

impl SymlinkCommand for ToAbsoluteOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        self.set_symlink_targets(ctx, iter)
    }
}

// the pattern is compiled once and paired with the opts
struct EditTarget {
    opts: EditTargetOpts,
    re: Regex,
}

impl SetSymlinkTargets for EditTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        let target_path_display = link.target_path.display().to_string();
        if !self.re.is_match(&target_path_display) {
            return Ok(None);
        }
        let edited = if self.opts.replace_all {
            self.re
                .replace_all(&target_path_display, &self.opts.replace)
        } else {
            self.re.replace(&target_path_display, &self.opts.replace)
        };
        Ok(Some(PathBuf::from(edited.into_owned())))
    }
}

impl SymlinkCommand for EditTargetOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        let re = Regex::new(&self.pattern)?;
        EditTarget { opts: self, re }.set_symlink_targets(ctx, iter)
    }
}

// commands that replace non-dangling symlinks with something
// derived from their resolved targets
trait ReplaceAttachedLinks {
    type Target;
    fn resolve(&self, link: &Symlink) -> Result<Self::Target>;
    fn skip_reason(&self, link: &Symlink, target: &Self::Target) -> Result<Option<&str>>;
    fn apply(&self, link: &Symlink, target: &Self::Target) -> Result<()>;

    fn replace_links(&self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            run_with_error_logger(|| {
                if link.is_dangling {
                    log_dangling_link(&ctx.cmd_name, &link.origin_path, &link.target_path);
                    return Ok(());
                }
                let target = self.resolve(&link)?;
                if let Some(reason) = self.skip_reason(&link, &target)? {
                    log_link_err(
                        Some(&ctx.cmd_name),
                        Some(reason),
                        &link.origin_path,
                        &link.target_path,
                    );
                    return Ok(());
                }
                if ctx.verbose {
                    log_link_from_cmd(
                        &ctx.cmd_name,
                        &link.origin_path,
                        &link.target_path,
                    );
                }
                if !ctx.dry_run {
                    self.apply(&link, &target)?;
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl ReplaceAttachedLinks for ToHardlinkOpts {
    type Target = ();

    fn resolve(&self, _link: &Symlink) -> Result<()> {
        Ok(())
    }

    fn skip_reason(&self, link: &Symlink, _target: &()) -> Result<Option<&str>> {
        if link.target_path_resolvable.is_dir() {
            return Ok(Some("skipping directory"));
        }
        if is_cross_device(&link.origin_path, &link.target_path_resolvable)? {
            return Ok(Some("skipping cross-device link"));
        }
        Ok(None)
    }

    fn apply(&self, link: &Symlink, _target: &()) -> Result<()> {
        fs::remove_file(&link.origin_path)?;
        create_hard_link(&link.target_path_resolvable, &link.origin_path)
    }
}

impl SymlinkCommand for ToHardlinkOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        self.replace_links(ctx, iter)
    }
}

impl ReplaceAttachedLinks for ToTreeOpts {
    type Target = ();

    fn resolve(&self, _link: &Symlink) -> Result<()> {
        Ok(())
    }

    fn skip_reason(&self, link: &Symlink, _target: &()) -> Result<Option<&str>> {
        if !link.target_path_resolvable.is_dir() {
            return Ok(Some("skipping file"));
        }
        // only the hardlink tree is bound to a single filesystem
        if self.hard && is_cross_device(&link.origin_path, &link.target_path_resolvable)? {
            return Ok(Some("skipping cross-device link"));
        }
        Ok(None)
    }

    fn apply(&self, link: &Symlink, _target: &()) -> Result<()> {
        fs::remove_file(&link.origin_path)?;
        if self.hard {
            create_hard_link_tree(&link.target_path_resolvable, &link.origin_path)
        } else {
            create_symlink_tree(&link.target_path_resolvable, &link.origin_path)
        }
    }
}

impl SymlinkCommand for ToTreeOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        self.replace_links(ctx, iter)
    }
}

impl ReplaceAttachedLinks for ReplaceWithTargetOpts {
    type Target = PathBuf;

    fn resolve(&self, link: &Symlink) -> Result<PathBuf> {
        Ok(fs::canonicalize(&link.target_path_resolvable)?)
    }

    fn skip_reason(&self, link: &Symlink, target: &PathBuf) -> Result<Option<&str>> {
        if is_cross_device(&link.origin_path, target)? {
            return Ok(Some("skipping cross-device link"));
        }
        Ok(None)
    }

    fn apply(&self, link: &Symlink, target: &PathBuf) -> Result<()> {
        if target.is_dir() {
            // moving a file onto an existing symlink auto-deletes the symlink.
            // only have to remove existing if it's a directory.
            // TODO: this is sloppy...
            fs::remove_file(&link.origin_path)?;
        }
        fs::rename(target, &link.origin_path)?;
        Ok(())
    }
}

impl SymlinkCommand for ReplaceWithTargetOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        self.replace_links(ctx, iter)
    }
}

impl SymlinkCommand for RemoveOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            run_with_error_logger(|| {
                if ctx.verbose {
                    log_link_from_cmd(
                        &ctx.cmd_name,
                        &link.origin_path,
                        &link.target_path,
                    );
                }
                if !ctx.dry_run {
                    fs::remove_file(&link.origin_path)?;
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl SymlinkCommand for ExecOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        for link in iter {
            run_with_error_logger(|| {
                if ctx.verbose {
                    println!(
                        "{}: {} {} {}",
                        ctx.cmd_name.bold(),
                        self.cmd_string.blue(),
                        link.origin_path.display().to_string().cyan(),
                        link.target_path.display().to_string().yellow(),
                    );
                }
                if !ctx.dry_run {
                    Command::new(&shell)
                        .arg("-c")
                        .arg(&self.cmd_string)
                        .arg("--")
                        .arg(&link.origin_path)
                        .arg(&link.target_path)
                        .status()?;
                }
                Ok(())
            });
        }
        Ok(())
    }
}
