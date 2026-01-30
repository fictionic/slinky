use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use regex::Regex;
use slinky::{
    cli::{SlinkyCli, SlinkyCommand},
    create_hard_link, create_hard_link_tree, create_symlink_tree, handle_operation, log_dangling_link,
    log_link, log_link_err, log_transformation,
};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

fn main() -> Result<()> {
    let cli = SlinkyCli::parse();

    if !cli.path.exists() {
        anyhow::bail!("{}: No such file or directory", cli.path.display());
    }

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

    let mut walker = WalkDir::new(&cli.path).follow_links(false);
    if let Some(depth) = cli.max_depth {
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
        if cli.only_dangling && !is_dangling {
            continue;
        }
        if cli.only_attached && is_dangling {
            continue;
        }
        if cli.only_absolute && !is_absolute {
            continue;
        }
        if cli.only_relative && is_absolute {
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

        let cmd_name = cli.command.to_string(); // for verbose messages

        match cli.command {
            SlinkyCommand::List { status } => {
                let prefix = if status {
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

            SlinkyCommand::Tidy => {
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
                            std::path::Component::CurDir => {} // Ignore .
                            std::path::Component::ParentDir => {
                                if let Some(std::path::Component::Normal(..)) =
                                    cleaned.components().next_back()
                                {
                                    cleaned.pop();
                                } else if cleaned.as_os_str().is_empty()
                                    || cleaned.components().next_back()
                                        == Some(std::path::Component::ParentDir)
                                {
                                    // Keep leading .. in relative paths or append to existing ..
                                    cleaned.push(component);
                                }
                                // If at RootDir, .. is a no-op
                            }
                            _ => {} // Ignore other component types like Prefix, RootDir
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
                    } else {
                        log_link_err(
                            Some(cmd_name.bold()),
                            Some("target is already tidy".green()),
                            &path.to_string_lossy(),
                            &target_str,
                        );
                    }
                    Ok(())
                });
            }

            SlinkyCommand::EditTarget {
                ref pattern,
                ref replace,
                replace_all,
            } => {
                let re = Regex::new(&pattern)?;
                if re.is_match(&target_str) {
                    handle_operation(|| {
                        let new_target_str = if replace_all {
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

            SlinkyCommand::ToAbsolute => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else if !target_path.is_absolute() {
                        // Use canonicalize to resolve the true absolute path
                        let abs_target = fs::canonicalize(&target_resolved).context(format!(
                            "Failed to resolve absolute path for {}",
                            path.display()
                        ))?;
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

            SlinkyCommand::ToRelative => {
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

            SlinkyCommand::ToHardlink => {
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
                            create_hard_link(&target_resolved, path)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ToHardlinkTree => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else if !target_resolved.is_dir() {
                        log_link_err(
                            Some(cmd_name.bold()),
                            Some("skipping file".red()),
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
                            create_hard_link_tree(&target_resolved, path)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ToTree => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, &path.to_string_lossy(), &target_str);
                    } else if !target_resolved.is_dir() {
                        log_link_err(
                            Some(cmd_name.bold()),
                            Some("skipping file".red()),
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
                            create_symlink_tree(&target_resolved, path)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ReplaceWithTarget => {
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

            SlinkyCommand::Delete => {
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

            SlinkyCommand::Exec { ref cmd_string } => {
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
