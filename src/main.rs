use anyhow::{Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use colored::*;
use regex::Regex;
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// Wrangle symbolic links
#[derive(Parser)]
#[command(name = "slinky", version = "0.1.0", about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Describe any changes to be made to the filesystem.
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Don't modify the filesystem.
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,

    /// Control color output.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
}

#[derive(ValueEnum, Clone, Debug)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand, Debug, strum::Display)]
#[command(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
enum Commands {
    /// Search for and perform actions on all symlinks under a given path.
    ForEach(ForEachArgs),

    /// Create a new symlink.
    Create {
        /// The target that the link will point to.
        target: String,
        #[arg(help = "The path at which to create the link. Default is the current directory.", long_help = concat!(
                "The path at which to create the link. ",
                "If the path is a directory, the link will be created inside that directory with the same name as the target. ",
                "Default is the current directory."
        ))]
        origin: Option<String>,
        /// Transform the target string into an absolute path to the target, if it exists.
        #[arg(short = 'b', long, conflicts_with_all = ["relative", "allow_dangling"])]
        absolute: bool,
        /// Transform the target string into a relative path to the target, if it exists.
        #[arg(short = 'r', long, conflicts_with_all = ["absolute", "allow_dangling"])]
        relative: bool,
        /// Allow creation of dangling symlinks
        #[arg(long, conflicts_with_all = ["absolute", "relative"])]
        allow_dangling: bool,
    },

    /// Generate shell completions or man pages.
    Generate {
        #[command(subcommand)]
        subcommand: GenerateSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum GenerateSubcommand {
    /// Generate shell completions.
    Completions {
        /// The shell to generate completions for.
        shell: Shell,
    },
    /// Generate a man page.
    Man,
}

#[derive(Args, Debug)]
struct ForEachArgs {
    /// The path in which to search for symlinks
    #[arg(default_value = ".")]
    path: PathBuf,

    /// What to do to each symlink found.
    #[command(subcommand)]
    command: Option<ForEachSubcommand>,

    /// Only act on dangling symlinks.
    #[arg(short = 'x', long)]
    only_dangling: bool,

    /// Only act on 'attached' (non-dangling) symlinks.
    #[arg(short = 'a', long)]
    only_attached: bool,

    /// Only act on absolute symlinks.
    #[arg(short = 'b', long)]
    only_absolute: bool,

    /// Only act on relative symlinks.
    #[arg(short = 'r', long)]
    only_relative: bool,

    /// Only act on symlinks whose origin path matches the given regex
    #[arg(short = 'o', long, value_name = "FILTER")]
    filter_origin: Option<String>,

    /// Only act on symlinks whose target string matches the given regex.
    #[arg(short = 't', long, value_name = "FILTER")]
    filter_target: Option<String>,

    /// Descend at most NUM directories
    #[arg(short = 'd', long, value_name = "NUM")]
    max_depth: Option<usize>,
}

#[derive(Subcommand, Debug, strum::Display)]
#[command(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
enum ForEachSubcommand {
    /// Print a description of each link, formatted as `origin -> target`. This is the default command.
    Print {
        /// Prefix the link description with its (attached/dangling) status
        #[arg(short, long)]
        status: bool,
    },
    /// Convert absolute symlinks to relative symlinks. Fails on dangling symlinks.
    ToRelative,
    /// Convert relative symlinks to absolute symlinks. Fails on dangling symlinks.
    ToAbsolute,
    /// Lexically tidy the target path (e.g., remove redundant `..` or `.`)
    Tidy,
    /// Edit the target string of symlinks by replacing regex matches
    EditTarget {
        pattern: String,
        replace: String,
        /// Replace all occurrences of the pattern ('global' replace)
        #[arg(short = 'g', long)]
        replace_all: bool,
    },
    /// Convert symlinks to hardlinks. Fails on dangling symlinks, symlinks to directories, and cross-device symlinks.
    ToHardlink,
    /// Recursively mirror target directories with hardlinks. Fails on dangling symlinks.
    ToHardlinkTree,
    /// Move the target to the symlink's location. Fails on dangling symlinks.
    ReplaceWithTarget,
    /// Delete symlinks
    Delete,
    /// Run a shell command against symlinks
    #[command(long_about = concat!(
        "Run a shell command against symlinks. ",
        "The command must be passed as a single string. ",
        "It will be run using $SHELL, with $1 bound to the link origin and $2 bound to the link target."
    ))]
    Exec { cmd_string: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.color {
        ColorChoice::Always => control::set_override(true),
        ColorChoice::Never => control::set_override(false),
        ColorChoice::Auto => {}
    }

    // Check global commands first
    match &cli.command {
        Commands::Create {
            target,
            origin,
            absolute,
            relative,
            allow_dangling,
        } => {
            let cmd_name = "create";
            let target_path = Path::new(target);

            if !allow_dangling && !target_path.exists() {
                anyhow::bail!(
                    "Target does not exist; refusing to create dangling symlink without --allow-dangling"
                );
            }

            let origin_root = origin
                .as_deref()
                .map(Path::new)
                .unwrap_or(Path::new("."));
            let origin_path_buf;
            let origin_path = if origin_root.is_dir() {
                let resolved_target = if target_path.exists() {
                    fs::canonicalize(target_path)?
                } else {
                    target_path.to_path_buf()
                };
                let file_name = resolved_target
                    .file_name()
                    .context("Could not get basename; target path terminates in ..")?;
                origin_path_buf = origin_root.join(file_name);
                &origin_path_buf
            } else {
                origin_root
            };

            let final_target = if *absolute {
                fs::canonicalize(target_path)?.to_string_lossy().to_string()
            } else if *relative {
                let abs_target = fs::canonicalize(target_path)?;
                let origin_parent = origin_path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let abs_origin_parent = fs::canonicalize(origin_parent)?;
                pathdiff::diff_paths(&abs_target, &abs_origin_parent)
                    .context("Failed to calculate relative path")?
                    .to_string_lossy()
                    .to_string()
            } else {
                target.clone()
            };

            if cli.verbose {
                log_link(
                    Some(cmd_name.bold()),
                    &origin_path.display().to_string(),
                    &final_target,
                );
            }

            if !cli.dry_run {
                symlink(&final_target, origin_path)?;
            }
            Ok(())
        }
        Commands::ForEach(args) => run_foreach(&cli, args),
        Commands::Generate { subcommand } => match subcommand {
            GenerateSubcommand::Completions { shell } => {
                let mut cmd = Cli::command();
                let bin_name = cmd.get_name().to_string();
                generate(*shell, &mut cmd, bin_name, &mut std::io::stdout());
                Ok(())
            }
            GenerateSubcommand::Man => {
                let cmd = Cli::command();
                let mut buffer: Vec<u8> = Default::default();

                // 1. Render root with header
                clap_mangen::Man::new(cmd.clone()).render(&mut buffer)?;

                // 2. Append subcommands recursively
                for sub in cmd.get_subcommands() {
                    if sub.get_name() == "help" || sub.get_name() == "generate" {
                        continue;
                    }
                    render_subcommand_man(sub, &mut buffer)?;
                }

                std::io::stdout().write_all(&buffer)?;
                Ok(())
            }
        },
    }
}

fn render_subcommand_man(cmd: &clap::Command, buffer: &mut Vec<u8>) -> Result<()> {
    buffer.write_all(format!("\n.SH SUBCOMMAND: {}\n", cmd.get_name().to_uppercase()).as_bytes())?;

    let mut sub_buffer: Vec<u8> = Default::default();
    clap_mangen::Man::new(cmd.clone()).render(&mut sub_buffer)?;
    let man_content = String::from_utf8_lossy(&sub_buffer);

    let mut lines = man_content.lines().peekable();
    let mut passed_name = false;

    while let Some(line) = lines.next() {
        if line.starts_with(".TH") || line.starts_with(".SH NAME") {
            continue;
        }
        // Skip the name/description line right after .SH NAME
        if !passed_name && !line.starts_with('.') && !line.is_empty() {
            passed_name = true;
            continue;
        }
        if line.starts_with(".SH") {
            passed_name = true;
            let demoted = line.replace(".SH", ".SS");
            buffer.write_all(demoted.as_bytes())?;
        } else if passed_name {
            buffer.write_all(line.as_bytes())?;
        } else {
            continue;
        }
        buffer.write_all(b"\n")?;
    }

    for sub in cmd.get_subcommands() {
        render_subcommand_man(sub, buffer)?;
    }
    Ok(())
}

fn run_foreach(cli: &Cli, args: &ForEachArgs) -> Result<()> {
    let default_cmd = ForEachSubcommand::Print { status: false };
    let command = args.command.as_ref().unwrap_or(&default_cmd);
    let cmd_name = command.to_string();

    let origin_filter_re = args
        .filter_origin
        .as_ref()
        .map(|p| Regex::new(p))
        .transpose()?;
    let target_filter_re = args
        .filter_target
        .as_ref()
        .map(|p| Regex::new(p))
        .transpose()?;

    let mut walker = WalkDir::new(&args.path).follow_links(false);
    if let Some(depth) = args.max_depth {
        walker = walker.max_depth(depth);
    }

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_symlink() {
            continue;
        }

        let target_path = fs::read_link(path)?;
        let link_dir = path.parent().unwrap_or_else(|| Path::new("."));

        let target_str = target_path.to_string_lossy(); // for verbose messages

        let target_resolved = if target_path.is_absolute() {
            target_path.clone()
        } else {
            link_dir.join(&target_path)
        };

        let is_dangling = !target_resolved.exists();
        let is_absolute = target_path.is_absolute();

        // Filters
        if args.only_dangling && !is_dangling {
            continue;
        }
        if args.only_attached && is_dangling {
            continue;
        }
        if args.only_absolute && !is_absolute {
            continue;
        }
        if args.only_relative && is_absolute {
            continue;
        }
        if let Some(re) = &origin_filter_re
            && !re.is_match(&path.to_string_lossy())
        {
            continue;
        }
        if let Some(re) = &target_filter_re
            && !re.is_match(&target_path.to_string_lossy())
        {
            continue;
        }

        match command {
            ForEachSubcommand::Print { status } => {
                let prefix = if *status {
                    Some(if is_dangling {
                        "dangling".red()
                    } else {
                        "attached".green()
                    })
                } else {
                    None
                };
                log_link(
                    prefix,
                    &path.display().to_string(),
                    &target_path.to_string_lossy(),
                );
            }

            ForEachSubcommand::Tidy => {
                handle_operation(|| {
                    let mut cleaned = PathBuf::new();
                    let mut components = target_path.components().peekable();

                    // Handle absolute paths / prefixes
                    if let Some(c @ std::path::Component::Prefix(..)) = components.peek() {
                        cleaned.push(c);
                        components.next();
                    }
                    if let Some(c @ std::path::Component::RootDir) = components.peek() {
                        cleaned.push(c);
                        components.next();
                    }

                    for component in components {
                        match component {
                            std::path::Component::Normal(c) => cleaned.push(c),
                            std::path::Component::CurDir => {}
                            std::path::Component::ParentDir => {
                                if let Some(std::path::Component::Normal(..)) = cleaned.components().next_back() {
                                    cleaned.pop();
                                } else if cleaned.as_os_str().is_empty() || cleaned.components().next_back() == Some(std::path::Component::ParentDir) {
                                    // Keep leading .. in relative paths or append to existing ..
                                    cleaned.push(component);
                                }
                                // If at RootDir, .. is a no-op
                            }
                            _ => {}
                        }
                    }

                    let new_target_str = cleaned.to_string_lossy();
                    if new_target_str != target_str {
                        if cli.verbose {
                            log_transformation(
                                &cmd_name,
                                &path.to_string_lossy(),
                                &target_str,
                                &new_target_str,
                            );
                        }
                        if !cli.dry_run {
                            fs::remove_file(path)?;
                            symlink(cleaned, path)?;
                        }
                    }
                    Ok(())
                });
            }

            ForEachSubcommand::EditTarget {
                pattern,
                replace,
                replace_all,
            } => {
                let re = Regex::new(pattern)?;
                if re.is_match(&target_str) {
                    handle_operation(|| {
                        let new_target_str = if *replace_all {
                            re.replace_all(&target_str, replace).into_owned()
                        } else {
                            re.replace(&target_str, replace).into_owned()
                        };
                        if new_target_str != target_str {
                            if cli.verbose {
                                log_transformation(
                                    &cmd_name,
                                    &path.to_string_lossy(),
                                    &target_str,
                                    &new_target_str,
                                );
                            }
                            if !cli.dry_run {
                                fs::remove_file(path)?;
                                symlink(new_target_str, path)?;
                            }
                        } else {
                            log_link_err(
                                Some(cmd_name.bold()),
                                Some("new target is identical to old target".red()),
                                &path.to_string_lossy(),
                                &target_str,
                            );
                        }
                        Ok(())
                    });
                }
            }

            ForEachSubcommand::ToAbsolute => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else if !target_path.is_absolute() {
                        // Use canonicalize to resolve the true absolute path
                        let abs_target = fs::canonicalize(&target_resolved).context(
                            format!("Failed to resolve absolute path for {}", path.display()),
                        )?;
                        if cli.verbose {
                            let new_target_str = abs_target.to_string_lossy();
                            log_transformation(
                                &cmd_name,
                                &path.to_string_lossy(),
                                &target_str,
                                &new_target_str,
                            );
                        }
                        if !cli.dry_run {
                            fs::remove_file(path)?;
                            symlink(abs_target, path)?;
                        }
                    }
                    Ok(())
                });
            }

            ForEachSubcommand::ToRelative => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else if target_path.is_absolute() {
                        // Resolve the target and the link's parent to find the relative difference
                        let abs_target = fs::canonicalize(&target_resolved)?;
                        let abs_link_dir = fs::canonicalize(link_dir)?;

                        if let Some(rel_target) = pathdiff::diff_paths(&abs_target, &abs_link_dir) {
                            let new_target_str = rel_target.to_string_lossy();
                            if cli.verbose {
                                log_transformation(
                                    &cmd_name,
                                    &path.to_string_lossy(),
                                    &target_str,
                                    &new_target_str,
                                );
                            }
                            if !cli.dry_run {
                                fs::remove_file(path)?;
                                symlink(rel_target, path)?;
                            }
                        }
                    }
                    Ok(())
                });
            }

            ForEachSubcommand::ToHardlink => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else if target_resolved.is_dir() {
                        log_link_err(
                            Some(cmd_name.bold()),
                            Some("skipping directory".red()),
                            &path.to_string_lossy(),
                            &target_str,
                        );
                    } else {
                        if cli.verbose {
                            log_link(
                                Some(cmd_name.bold()),
                                &path.to_string_lossy(),
                                &target_resolved.to_string_lossy(),
                            );
                        }
                        if !cli.dry_run {
                            fs::remove_file(path)?;
                            fs::hard_link(&target_resolved, path)
                                .context("Hardlink failed (likely cross-device)")?;
                        }
                    }
                    Ok(())
                });
            }

            ForEachSubcommand::ToHardlinkTree => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else {
                        if cli.verbose {
                            log_link(
                                Some(cmd_name.bold()),
                                &path.to_string_lossy(),
                                &target_resolved.to_string_lossy(),
                            );
                        }
                        if !cli.dry_run {
                            if target_resolved.is_dir() {
                                fs::remove_file(path)?;
                                fs::create_dir_all(path)?;
                                for sub_entry in WalkDir::new(&target_resolved)
                                    .into_iter()
                                    .filter_map(|e| e.ok())
                                {
                                    let rel = sub_entry.path().strip_prefix(&target_resolved)?;
                                    let dest = path.join(rel);
                                    if sub_entry.path().is_dir() {
                                        fs::create_dir_all(&dest)?;
                                    } else {
                                        fs::hard_link(sub_entry.path(), &dest)?;
                                    }
                                }
                            } else {
                                fs::remove_file(path)?;
                                fs::hard_link(&target_resolved, path)?;
                            }
                        }
                    }
                    Ok(())
                });
            }

            ForEachSubcommand::ReplaceWithTarget => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else {
                        if cli.verbose {
                            log_link(
                                Some(cmd_name.bold()),
                                &path.to_string_lossy(),
                                &target_resolved.to_string_lossy(),
                            );
                        }
                        if !cli.dry_run {
                            let actual_target = fs::canonicalize(&target_resolved)?;
                            fs::remove_file(path)?;
                            fs::rename(actual_target, path)?;
                        }
                    }
                    Ok(())
                });
            }

            ForEachSubcommand::Delete => {
                if cli.verbose {
                    log_link(
                        Some(cmd_name.bold().red()),
                        &path.to_string_lossy(),
                        &target_str,
                    );
                }
                if !cli.dry_run {
                    handle_operation(|| {
                        fs::remove_file(path)?;
                        Ok(())
                    });
                }
            }

            ForEachSubcommand::Exec { cmd_string } => {
                handle_operation(|| {
                    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                    if cli.verbose {
                        println!(
                            "{}: {} {} {}",
                            cmd_name.bold(),
                            cmd_string.blue(),
                            path.to_string_lossy().cyan(),
                            target_str.yellow(),
                        );
                    }
                    if !cli.dry_run {
                        Command::new(shell)
                            .arg("-c")
                            .arg(cmd_string)
                            .arg("--")
                            .arg(path)
                            .arg(&target_path)
                            .status()?;
                    }
                    Ok(())
                });
            }
        }
    }
    Ok(())
}

fn handle_operation<F>(op: F)
where
    F: FnOnce() -> Result<()>,
{
    if let Err(e) = op() {
        eprintln!("{}: {}", "Error".red(), e);
    }
}

fn log_dangling_link(cmd: &str, link: &str, target: &str) {
    log_link_err(
        Some(cmd.bold()),
        Some("skipping dangling symlink".red()),
        link,
        target,
    );
}

fn log_link_err(
    cmd: Option<ColoredString>,
    err_msg: Option<ColoredString>,
    link: &str,
    target: &str,
) {
    if let Some(c) = cmd {
        eprint!("{}: ", c);
    }
    if let Some(p) = err_msg {
        eprint!("{}: ", p);
    }
    eprintln!("{} -> {}", link.cyan(), target.yellow());
}

fn log_link(prefix: Option<ColoredString>, link: &str, target: &str) {
    if let Some(p) = prefix {
        print!("{}: ", p);
    }
    println!("{} -> {}", link.cyan(), target.yellow());
}

fn log_transformation(cmd_name: &str, link: &str, old: &str, new: &str) {
    println!(
        "{}: {} -> ({} {} {})",
        cmd_name.bold(),
        link.cyan(),
        old.dimmed(),
        "=>".bright_white(),
        new.yellow()
    );
}
