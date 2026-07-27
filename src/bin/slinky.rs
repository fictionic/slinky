use anyhow::Result;
use clap::Parser;
use colored::*;
use regex::Regex;
use slinky::cli::{SlinkyCli, SlinkyCommand};
use slinky::create_hard_link;
use slinky::create_hard_link_tree;
use slinky::create_symlink_tree;
use slinky::edit_symlink_target;
use slinky::handle_operation;
use slinky::log_dangling_link;
use slinky::log_link;
use slinky::log_link_err;
use slinky::log_transformation;
use slinky::tidy::PathTidier;
use slinky::util::get_symlink_parent;
use std::fs::{self};
use std::os::unix::fs::symlink;
use std::path::Path;
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

    let edit_target_re = match &cli.command {
        SlinkyCommand::EditTarget { pattern, .. } => Some(Regex::new(pattern)?),
        _ => None,
    };

    let mut walker = WalkDir::new(&cli.path)
        .follow_root_links(false)
        .follow_links(false);
    if let Some(depth) = cli.max_depth {
        walker = walker.max_depth(depth);
    }

    let tidier = PathTidier::new(cli.path);

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let entry_path = entry.path();
        if !entry_path.is_symlink() {
            continue;
        }

        let origin_path = entry_path;
        let origin_path_display = origin_path.display().to_string();

        let target_path = fs::read_link(origin_path)?;
        let target_path_display = target_path.display().to_string();

        let origin_parent_dir_path = get_symlink_parent(origin_path);

        // resolve relative targets against the parent dir
        let target_resolved = if target_path.is_absolute() {
            target_path.clone()
        } else {
            origin_parent_dir_path.join(&target_path)
        };

        // TODO: only check for existence if a filter asks for it.
        // slight performance boost?
        let is_dangling = !target_resolved.exists();
        let is_absolute = target_path.is_absolute();

        // boolean filters
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

        // regex filters
        fn filter_matches(re: &Regex, value: &Path) -> Option<bool> {
            // return None if path isn't valid UTF-8
            value.to_str().map(|s| re.is_match(s))
        }

        let log_unicode_err = |path_type: &str| {
            log_link_err(
                None,
                Some(format!("cannot filter on {} path because it contains invalid unicode", path_type).red()),
                origin_path,
                &target_path,
            );
        };

        if let Some(re) = &origin_filter_re {
            match filter_matches(re, origin_path) {
                Some(true) => {}
                Some(false) => continue, // filtered out
                None => {
                    log_unicode_err("origin");
                    continue;
                }
            }
        }

        if let Some(re) = &target_filter_re {
            match filter_matches(re, &target_path) {
                Some(true) => {}
                Some(false) => continue, // filtered out
                None => {
                    log_unicode_err("target");
                    continue;
                }
            }
        }

        let cmd_name = cli.command.to_string();

        match cli.command {
            SlinkyCommand::List {
                status,
                origin_only,
            } => {
                if origin_only {
                    println!("{}", origin_path.display());
                } else {
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
                        origin_path,
                        &target_path,
                    );
                }
            }

            SlinkyCommand::TidyTarget (opts) => {
                handle_operation(|| {
                    let tidied = tidier.tidy(
                        &target_path,
                        &origin_path,
                        &opts,
                    );

                    if tidied != target_path {
                        if cli.verbose {
                            log_transformation(
                                &cmd_name,
                                origin_path,
                                &target_path,
                                &tidied,
                            );
                        }
                        if !cli.dry_run {
                            edit_symlink_target(origin_path, &tidied)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::Canonicalize => {
                handle_operation(|| {
                    let canonicalized = fs::canonicalize(&target_path)?;

                    if canonicalized != target_path {
                        if cli.verbose {
                            log_transformation(
                                &cmd_name,
                                origin_path,
                                &target_path,
                                &canonicalized,
                            );
                        }
                        if !cli.dry_run {
                            edit_symlink_target(origin_path, canonicalized)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::EditTarget {
                ref replace,
                replace_all,
                ..
            } => {
                let re = edit_target_re.as_ref().expect("EditTarget pattern compiled before the walk");
                if re.is_match(&target_path_display) {
                    handle_operation(|| {
                        let edited = if replace_all {
                            re.replace_all(&target_path_display, replace).into_owned()
                        } else {
                            re.replace(&target_path_display, replace).into_owned()
                        };
                        if edited != target_path_display {
                            if cli.verbose {
                                log_transformation(
                                    &cmd_name,
                                    origin_path,
                                    &target_path,
                                    &edited,
                                );
                            }
                            if !cli.dry_run {
                                edit_symlink_target(origin_path, &edited)?;
                            }
                        }
                        Ok(())
                    });
                }
            }

            SlinkyCommand::ToAbsolute => {
                handle_operation(|| {
                    if !target_path.is_absolute() {
                        // if target_resolved is relative, it's because cli.path
                        // is relative; in this case, we prepend the process cwd
                        // (this is what absolute() does)
                        let absolutified = std::path::absolute(&target_resolved)?;
                        if cli.verbose {
                            log_transformation(
                                &cmd_name,
                                origin_path,
                                &target_path,
                                &absolutified,
                            );
                        }
                        if !cli.dry_run {
                            edit_symlink_target(origin_path, absolutified)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ToRelative { lexical } => {
                handle_operation(|| {
                    if target_path.is_absolute() {
                        let target_abs = std::path::absolute(&target_resolved)?;
                        let origin_parent_abs = if lexical {
                            std::path::absolute(origin_parent_dir_path)?
                        } else {
                            fs::canonicalize(origin_parent_dir_path)?
                        };

                        if let Some(relativized) = pathdiff::diff_paths(&target_abs, &origin_parent_abs) {
                            if cli.verbose {
                                log_transformation(
                                    &cmd_name,
                                    origin_path,
                                    &target_path,
                                    &relativized,
                                );
                            }
                            if !cli.dry_run {
                                fs::remove_file(origin_path)?;
                                symlink(relativized, origin_path)?;
                            }
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ToHardlink => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, origin_path, &target_path);
                    } else if target_resolved.is_dir() {
                        log_link_err(
                            Some(cmd_name.bold()),
                            Some("skipping directory".red()),
                            origin_path,
                            &target_path,
                        );
                    } else {
                        if cli.verbose {
                            log_link(
                                Some(cmd_name.bold()),
                                origin_path,
                                &target_resolved,
                            );
                        }
                        if !cli.dry_run {
                            fs::remove_file(origin_path)?;
                            create_hard_link(&target_resolved, origin_path)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ToTree { hard } => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, origin_path, &target_path);
                    } else if !target_resolved.is_dir() {
                        log_link_err(
                            Some(cmd_name.bold()),
                            Some("skipping file".red()),
                            origin_path,
                            &target_path,
                        );
                    } else {
                        if cli.verbose {
                            log_link(
                                Some(cmd_name.bold()),
                                origin_path,
                                &target_resolved,
                            );
                        }
                        if !cli.dry_run {
                            fs::remove_file(origin_path)?;
                            if hard {
                                create_hard_link_tree(&target_resolved, origin_path)?;
                            } else {
                                create_symlink_tree(&target_resolved, origin_path)?;
                            }
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::ReplaceWithTarget => {
                handle_operation(|| {
                    if is_dangling {
                        log_dangling_link(&cmd_name, origin_path, &target_path);
                    } else {
                        if cli.verbose {
                            log_link(
                                Some(cmd_name.bold()),
                                origin_path,
                                &target_resolved,
                            );
                        }
                        if !cli.dry_run {
                            let actual_target = fs::canonicalize(&target_resolved)?;
                            fs::remove_file(origin_path)?;
                            fs::rename(actual_target, origin_path)?;
                        }
                    }
                    Ok(())
                });
            }

            SlinkyCommand::Remove => {
                if cli.verbose {
                    log_link(
                        Some(cmd_name.bold().red()),
                        origin_path,
                        &target_path,
                    );
                }
                if !cli.dry_run {
                    handle_operation(|| {
                        fs::remove_file(origin_path)?;
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
                            origin_path_display.cyan(),
                            target_path_display.yellow(),
                        );
                    }
                    if !cli.dry_run {
                        Command::new(shell)
                            .arg("-c")
                            .arg(cmd_string)
                            .arg("--")
                            .arg(origin_path)
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
