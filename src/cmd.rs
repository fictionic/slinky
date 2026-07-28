use std::fs;
use std::os::unix;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use colored::*;
use regex::Regex;

use crate::cli::{
    CanonicalizeOpts,
    EditTargetOpts,
    ExecOpts,
    ListOpts,
    RemoveOpts,
    ReplaceWithTargetOpts,
    TidyTargetOpts,
    ToAbsoluteOpts,
    ToHardlinkOpts,
    ToRelativeOpts,
    ToTreeOpts,
};
use crate::logging::{
    log_dangling_link,
    log_link,
    log_link_err,
    log_transformation,
    run_with_error_logger,
};
use crate::tidy::PathTidier;
use crate::path::get_symlink_parent;
use crate::walk::{SlinkyCtx, Symlink, SymlinkIter};
use crate::fs::{
    create_hard_link,
    create_hard_link_tree,
    create_symlink_tree,
};

pub trait SymlinkCommand {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()>;
}

// commands that only rewrite the target string of each link. returning None leaves the link alone.
trait SetTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>>;
}

fn set_target(cmd: &impl SetTarget, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
    for link in iter {
        run_with_error_logger(|| {
            let Some(new_target_path) = cmd.get_new_target(&link)? else {
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
                log_link(prefix, &link.origin_path, &link.target_path);
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

impl SetTarget for TidyTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        let tidied = self
            .tidier
            .tidy(&link.target_path, &link.origin_path, &self.opts);
        Ok(Some(PathBuf::from(tidied)))
    }
}

impl SymlinkCommand for TidyTargetOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        let cmd = TidyTarget {
            opts: self,
            tidier: PathTidier::new(ctx.walker_root_path.clone()),
        };
        set_target(&cmd, ctx, iter)
    }
}

impl SetTarget for CanonicalizeOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        // need to use target_path_abs otherwise it resolves against process CWD
        Ok(Some(fs::canonicalize(&link.target_path_resolved)?))
    }
}

impl SymlinkCommand for CanonicalizeOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        set_target(&self, ctx, iter)
    }
}

impl SetTarget for ToRelativeOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        if !link.is_absolute {
            return Ok(None);
        }
        let target_abs = std::path::absolute(&link.target_path_resolved)?;
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
        set_target(&self, ctx, iter)
    }
}

impl SetTarget for ToAbsoluteOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        if link.is_absolute {
            return Ok(None);
        }
        // if target_path_resolved is relative, it's because the walk root is relative; in this
        // case, we prepend the process cwd (this is what absolute() does)
        Ok(Some(std::path::absolute(&link.target_path_resolved)?))
    }
}

impl SymlinkCommand for ToAbsoluteOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        set_target(&self, ctx, iter)
    }
}

// the pattern is compiled once and paired with the opts
struct EditTarget {
    opts: EditTargetOpts,
    re: Regex,
}

impl SetTarget for EditTarget {
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
        set_target(&EditTarget { opts: self, re }, ctx, iter)
    }
}

impl SymlinkCommand for ToHardlinkOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            run_with_error_logger(|| {
                if link.is_dangling {
                    log_dangling_link(&ctx.cmd_name, &link.origin_path, &link.target_path);
                } else if link.target_path_resolved.is_dir() {
                    log_link_err(
                        Some(ctx.cmd_name.bold()),
                        Some("skipping directory".red()),
                        &link.origin_path,
                        &link.target_path,
                    );
                } else {
                    if ctx.verbose {
                        log_link(
                            Some(ctx.cmd_name.bold()),
                            &link.origin_path,
                            &link.target_path_resolved,
                        );
                    }
                    if !ctx.dry_run {
                        fs::remove_file(&link.origin_path)?;
                        create_hard_link(&link.target_path_resolved, &link.origin_path)?;
                    }
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl SymlinkCommand for ToTreeOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            run_with_error_logger(|| {
                if link.is_dangling {
                    log_dangling_link(&ctx.cmd_name, &link.origin_path, &link.target_path);
                } else if !link.target_path_resolved.is_dir() {
                    log_link_err(
                        Some(ctx.cmd_name.bold()),
                        Some("skipping file".red()),
                        &link.origin_path,
                        &link.target_path,
                    );
                } else {
                    if ctx.verbose {
                        log_link(
                            Some(ctx.cmd_name.bold()),
                            &link.origin_path,
                            &link.target_path_resolved,
                        );
                    }
                    if !ctx.dry_run {
                        fs::remove_file(&link.origin_path)?;
                        if self.hard {
                            create_hard_link_tree(&link.target_path_resolved, &link.origin_path)?;
                        } else {
                            create_symlink_tree(&link.target_path_resolved, &link.origin_path)?;
                        }
                    }
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl SymlinkCommand for ReplaceWithTargetOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            run_with_error_logger(|| {
                if link.is_dangling {
                    log_dangling_link(&ctx.cmd_name, &link.origin_path, &link.target_path);
                } else {
                    if ctx.verbose {
                        log_link(
                            Some(ctx.cmd_name.bold()),
                            &link.origin_path,
                            &link.target_path_resolved,
                        );
                    }
                    if !ctx.dry_run {
                        let actual_target = fs::canonicalize(&link.target_path_resolved)?;
                        fs::remove_file(&link.origin_path)?;
                        fs::rename(actual_target, &link.origin_path)?;
                    }
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl SymlinkCommand for RemoveOpts {
    fn run(self, ctx: &SlinkyCtx, iter: SymlinkIter) -> Result<()> {
        for link in iter {
            if ctx.verbose {
                log_link(
                    Some(ctx.cmd_name.bold().red()),
                    &link.origin_path,
                    &link.target_path,
                );
            }
            if !ctx.dry_run {
                run_with_error_logger(|| {
                    fs::remove_file(&link.origin_path)?;
                    Ok(())
                });
            }
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
