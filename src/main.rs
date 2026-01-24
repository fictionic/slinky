use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::*;
use regex::Regex;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "slinky", version = "0.1.0")]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,

    /// Describe any changes to be made to the filesystem.
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Don't modify the filesystem.
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,

    /// Only act on dangling symlinks.
    #[arg(short = 'g', long, global = true)]
    only_dangling: bool,

    /// Only act on 'attached' (non-dangling) symlinks.
    #[arg(short = 'a', long, global = true)]
    only_attached: bool,

    /// Only act on absolute symlinks. 
    #[arg( long, global = true)]
    only_absolute: bool,

    /// Only act on relative symlinks.
    #[arg( long, global = true)]
    only_relative: bool,

    /// Only search on symlinks whose target string matches the given regex.
    #[arg(short = 't', long, global = true)]
    filter_target: Option<String>,

    /// Descend at most MAX_DEPTH directories
    #[arg(short = 'd', long, global = true)]
    max_depth: Option<usize>,
}

#[derive(Subcommand, Debug, strum::Display)]
#[command(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")] // for verbose messages
enum Commands {
    /// Prints a list of symlinks in PATH. This is the default command.
    List,
    /// Edits the target string of symlinks by replacing regex matches
    EditTarget { pattern: String, replace: String },
    /// Converts absolute symlinks to relative symlinks
    ToRelative,
    /// Converts relative symlinks to absolute symlinks
    ToAbsolute,
    /// Convert symlinks to hardlinks (fails on directories/cross-device)
    ToHardlink,
    /// Recursively mirror target directories with hardlinks
    ToHardlinkTree,
    /// Move the target to the symlink's location
    ReplaceWithTarget,
    /// Delete symlinks
    Delete,
    /// Run a shell command against symlinks: $1 = link path, $2 = target string
    Exec { cmd_string: String },
    /// Create a symlink to target_file from link_origin
    LinkToFile { 
        target_file: String, 
        link_origin: Option<String>,
        /// Create an absolute symlink
        #[arg(long)]
        absolute: bool,
    },
    /// Create a symlink to target_string from link_origin
    CreateLink { target_string: String, link_origin: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::List);
    let cmd_name = command.to_string();

    match &command {
        Commands::LinkToFile { target_file, link_origin, absolute } => {
            let target_path = Path::new(target_file);
            if !target_path.exists() {
                anyhow::bail!("Target file does not exist: {}", target_file);
            }

            let final_target = if *absolute {
                fs::canonicalize(target_path)?.to_string_lossy().to_string()
            } else {
                target_file.clone()
            };

            let origin_path_buf;
            let origin_path = match link_origin {
                Some(o) => Path::new(o),
                None => {
                    let file_name = target_path.file_name()
                        .context("Target path terminates in ..")?;
                    origin_path_buf = Path::new(".").join(file_name);
                    &origin_path_buf
                }
            };

            if cli.verbose {
                println!("{}: {} -> {}", cmd_name.bold().cyan(), origin_path.display().to_string().cyan(), final_target.yellow());
            }

            if !cli.dry_run {
                if let Some(parent) = origin_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent)?;
                    }
                }
                symlink(&final_target, origin_path)?;
            }
            return Ok(());
        }
        Commands::CreateLink { target_string, link_origin } => {
            if cli.verbose {
                println!("{}: {} -> {}", cmd_name.bold().cyan(), link_origin.cyan(), target_string.yellow());
            }
            if !cli.dry_run {
                let origin = Path::new(link_origin);
                if let Some(parent) = origin.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent)?;
                    }
                }
                symlink(target_string, origin)?;
            }
            return Ok(());
        }
        _ => {}
    }

    let target_filter_re = cli.filter_target.as_ref().map(|p| Regex::new(p)).transpose()?;

    let mut walker = WalkDir::new(&cli.path).follow_links(false);
    if let Some(depth) = cli.max_depth { walker = walker.max_depth(depth); }

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_symlink() { continue; }

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
        if cli.only_dangling && !is_dangling { continue; }
        if cli.only_attached && is_dangling { continue; }
        if cli.only_absolute && !is_absolute { continue; }
        if cli.only_relative && is_absolute { continue; }
        if let Some(re) = &target_filter_re {
            if !re.is_match(&target_path.to_string_lossy()) { continue; }
        }

        match &command {
            Commands::List => {
                let status = if is_dangling { "dangling".red() } else { "attached".green() };
                println!(
                    "{}: {} -> {}",
                    status,
                    path.display().to_string().cyan(),
                    target_path.to_string_lossy().yellow()
                );
            }

            Commands::EditTarget { pattern, replace } => {
                let re = Regex::new(pattern)?;
                if re.is_match(&target_str) {
                    let new_target_str = re.replace_all(&target_str, replace).into_owned();
                    if cli.verbose {
                        log_transformation(
                            &cmd_name,
                            path,
                            &target_str,
                            &new_target_str
                        );
                    }
                    if !cli.dry_run {
                        fs::remove_file(path)?;
                        symlink(new_target_str, path)?;
                    }
                }
            }

            Commands::ToAbsolute => {
                if !target_path.is_absolute() {
                    // Use canonicalize to resolve the true absolute path
                    let abs_target = fs::canonicalize(&target_resolved)
                        .context(format!("Failed to resolve absolute path for {}", path.display()))?;
                    if cli.verbose {
                        let new_target_str = abs_target.to_string_lossy();
                        log_transformation(
                            &cmd_name,
                            path,
                            &target_str,
                            &new_target_str
                        );
                    }
                    if !cli.dry_run {
                        fs::remove_file(path)?;
                        symlink(abs_target, path)?;
                    }
                }
            }

            Commands::ToRelative => {
                if target_path.is_absolute() {
                    // Resolve the target and the link's parent to find the relative difference
                    let abs_target = fs::canonicalize(&target_resolved)?;
                    let abs_link_dir = fs::canonicalize(link_dir)?;

                    if let Some(rel_target) = pathdiff::diff_paths(&abs_target, &abs_link_dir) {
                        let new_target_str = rel_target.to_string_lossy();
                        if cli.verbose {
                            log_transformation(
                                &cmd_name,
                                path,
                                &target_str,
                                &new_target_str
                            );
                        }
                        if !cli.dry_run {
                            fs::remove_file(path)?;
                            symlink(rel_target, path)?;
                        }
                    }
                }
            }

            Commands::ToHardlink => {
                if target_resolved.is_dir() {
                    eprintln!("{}: Cannot hardlink directory {}", "Error".red(), path.display());
                    continue;
                }
                if cli.verbose {
                    println!(
                        "{}: {} -> {}",
                        &cmd_name.bold(),
                        path.to_string_lossy().cyan(),
                        target_resolved.to_string_lossy().yellow()
                    );
                }
                if !cli.dry_run {
                    fs::remove_file(path)?;
                    fs::hard_link(&target_resolved, path).context("Hardlink failed (likely cross-device)")?;
                }
            }

            Commands::ToHardlinkTree => {
                // TODO: describe every filesystem operation individually
                if cli.verbose {
                    println!(
                        "{}: {} -> {}",
                        cmd_name.bold(),
                        path.to_string_lossy().cyan(),
                        target_resolved.to_string_lossy().yellow()
                    );
                }
                if !cli.dry_run {
                    if target_resolved.is_dir() {
                        fs::remove_file(path)?;
                        fs::create_dir_all(path)?;
                        for sub_entry in WalkDir::new(&target_resolved).into_iter().filter_map(|e| e.ok()) {
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

            Commands::ReplaceWithTarget => {
                if cli.verbose {
                    println!(
                        "{}: {} -> {}",
                        &cmd_name.bold(),
                        path.to_string_lossy().cyan(),
                        target_resolved.to_string_lossy().yellow(),
                    );
                }
                if !is_dangling && !cli.dry_run {
                    let actual_target = fs::canonicalize(&target_resolved)?;
                    fs::remove_file(path)?;
                    fs::rename(actual_target, path)?;
                }
            }

            Commands::Delete => {
                if cli.verbose {
                    println!(
                        "{}: {}",
                        cmd_name.bold().red(),
                        path.to_string_lossy().cyan()
                    );
                }
                if !cli.dry_run {
                    fs::remove_file(path)?;
                }
            }

            Commands::Exec { cmd_string } => {
                let abs_path = fs::canonicalize(path)?;
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
                    Command::new(shell).arg("-c").arg(cmd_string).arg("--")
                        .arg(&abs_path).arg(&target_path).status()?;
                }
            }

            Commands::LinkToFile { .. } | Commands::CreateLink { .. } => unreachable!(),
        }
    }
    Ok(())
}

fn log_transformation(cmd: &str, link: impl AsRef<Path>, old: &str, new: &str) {
    println!(
        "{}: {} -> ({} {} {})",
        cmd.bold().cyan(),
        link.as_ref().display(),
        old.dimmed(),
        "=>".bright_black(),
        new.yellow()
    );
}
