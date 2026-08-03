use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "slinky", version, about = "Wrangle symbolic links")]
pub struct SlinkyCli {
    /// The path in which to search for symlinks
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// What to do to each symlink found.
    // TODO: can/should I make 'list' the default command?
    #[command(subcommand)]
    pub command: SlinkyCommand,

    /// Only act on dangling symlinks.
    #[arg(short = 'x', long, conflicts_with = "only_attached")]
    pub only_dangling: bool,

    /// Only act on 'attached' (non-dangling) symlinks.
    #[arg(short = 'a', long)]
    pub only_attached: bool,

    /// Only act on absolute symlinks.
    #[arg(short = 'b', long, conflicts_with = "only_relative")]
    pub only_absolute: bool,

    /// Only act on relative symlinks.
    #[arg(short = 'r', long)]
    pub only_relative: bool,

    /// Only act on symlinks whose origin path matches the given regex
    #[arg(short = 'o', long, value_name = "FILTER")]
    pub filter_origin: Option<String>,

    /// Only act on symlinks whose target string matches the given regex.
    #[arg(short = 't', long, value_name = "FILTER")]
    pub filter_target: Option<String>,

    /// Descend at most NUM directories
    #[arg(short = 'd', long, value_name = "NUM")]
    pub max_depth: Option<usize>,

    /// Describe any changes to be made.
    #[arg(short, long)]
    pub verbose: bool,

    /// Don't make any changes.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

#[derive(Subcommand, Debug, strum::Display, Clone)]
#[command(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum SlinkyCommand {
    /// List symlinks.
    #[command(visible_alias = "ls")]
    List(ListOpts),
    /// Remove redundant elements from symlink target paths.
    ///
    /// By default, tidy-target only makes changes that are guaranteed to
    /// preserve the semantics of the link:
    ///
    /// * empty segments and "." segments are dropped ("a//./b" -> "a/b")
    ///
    /// * "foo/.." is collapsed only when "foo" is a physical directory, so a
    ///   symlink pivot is never silently redirected
    ///
    /// * a trailing-slash directory assertion is retained ("a/b/." -> "a/")
    ///
    /// * paths that climb out of the link's own directory and back in are left
    ///   in place to preserve portability
    ///
    /// The flags below opt into more aggressive folding at the cost of one of
    /// these guarantees.
    TidyTarget(TidyTargetOpts),
    /// Convert symlink paths into their canonical form (absolute path, with all
    /// intermediate symlinks resolved). Fails on dangling symlinks.
    Canonicalize(CanonicalizeOpts),
    /// Convert absolute symlinks to relative symlinks.
    ToRelative(ToRelativeOpts),
    /// Convert relative symlinks to absolute symlinks.
    ToAbsolute(ToAbsoluteOpts),
    /// Edit the target string of symlinks by replacing regex matches.
    EditTarget(EditTargetOpts),
    /// Convert symlinks to hardlinks. Fails on dangling symlinks, symlinks to
    /// directories, and cross-device symlinks.
    ToHardlink(ToHardlinkOpts),
    /// Convert a directory symlink into a directory tree of symlinks to files.
    /// Fails on dangling symlinks.
    ToTree(ToTreeOpts),
    /// Move the target to the symlink's location. Fails on dangling symlinks.
    ReplaceWithTarget(ReplaceWithTargetOpts),
    /// Remove symlinks.
    #[command(visible_alias = "rm")]
    Remove(RemoveOpts),
    /// Run a shell command against symlinks.
    ///
    /// The command must be passed as a single string.
    /// It will be run using $SHELL, with $1 bound to the link origin and $2
    /// bound to the link target.
    Exec(ExecOpts),
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ListOpts {
    /// Prefix the link description with its attached/dangling status.
    #[arg(short, long)]
    pub status: bool,

    /// Print only the origin path.
    #[arg(long)]
    pub origin_only: bool,
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct TidyTargetOpts {
    /// Collapse "in-and-out" dot-dot loops lexically, without consulting the
    /// filesystem. May affect link resolution.
    ///
    /// By default, "foo/.." is only collapsed if foo is a physical directory:
    /// if it is a symlink to a directory, then collapsing would change where
    /// the link points; if it is something else (or nonexistent), collapsing
    /// would still affect the semantics of the link.
    ///
    /// This is useful when working with symlinks that are intentionally
    /// dangling: perhaps they point to an unmounted filesystem, or sit in a
    /// staging directory.
    #[arg(short = 'l', long)]
    pub lexical_in_and_out: bool,

    /// Also collapse "out-and-in" dot-dot loops: relative paths that climb out
    /// of the link's own directory and then back into it. Affects link
    /// portability.
    ///
    /// For example: a link to "../../foo/bar/baz" within the directory /foo/bar
    /// would be collapsed to "baz".
    ///
    /// These can be created accidentally by naive relpath algorithms that only
    /// operate lexically (e.g. slinky to-relative --lexical). However, they are
    /// not always erroneous: a link that begins with a run of dot-dot segments
    /// is portable anywhere at the same depth within the directory that the
    /// path climbs up to. Collapsing would remove the portability.
    ///
    /// Use this option if you don't care about portability and just want the
    /// tidiest links possible.
    #[arg(short = 'c', long)]
    pub collapse_out_and_in: bool,

    /// Strip trailing slashes from target paths.
    ///
    /// The kernel interprets a trailing slash in a filesystem path as an
    /// assertion that the path resolves to a directory. If the path without
    /// the trailing slash resolves to something other than a directory, the
    /// path will fail to resolve.
    ///
    /// By default, slinky retains this assertion while tidying links:
    ///
    /// * "foo/" -> "foo/"
    ///
    /// * "foo/." -> "foo/"
    ///
    /// * "foo/bar/.." -> "foo/" (with caveat from --lexical-in-and-out)
    ///
    /// Use this option if you don't care about the directory assertion and
    /// just want the tidiest links possible.
    #[arg(short = 's', long)]
    pub strip_trailing_slash: bool,
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct CanonicalizeOpts {}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ToRelativeOpts {
    /// Compute the relative target lexically, without resolving symlink
    /// components in the link's own directory.
    ///
    /// The relative target is expressed as a run of ".." segments climbing
    /// from the link's directory up to a common ancestor, then back down to
    /// the target. That climb is only sound if each ".." lands where the
    /// path spells out.
    ///
    /// By default, slinky resolves the link's directory to its physical
    /// location first, so the emitted ".." run matches what the kernel
    /// actually does even when the directory is reached through a symlink.
    ///
    /// With this option, the link's directory is kept as named. This
    /// preserves symlink components (and keeps the transform symmetric with
    /// the target side), but the emitted ".." run becomes unsound when the
    /// directory has symlink components.
    #[arg(short = 'l', long)]
    pub lexical: bool,
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ToAbsoluteOpts {}

#[derive(Args, Debug, Clone, Default)]
pub struct EditTargetOpts {
    pub pattern: String,

    pub replace: String,

    /// Replace all occurrences of the pattern ('global' replace).
    #[arg(short = 'g', long)]
    pub replace_all: bool,
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ToHardlinkOpts {}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ToTreeOpts {
    /// Create hardlinks instead of a symlinks.
    #[arg(short = 'H', long)]
    pub hard: bool,
}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct ReplaceWithTargetOpts {}

#[derive(Args, Debug, Clone, Copy, Default)]
pub struct RemoveOpts {}

#[derive(Args, Debug, Clone, Default)]
pub struct ExecOpts {
    pub cmd_string: String,
}

#[derive(Parser)]
#[command(
    name = "slinky-ln",
    version,
    about = "Create symbolic links without confusion"
)]
pub struct SlinkyLnCli {
    /// The path that the link will point to.
    pub target: String,

    /// The path where the link will live. If a directory is provided, the link will be created inside that directory with the same basename as the target.
    #[arg(default_value = ".")]
    pub origin: Option<String>,

    /// Force creation of the link by overwriting existing files. Will not overwrite directories.
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Transform the target string into an absolute path to the target, if it exists.
    #[arg(short = 'b', long, conflicts_with_all = ["relative", "allow_dangling", "hard", "tree"])]
    pub absolute: bool,

    /// Transform the target string into a relative path to the target, if it exists.
    #[arg(short = 'r', long, conflicts_with_all = ["absolute", "allow_dangling", "hard", "tree"])]
    pub relative: bool,

    /// Dereference the target file if it is a symbolic link.
    #[arg(short = 'L', long)]
    pub dereference: bool,

    /// Allow creation of dangling symlinks.
    #[arg(long, conflicts_with_all = ["absolute", "relative", "hard", "tree"])]
    pub allow_dangling: bool,

    /// Create a hardlink instead of a symlink.
    #[arg(short = 'H', long, conflicts_with_all = ["absolute", "relative", "allow_dangling"])]
    pub hard: bool,

    /// Create a tree of directories and symlinks (or hardlinks if --hard is passed) to mirror a target.
    #[arg(short = 'T', long, conflicts_with_all = ["absolute", "relative", "allow_dangling"])]
    pub tree: bool,

    /// Describe any changes to be made to the filesystem.
    #[arg(short, long)]
    pub verbose: bool,

    /// Don't modify the filesystem.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}
