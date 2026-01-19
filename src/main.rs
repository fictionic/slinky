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

    /// Only search for dangling symlinks.
    #[arg(short = 'g', long, global = true)]
    only_dangling: bool,

    /// Only search on 'attached' (non-dangling) symlinks.
    #[arg(short = 'a', long, global = true)]
    only_attached: bool,

    /// Only search on symlinks whose target string matches the given regex.
    #[arg(short = 't', long, global = true)]
    filter_target: Option<String>,

    /// Descend at most MAX_DEPTH directories
    #[arg(short = 'd', long, global = true)]
    max_depth: Option<usize>,
}

#[derive(Subcommand, Debug)]
#[command(rename_all = "kebab-case")]
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
    /// Run a shell command: $1 = link path, $2 = target string
    Exec { cmd_string: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::List);
    let target_filter_re = cli.filter_target.as_ref().map(|p| Regex::new(p)).transpose()?;

    let mut walker = WalkDir::new(&cli.path).follow_links(false);
    if let Some(depth) = cli.max_depth { walker = walker.max_depth(depth); }

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_symlink() { continue; }

        let target_raw = fs::read_link(path)?;
        let link_dir = path.parent().unwrap_or_else(|| Path::new("."));
        
        let target_resolved = if target_raw.is_absolute() {
            target_raw.clone()
        } else {
            link_dir.join(&target_raw)
        };
        
        let is_dangling = !target_resolved.exists();

        // Filters
        if cli.only_dangling && !is_dangling { continue; }
        if cli.only_attached && is_dangling { continue; }
        if let Some(re) = &target_filter_re {
            if !re.is_match(&target_raw.to_string_lossy()) { continue; }
        }

        match &command {
            Commands::List => {
                let status = if is_dangling { "dangling".red() } else { "attached".green() };
                println!("{}: {} -> {}", status, path.display().to_string().cyan(), target_raw.to_string_lossy().yellow());
            }

            Commands::EditTarget { pattern, replace } => {
                let re = Regex::new(pattern)?;
                let target_str = target_raw.to_string_lossy();
                if re.is_match(&target_str) {
                    let new_target = re.replace_all(&target_str, replace).into_owned();
                    if cli.verbose { println!("{}: {} -> {}", "Edit".magenta(), path.display(), new_target); }
                    if !cli.dry_run {
                        fs::remove_file(path)?;
                        symlink(new_target, path)?;
                    }
                }
            }

            Commands::ToAbsolute => {
                if !target_raw.is_absolute() {
                    // Use canonicalize to resolve the true absolute path
                    let abs_target = fs::canonicalize(&target_resolved)
                        .context(format!("Failed to resolve absolute path for {}", path.display()))?;
                    if cli.verbose { println!("{}: {} -> {}", "Absolute".blue(), path.display(), abs_target.display()); }
                    if !cli.dry_run {
                        fs::remove_file(path)?;
                        symlink(abs_target, path)?;
                    }
                }
            }

            Commands::ToRelative => {
                if target_raw.is_absolute() {
                    // Resolve the target and the link's parent to find the relative difference
                    let abs_target = fs::canonicalize(&target_resolved)?;
                    let abs_link_dir = fs::canonicalize(link_dir)?;
                    
                    if let Some(rel_target) = pathdiff::diff_paths(&abs_target, &abs_link_dir) {
                        if cli.verbose { println!("{}: {} -> {}", "Relative".blue(), path.display(), rel_target.display()); }
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
                if !cli.dry_run {
                    fs::remove_file(path)?;
                    fs::hard_link(&target_resolved, path).context("Hardlink failed (likely cross-device)")?;
                }
            }

            Commands::ToHardlinkTree => {
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
                if !is_dangling && !cli.dry_run {
                    let actual_target = fs::canonicalize(&target_resolved)?;
                    fs::remove_file(path)?;
                    fs::rename(actual_target, path)?;
                }
            }

            Commands::Exec { cmd_string } => {
                let abs_path = fs::canonicalize(path)?;
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                if cli.verbose { println!("{} running command for {}", "Exec:".purple(), path.display()); }
                if !cli.dry_run {
                    Command::new(shell).arg("-c").arg(cmd_string).arg("--")
                        .arg(&abs_path).arg(&target_raw).status()?;
                }
            }
        }
    }
    Ok(())
}
