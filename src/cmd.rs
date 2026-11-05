use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use colored::*;
use fs_err as fs;
use regex::bytes::Regex;

use crate::cli::{
    CanonicalizeOpts, EditTargetOpts, ExecOpts, ListOpts, RemoveOpts, ReplaceWithTargetOpts,
    TidyTargetOpts, ToAbsoluteOpts, ToHardlinkOpts, ToRelativeOpts, ToTreeOpts,
};
use crate::fs::{
    create_hard_link, create_hard_link_tree, create_symlink, create_symlink_tree, is_cross_device,
    replace_file, replace_file_with, symlink_chain,
};
use crate::logging::log_link;
use crate::path::{diff_paths, dirpath};
use crate::tidy::PathTidier;
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
                    replace_file_with(&link.origin_path, |temp_path| {
                        create_symlink(&new_target_path, temp_path)
                    })?;
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
                format!("could not read from {}", ctx.walker_root_path.display())
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
        let origin_parent_dir_path = dirpath(&link.origin_path);
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

impl EditTarget {
    fn new(opts: EditTargetOpts) -> Result<Self> {
        let re = Regex::new(&opts.pattern)?;
        Ok(Self { opts, re })
    }
}

impl RewriteTarget for EditTarget {
    fn get_new_target(&self, link: &Symlink) -> Result<Option<PathBuf>> {
        // match on the raw bytes, so a target that is not valid UTF-8 keeps each
        // byte outside the match
        let target_bytes = link.target_path.as_os_str().as_bytes();
        if !self.re.is_match(target_bytes) {
            return Ok(None);
        }
        let replace = self.opts.replace.as_bytes();
        let edited = if self.opts.replace_all {
            self.re.replace_all(target_bytes, replace)
        } else {
            self.re.replace(target_bytes, replace)
        };
        Ok(Some(PathBuf::from(OsStr::from_bytes(&edited))))
    }
}

impl RunSlinkyCommand for EditTargetOpts {
    // skips nothing, so a pattern like `^$` can repair empty links
    const LINK_SKIPS: &'static [SymlinkSkipCondition] = &[];
    fn run(self, ctx: &SlinkyCtx) -> Result<()> {
        EditTarget::new(self)?.rewrite_targets(Self::LINK_SKIPS, ctx)
    }
}

// materializing a link needs a target that exists
const MATERIALIZE_LINK_SKIPS: &[SymlinkSkipCondition] =
    &[SymlinkSkipCondition::Empty, SymlinkSkipCondition::Dangling];

// commands that replace non-dangling symlinks with something
// derived from their resolved targets
trait MaterializeTarget {
    // true if the command has --physical and it is set. commands with --physical override this
    fn physical(&self) -> bool {
        false
    }

    // the path to the file that the command acts on. by default this is the end of the symlink
    // chain, but with --physical it is just a single hop down the chain.
    fn effective_target(&self, link: &Symlink) -> Result<PathBuf> {
        if self.physical() {
            Ok(link.target_path_resolvable.clone())
        } else {
            link.resolve()
        }
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

    fn physical(&self) -> bool {
        self.physical
    }

    fn apply(&self, _ctx: &SlinkyCtx, link: &Symlink, effective_target: &Path) -> Result<()> {
        replace_file_with(&link.origin_path, |temp_path| {
            create_hard_link(effective_target, temp_path)
        })
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
        replace_file_with(&link.origin_path, |temp_path| {
            if self.hard {
                create_hard_link_tree(effective_target, temp_path)
            } else {
                create_symlink_tree(&link.target_path, temp_path)
            }
        })
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

    fn physical(&self) -> bool {
        self.physical
    }

    fn apply(&self, ctx: &SlinkyCtx, link: &Symlink, effective_target: &Path) -> Result<()> {
        // we need to take care to clean up any intermediate symlinks. compute the chain before
        // touching anything.
        let chain = if self.physical {
            Vec::new()
        } else {
            symlink_chain(&link.target_path_resolvable)
        };

        // move the target file to the origin path.
        replace_file(&link.origin_path, effective_target)?;

        // cleanup.
        // each window is an intermediate symlink and the path it pointed at; the final element is
        // the target just moved, so it has no window of its own
        for pair in chain.windows(2) {
            let (orphan, pointed_at) = (&pair[0], &pair[1]);
            // the origin is already correct at this point, so a failure to tidy up is reported but
            // does not fail the command
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
        if !self.force && !ctx.dry_run {
            anyhow::bail!("refusing to remove symlinks without -f; use -nv to preview");
        }
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
                let status = Command::new("/bin/sh") // POSIX shell, for $1/$2 semantics
                    .arg("-c")
                    .arg(&self.cmd_string)
                    .arg("slinky") // set $0 to "slinky", for shell error messages
                    .arg(&link.origin_path)
                    .arg(&link.target_path)
                    .status()?;
                // a failure on one link is reported, and the walk moves on to the next
                if !status.success() {
                    anyhow::bail!("command failed ({status})");
                }
            }
            Ok(())
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::os::unix::fs::{MetadataExt, symlink};
    use tempfile::tempdir;

    fn link(origin_path: impl Into<PathBuf>, target_path: impl Into<PathBuf>) -> Symlink {
        Symlink::new(origin_path.into(), target_path.into())
    }

    // a canonical temp dir, so that expected paths compare equal to computed ones
    fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        (dir, base)
    }

    // EditTarget

    fn edit_target(pattern: &str, replace: &str, replace_all: bool) -> EditTarget {
        EditTarget::new(EditTargetOpts {
            pattern: pattern.to_string(),
            replace: replace.to_string(),
            replace_all,
        })
        .unwrap()
    }

    #[test]
    fn edit_target_invalid_pattern_is_error() {
        let opts = EditTargetOpts {
            pattern: "(".to_string(),
            ..Default::default()
        };
        assert!(EditTarget::new(opts).is_err());
    }

    #[test]
    fn edit_target_no_match_leaves_link_alone() {
        let edit = edit_target("nomatch", "x", false);
        assert_eq!(
            edit.get_new_target(&link("link", "orig.txt")).unwrap(),
            None
        );
    }

    #[test]
    fn edit_target_replaces_first_match() {
        let edit = edit_target("a", "b", false);
        assert_eq!(
            edit.get_new_target(&link("link", "a-a.txt")).unwrap(),
            Some(PathBuf::from("b-a.txt"))
        );
    }

    #[test]
    fn edit_target_replace_all_replaces_every_match() {
        let edit = edit_target("a", "b", true);
        assert_eq!(
            edit.get_new_target(&link("link", "a-a.txt")).unwrap(),
            Some(PathBuf::from("b-b.txt"))
        );
    }

    #[test]
    fn edit_target_pattern_is_a_regex() {
        // the dot is escaped, so it matches only a dot
        let edit = edit_target(r"1\.0", "2.0", false);
        assert_eq!(
            edit.get_new_target(&link("link", "version-1.0.txt"))
                .unwrap(),
            Some(PathBuf::from("version-2.0.txt"))
        );
        assert_eq!(
            edit.get_new_target(&link("link", "version-1x0.txt"))
                .unwrap(),
            None
        );
    }

    #[test]
    fn edit_target_can_fill_empty_link() {
        let edit = edit_target("^$", "new", false);
        assert_eq!(
            edit.get_new_target(&link("link", "")).unwrap(),
            Some(PathBuf::from("new"))
        );
    }

    #[test]
    fn edit_target_non_utf8_target_keeps_other_bytes() {
        let edit = edit_target("old", "new", false);
        let target = OsStr::from_bytes(b"/old/p\xffath");
        assert_eq!(
            edit.get_new_target(&link("link", target)).unwrap(),
            Some(PathBuf::from(OsStr::from_bytes(b"/new/p\xffath")))
        );
    }

    #[test]
    fn edit_target_can_match_raw_byte() {
        // without unicode mode, \xff matches the raw byte
        let edit = edit_target(r"(?-u:\xff)", "f", false);
        let target = OsStr::from_bytes(b"/old/p\xffath");
        assert_eq!(
            edit.get_new_target(&link("link", target)).unwrap(),
            Some(PathBuf::from("/old/pfath"))
        );
    }

    // ToAbsoluteOpts

    #[test]
    fn to_absolute_leaves_absolute_link_alone() {
        let new = ToAbsoluteOpts {}.get_new_target(&link("link", "/x/f"));
        assert_eq!(new.unwrap(), None);
    }

    #[test]
    fn to_absolute_resolves_target_from_link_dir() {
        // `..` is kept, because the link dir can be a symlink
        let new = ToAbsoluteOpts {}.get_new_target(&link("/base/sub/link", "../f"));
        assert_eq!(new.unwrap(), Some(PathBuf::from("/base/sub/../f")));
    }

    #[test]
    fn to_absolute_relative_walk_root_resolves_from_cwd() {
        let new = ToAbsoluteOpts {}.get_new_target(&link("sub/link", "f"));
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(new.unwrap(), Some(cwd.join("sub/f")));
    }

    // ToRelativeOpts

    #[test]
    fn to_relative_leaves_relative_link_alone() {
        let new = ToRelativeOpts {}.get_new_target(&link("link", "f"));
        assert_eq!(new.unwrap(), None);
    }

    #[test]
    fn to_relative_target_is_relative_to_link_dir() {
        // the target does not need to exist
        let (_dir, base) = canonical_tempdir();
        fs::create_dir(base.join("links")).unwrap();
        let new =
            ToRelativeOpts {}.get_new_target(&link(base.join("links/link"), base.join("data/f")));
        assert_eq!(new.unwrap(), Some(PathBuf::from("../data/f")));
    }

    #[test]
    fn to_relative_link_to_own_directory_is_dot() {
        // the relative path from a directory to itself is ".", not "". an empty target cannot be
        // written
        let (_dir, base) = canonical_tempdir();
        let new = ToRelativeOpts {}.get_new_target(&link(base.join("self"), base.clone()));
        assert_eq!(new.unwrap(), Some(PathBuf::from(".")));
    }

    #[test]
    fn to_relative_counts_climbs_from_physical_link_dir() {
        // the link is named through shallow -> real/deep/sub. the kernel reads the link text from
        // the physical directory, so the `..` climbs are counted from there
        let (_dir, base) = canonical_tempdir();
        fs::create_dir_all(base.join("real/deep/sub")).unwrap();
        symlink("real/deep/sub", base.join("shallow")).unwrap();
        let new = ToRelativeOpts {}
            .get_new_target(&link(base.join("shallow/mylink"), base.join("target.txt")));
        assert_eq!(new.unwrap(), Some(PathBuf::from("../../../target.txt")));
    }

    #[test]
    fn to_relative_link_dir_with_parent_component() {
        // a walk root such as "../d" gives a link dir with `..` in it
        let (_dir, base) = canonical_tempdir();
        fs::create_dir(base.join("cwd")).unwrap();
        fs::create_dir(base.join("d")).unwrap();
        let new = ToRelativeOpts {}.get_new_target(&link(
            base.join("cwd/../d/link"),
            base.join("cwd/target.txt"),
        ));
        assert_eq!(new.unwrap(), Some(PathBuf::from("../cwd/target.txt")));
    }

    // RewriteTarget::rewrite_targets

    // a rewrite that gives a fixed new target, or leaves the link alone with None
    struct FixedTarget(Option<&'static str>);

    impl RewriteTarget for FixedTarget {
        fn get_new_target(&self, _link: &Symlink) -> Result<Option<PathBuf>> {
            Ok(self.0.map(PathBuf::from))
        }
    }

    // base/link -> old. also returns the inode of the link, which changes when it is rewritten
    fn rewrite_fixture() -> (tempfile::TempDir, PathBuf, u64) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("link");
        symlink("old", &path).unwrap();
        let ino = fs::symlink_metadata(&path).unwrap().ino();
        (dir, path, ino)
    }

    fn run_rewrite(rewrite: FixedTarget, path: &Path, dry_run: bool) -> SlinkyCtx {
        let ctx = SlinkyCtx::for_test(vec![Ok(link(path, "old"))], dry_run);
        rewrite.rewrite_targets(&[], &ctx).unwrap();
        ctx
    }

    #[test]
    fn rewrite_targets_writes_new_target() {
        let (_dir, path, _ino) = rewrite_fixture();
        let ctx = run_rewrite(FixedTarget(Some("new")), &path, false);
        assert_eq!(fs::read_link(&path).unwrap(), Path::new("new"));
        assert!(!ctx.failed());
    }

    #[test]
    fn rewrite_targets_dry_run_writes_nothing() {
        let (_dir, path, ino) = rewrite_fixture();
        run_rewrite(FixedTarget(Some("new")), &path, true);
        assert_eq!(fs::read_link(&path).unwrap(), Path::new("old"));
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), ino);
    }

    #[test]
    fn rewrite_targets_none_leaves_link_alone() {
        let (_dir, path, ino) = rewrite_fixture();
        run_rewrite(FixedTarget(None), &path, false);
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), ino);
    }

    #[test]
    fn rewrite_targets_unchanged_target_leaves_link_alone() {
        let (_dir, path, ino) = rewrite_fixture();
        run_rewrite(FixedTarget(Some("old")), &path, false);
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), ino);
    }

    // MaterializeTarget::materialize_links

    // a command that records which links reach apply
    struct RecordApply {
        skips: &'static [MaterializeTargetSkipCondition],
        applied: RefCell<Vec<PathBuf>>,
    }

    impl MaterializeTarget for RecordApply {
        fn skip_conditions(&self) -> &'static [MaterializeTargetSkipCondition] {
            self.skips
        }

        fn apply(&self, _ctx: &SlinkyCtx, link: &Symlink, _effective_target: &Path) -> Result<()> {
            self.applied.borrow_mut().push(link.origin_path.clone());
            Ok(())
        }
    }

    // walks to_dir -> base/dir and to_file -> base/file, and returns the links that reach apply.
    // the skips read only the targets, so the links themselves need not exist
    fn run_materialize(
        skips: &'static [MaterializeTargetSkipCondition],
        dry_run: bool,
    ) -> Vec<PathBuf> {
        let (_dir, base) = canonical_tempdir();
        fs::create_dir(base.join("dir")).unwrap();
        fs::write(base.join("file"), "").unwrap();
        let ctx = SlinkyCtx::for_test(
            vec![
                Ok(link("to_dir", base.join("dir"))),
                Ok(link("to_file", base.join("file"))),
            ],
            dry_run,
        );
        let cmd = RecordApply {
            skips,
            applied: RefCell::new(Vec::new()),
        };
        cmd.materialize_links(&[], &ctx).unwrap();
        cmd.applied.into_inner()
    }

    #[test]
    fn materialize_links_applies_each_link() {
        assert_eq!(
            run_materialize(&[], false),
            ["to_dir", "to_file"].map(PathBuf::from)
        );
    }

    #[test]
    fn materialize_links_does_not_apply_skipped_link() {
        let applied = run_materialize(&[MaterializeTargetSkipCondition::Directory], false);
        assert_eq!(applied, [PathBuf::from("to_file")]);
    }

    #[test]
    fn materialize_links_dry_run_does_not_apply() {
        assert!(run_materialize(&[], true).is_empty());
    }

    // RemoveOpts

    #[test]
    fn remove_removes_link() {
        let (_dir, path, _ino) = rewrite_fixture();
        let ctx = SlinkyCtx::for_test(vec![Ok(link(&path, "old"))], false);
        RemoveOpts { force: true }.run(&ctx).unwrap();
        assert!(fs::symlink_metadata(&path).is_err());
    }

    #[test]
    fn remove_without_force_is_error_and_keeps_link() {
        let (_dir, path, _ino) = rewrite_fixture();
        let ctx = SlinkyCtx::for_test(vec![Ok(link(&path, "old"))], false);
        assert!(RemoveOpts { force: false }.run(&ctx).is_err());
        assert_eq!(fs::read_link(&path).unwrap(), Path::new("old"));
    }

    #[test]
    fn remove_dry_run_keeps_link() {
        let (_dir, path, _ino) = rewrite_fixture();
        let ctx = SlinkyCtx::for_test(vec![Ok(link(&path, "old"))], true);
        RemoveOpts { force: true }.run(&ctx).unwrap();
        assert_eq!(fs::read_link(&path).unwrap(), Path::new("old"));
    }

    #[test]
    fn remove_dry_run_without_force_keeps_link() {
        // a preview does not need --force
        let (_dir, path, _ino) = rewrite_fixture();
        let ctx = SlinkyCtx::for_test(vec![Ok(link(&path, "old"))], true);
        RemoveOpts { force: false }.run(&ctx).unwrap();
        assert_eq!(fs::read_link(&path).unwrap(), Path::new("old"));
    }
}
