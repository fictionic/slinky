use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "slinky", version = "0.1.0", about = "Wrangle symbolic links")]
pub struct SlinkyCli {
    /// The path in which to search for symlinks
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// What to do to each symlink found.
    #[command(subcommand)]
    pub command: SlinkyCommand,

    /// Only act on dangling symlinks.
    #[arg(short = 'x', long)]
    pub only_dangling: bool,

    /// Only act on 'attached' (non-dangling) symlinks.
    #[arg(short = 'a', long)]
    pub only_attached: bool,

    /// Only act on absolute symlinks.
    #[arg(short = 'b', long)]
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
    List {
        /// Prefix the link description with its attached/dangling status.
        #[arg(short, long)]
        status: bool,

        /// Print only the origin path.
        #[arg(long)]
        origin_only: bool,
    },
    /// Convert absolute symlinks to relative symlinks. Fails on dangling symlinks.
    ToRelative,
    /// Convert relative symlinks to absolute symlinks. Fails on dangling symlinks.
    ToAbsolute,
    /// Lexically tidy the target path (e.g., remove redundant `..` or `.`)
    Tidy,
    /// Edit the target string of symlinks by replacing regex matches.
    EditTarget {
        pattern: String,
        replace: String,
        /// Replace all occurrences of the pattern ('global' replace).
        #[arg(short = 'g', long)]
        replace_all: bool,
    },
    /// Convert symlinks to hardlinks. Fails on dangling symlinks, symlinks to directories, and cross-device symlinks.
    ToHardlink,
    /// Convert a directory symlink into a directory tree of symlinks to files. Fails on dangling symlinks.
    ToTree {
        /// Create hardlinks instead of a symlinks.
        #[arg(short = 'H', long)]
        hard: bool,
    },
    /// Move the target to the symlink's location. Fails on dangling symlinks.
    ReplaceWithTarget,
    /// Remove symlinks.
    #[command(visible_alias = "rm")]
    Remove,
    /// Run a shell command against symlinks.
    #[command(long_about = concat!(
        "Run a shell command against symlinks. ",
        "The command must be passed as a single string. ",
        "It will be run using $SHELL, with $1 bound to the link origin and $2 bound to the link target."
    ))]
    Exec { cmd_string: String },
}

#[derive(Parser)]
#[command(name = "slinky-ln", version = "0.1.0", about = "Create symbolic links without confusion")]
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
