use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use colored::*;
use regex::Regex;

use crate::cli::{
    CanonicalizeOpts, EditTargetOpts, ExecOpts, ListOpts, RemoveOpts, ReplaceWithTargetOpts,
    TidyTargetOpts, ToAbsoluteOpts, ToHardlinkOpts, ToRelativeOpts, ToTreeOpts,
};
use crate::fs::{
    create_hard_link, create_hard_link_tree, create_symlink, create_symlink_tree, is_cross_device,
    symlink_chain,
};
use crate::logging::log_link;
use crate::path::{diff_paths, symlink_parent};
use crate::tidy::{PathTidier};
use crate::walk::{SlinkyCtx, Symlink, SymlinkSkipCondition};

pub trait RunSlinkyCommand {
    // links that this command does not act on, checked in order
    const LINK_SKIPS: &'static [SymlinkSkipCondition];
    fn run(self, ctx: &SlinkyCtx) -> Result<()>;
}

impl RunSlinkyCommand for ListOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        ctx.walk(Self::LINK_SKIPS, |link| {
            if self.origin_only {
                println!("{}", link.origin_path.display());
            } else if self.status {
                match link.is_dangling() {
                    Ok(is_dangling) => {
                        let label = match is_dangling {
                            false => "attached".green(),
                            true => "dangling".red(),
                        };
                        log_link(Some(label), &link.origin_path, &link.target_path);
                    }
                    Err(e) => {
                        log_link(
                            Some("unknown".magenta()),
                            &link.origin_path,
                            &link.target_path,
                        );
                        return Err(e);
                    }
                }
            } else {
                log_link(None, &link.origin_path, &link.target_path);
            }
            Ok(())
        });
        Ok(())
    }
}

// commands that only rewrite the target string of each link. returning None leaves the link alone.
trait RewriteTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>>;

    fn rewrite_targets(&self, skips: &[SymlinkSkipCondition], ctx: &SlinkyCtx) -> Result<()> {
        ctx.walk(skips, |link| {
            let Some(new_target_path) = self.get_new_target(link)? else {
                return Ok(());
            };
            if new_target_path != link.target_path {
                ctx.log_transformation(link, &new_target_path);
                if !ctx.dry_run {
                    fs::remove_file(&link.origin_path)?;
                    create_symlink(&new_target_path, &link.origin_path)?;
                }
            }
            Ok(())
        });
        Ok(())
    }
}

// the tidier holds per-run state, so it is built once and paired with the opts
struct TidyTarget {
    opts: TidyTargetOpts,
    tidier: PathTidier,
}

impl RewriteTarget for TidyTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        let tidied = self
            .tidier
            .tidy(&link.target_path, &link.origin_path, &self.opts);
        Ok(Some(PathBuf::from(tidied)))
    }
}

impl RunSlinkyCommand for TidyTargetOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[SymlinkSkipCondition::Empty];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        TidyTarget {
            opts: self,
            tidier: PathTidier::new(ctx.walker_root_path.clone()).with_context(|| {
                format!("Could not read from {}", ctx.walker_root_path.display())
            })?,
        }
        .rewrite_targets(Self::LINK_SKIPS, ctx)
    }
}

impl RewriteTarget for CanonicalizeOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        Ok(Some(fs::canonicalize(&link.target_path_resolvable)?))
    }
}

impl RunSlinkyCommand for CanonicalizeOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] =
        &[SymlinkSkipCondition::Empty, SymlinkSkipCondition::Dangling];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        self.rewrite_targets(Self::LINK_SKIPS, ctx)
    }
}

impl RewriteTarget for ToRelativeOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        if !link.is_absolute {
            return Ok(None);
        }
        let target_abs = std::path::absolute(&link.target_path_resolvable)?;
        let origin_parent_dir_path = symlink_parent(&link.origin_path);
        let origin_parent_canonical = fs::canonicalize(origin_parent_dir_path)?;
        // there should be no dot-dot components in the base path, so this should always return Some()
        Ok(diff_paths(&target_abs, &origin_parent_canonical))
    }
}

impl RunSlinkyCommand for ToRelativeOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[SymlinkSkipCondition::Empty];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        self.rewrite_targets(Self::LINK_SKIPS, ctx)
    }
}

impl RewriteTarget for ToAbsoluteOpts {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        if link.is_absolute {
            return Ok(None);
        }
        // if target_path_resolvable is relative, it's because the walk root is relative; in this
        // case, we prepend the process cwd (this is what absolute() does)
        Ok(Some(std::path::absolute(&link.target_path_resolvable)?))
    }
}

impl RunSlinkyCommand for ToAbsoluteOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[SymlinkSkipCondition::Empty];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        self.rewrite_targets(Self::LINK_SKIPS, ctx)
    }
}

// the pattern is compiled once and paired with the opts
struct EditTarget {
    opts: EditTargetOpts,
    re: Regex,
}

impl RewriteTarget for EditTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        // a lossy copy would put U+FFFD in place of each invalid byte, and writing
        // it back would silently point the link somewhere else. refuse instead
        let Some(target_path_str) = link.target_path.to_str() else {
            // TODO: does this print a warning properly?
            anyhow::bail!("cannot edit target: it is not valid UTF-8");
        };
        if !self.re.is_match(target_path_str) {
            return Ok(None);
        }
        let edited = if self.opts.replace_all {
            self.re.replace_all(target_path_str, &self.opts.replace)
        } else {
            self.re.replace(target_path_str, &self.opts.replace)
        };
        Ok(Some(PathBuf::from(edited.into_owned())))
    }
}

impl RunSlinkyCommand for EditTargetOpts {
    // skips nothing, so a pattern like `^$` can repair empty links
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        let re = Regex::new(&self.pattern)?;
        EditTarget { opts: self, re }.rewrite_targets(Self::LINK_SKIPS, ctx)
    }
}

// materializing a link needs a target that exists
const MATERIALIZE_LINK_SKIPS: &[SymlinkSkipCondition] =
    &[SymlinkSkipCondition::Empty, SymlinkSkipCondition::Dangling];

// commands that replace non-dangling symlinks with something
// derived from their resolved targets
trait MaterializeTarget {
    // the path to the file that the command acts on. by default this is the end of the symlink
    // chain, but some commands support --physical, in which case it should be just a single hop
    // down the chain.
    fn effective_target(&self, link: &Symlink) -> Result<PathBuf> {
        // TODO: is there a way to dedupe this? the `physical` field is different on each command struct...
        link.resolve()
    }
    fn skip_conditions(&self) -> &'static [MaterializeTargetSkipCondition];
    fn apply(&self, ctx: &SlinkyCtx, link: &Symlink, effective_target: &Path) -> Result<()>;

    fn materialize_links(&self, skips: &[SymlinkSkipCondition], ctx: &SlinkyCtx) -> Result<()> {
        ctx.walk(skips, |link| {
            let effective_target = self.effective_target(link)?;
            for condition in self.skip_conditions() {
                if condition.matches(link, &effective_target)? {
                    ctx.warn_on_link(condition.as_str(), &link.origin_path, &link.target_path);
                    return Ok(());
                }
            }
            ctx.log_link(&link.origin_path, &link.target_path);
            if !ctx.dry_run {
                self.apply(ctx, link, &effective_target)?;
            }
            Ok(())
        });
        Ok(())
    }
}

// materializing a symlink target can impose further conditions on the symlink
enum MaterializeTargetSkipCondition {
    Directory,
    NonDirectory,
    CrossDevice,
}

impl MaterializeTargetSkipCondition {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Directory => "skipping directory symlink",
            Self::NonDirectory => "skipping non-directory symlink",
            Self::CrossDevice => "skipping cross-device symlink",
        }
    }

    fn matches(&self, link: &Symlink, effective_target: &Path) -> Result<bool> {
        Ok(match self {
            Self::Directory => effective_target.is_dir(),
            Self::NonDirectory => !effective_target.is_dir(),
            Self::CrossDevice => is_cross_device(&link.origin_path, effective_target)?,
        })
    }
}

impl MaterializeTarget for ToHardlinkOpts {
    fn skip_conditions(&self) -> &'static [MaterializeTargetSkipCondition] {
        &[
            MaterializeTargetSkipCondition::Directory,
            MaterializeTargetSkipCondition::CrossDevice,
        ]
    }

    fn effective_target(&self, link: &Symlink) -> Result<PathBuf> {
        if self.physical {
            Ok(link.target_path_resolvable.clone())
        } else {
            link.resolve()
        }
    }

    fn apply(&self, _ctx: &SlinkyCtx, link: &Symlink, effective_target: &Path) -> Result<()> {
        fs::remove_file(&link.origin_path)?;
        create_hard_link(effective_target, &link.origin_path)
    }
}

impl RunSlinkyCommand for ToHardlinkOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = MATERIALIZE_LINK_SKIPS;
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        self.materialize_links(Self::LINK_SKIPS, ctx)
    }
}

impl MaterializeTarget for ToTreeOpts {
    fn skip_conditions(&self) -> &'static [MaterializeTargetSkipCondition] {
        if self.hard {
            &[
                MaterializeTargetSkipCondition::NonDirectory,
                MaterializeTargetSkipCondition::CrossDevice,
            ]
        } else {
            &[MaterializeTargetSkipCondition::NonDirectory]
        }
    }

    fn apply(&self, _ctx: &SlinkyCtx, link: &Symlink, effective_target: &Path) -> Result<()> {
        fs::remove_file(&link.origin_path)?;
        if self.hard {
            create_hard_link_tree(effective_target, &link.origin_path)
        } else {
            create_symlink_tree(&link.target_path, &link.origin_path)
        }
    }
}

impl RunSlinkyCommand for ToTreeOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = MATERIALIZE_LINK_SKIPS;
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        self.materialize_links(Self::LINK_SKIPS, ctx)
    }
}

impl MaterializeTarget for ReplaceWithTargetOpts {
    fn skip_conditions(&self) -> &'static [MaterializeTargetSkipCondition] {
        &[MaterializeTargetSkipCondition::CrossDevice]
    }

    fn effective_target(&self, link: &Symlink) -> Result<PathBuf> {
        if self.physical {
            Ok(link.target_path_resolvable.clone())
        } else {
            link.resolve()
        }
    }

    fn apply(&self, ctx: &SlinkyCtx, link: &Symlink, effective_target: &Path) -> Result<()> {
        // we need to take care to clean up any intermediate symlinks.
        // compute the chain before touching anything.
        let chain = if self.physical {
            Vec::new()
        } else {
            symlink_chain(&link.target_path_resolvable)
        };
        if effective_target.is_dir() {
            // moving a file onto an existing symlink auto-deletes the symlink.
            // only have to remove existing if it's a directory.
            // TODO: this feels weird. should make a more robust
            // replace-in-place function using temp files, or something.
            fs::remove_file(&link.origin_path)?;
        }
        fs::rename(effective_target, &link.origin_path)?;

        // cleanup.
        // each window is an intermediate symlink and the path it pointed at; the
        // final element is the target just moved, so it has no window of its own
        for pair in chain.windows(2) {
            let (orphan, pointed_at) = (&pair[0], &pair[1]);
            // the origin is already correct at this point, so a failure to tidy
            // up is reported but does not fail the command
            match fs::remove_file(orphan) {
                Ok(()) => ctx.log_link(orphan, pointed_at),
                Err(e) => ctx.warn_on_link(
                    &format!("could not remove orphaned symlink: {e}"),
                    orphan,
                    pointed_at,
                ),
            }
        }
        Ok(())
    }
}

impl RunSlinkyCommand for ReplaceWithTargetOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = MATERIALIZE_LINK_SKIPS;
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        self.materialize_links(Self::LINK_SKIPS, ctx)
    }
}

impl RunSlinkyCommand for RemoveOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        ctx.walk(Self::LINK_SKIPS, |link| {
            ctx.log_link(&link.origin_path, &link.target_path);
            if !ctx.dry_run {
                fs::remove_file(&link.origin_path)?;
            }
            Ok(())
        });
        Ok(())
    }
}

impl RunSlinkyCommand for ExecOpts {
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        ctx.walk(Self::LINK_SKIPS, |link| {
            if ctx.verbose {
                println!(
                    "{}: {} {} {}",
                    ctx.cmd_name.bold(),
                    self.cmd_string.blue(),
                    link.origin_path.display().to_string().cyan(),
                    link.target_path.display().to_string().magenta(),
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
        Ok(())
    }
}
