use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use slinky::{cli::SlinkyLnCli, create_hard_link, create_hard_link_tree, create_symlink_tree, log_link};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let cli = SlinkyLnCli::parse();

    // read target string from CLI input
    let raw_target_string = cli.target.as_ref().context("Target is required")?;

    // dereference target string if necessary
    let (base_target_path, base_target_string) = if cli.dereference {
        let resolved_path = dereference_symlink(Path::new(raw_target_string));
        let resolved_string = resolved_path.to_string_lossy().to_string();
        (resolved_path, resolved_string)
    } else {
        (PathBuf::from(raw_target_string), raw_target_string.clone())
    };

    // determine where the new link will be created (the 'origin')
    let origin_input = cli.origin.as_deref().map(Path::new).unwrap_or(Path::new("."));
    let origin_path_buf;
    let origin_path = if origin_input.is_dir() {
        let resolved_target = if base_target_path.exists() {
            fs::canonicalize(&base_target_path)?
        } else {
            base_target_path.clone()
        };
        let file_name = resolved_target
            .file_name()
            .context("Could not get basename; target path terminates in ..")?;
        origin_path_buf = origin_input.join(file_name);
        &origin_path_buf
    } else {
        origin_input
    };

    if cli.force && origin_path.exists() {
        fs::remove_file(origin_path)?;
    }

    let target_exists = base_target_path.exists();

    // which type of link are we creating?
    if cli.tree {
        if !target_exists {
            anyhow::bail!("Target does not exist; cannot create tree");
        }
        if cli.hard {
            if cli.verbose {
                let label = "create hardlink tree";
                log_link(
                    Some(label.bold()),
                    &origin_path.display().to_string(),
                    &raw_target_string,
                );
            }
            if !cli.dry_run {
                create_hard_link_tree(&base_target_path, origin_path)?;
            }
        } else {
            if cli.verbose {
                let label = "create symlink tree";
                log_link(
                    Some(label.bold()),
                    &origin_path.display().to_string(),
                    &raw_target_string,
                );
            }
            if !cli.dry_run {
                create_symlink_tree(&base_target_path, origin_path)?;
            }
        }
    } else if cli.hard {
        if !target_exists {
            anyhow::bail!("Target does not exist; cannot create hardlink");
        }
        if cli.verbose {
            let label = "create hardlink";
            log_link(
                Some(label.bold()),
                &origin_path.display().to_string(),
                &raw_target_string,
            );
        }
        if !cli.dry_run {
            create_hard_link(&base_target_path, origin_path)?;
        }
    } else {
        if !target_exists && !cli.allow_dangling {
            anyhow::bail!("Target does not exist; refusing to create dangling symlink without --allow-dangling");
        }
        // transform target string for --relative and --absolute if necessary
        let target_contents = if cli.absolute {
            fs::canonicalize(&base_target_path)?
                .to_string_lossy()
                .to_string()
        } else if cli.relative {
            let abs_target = fs::canonicalize(&base_target_path)?;
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
            base_target_string
        };
        // create the symlink
        if cli.verbose {
            log_link(
                Some("create symlink".bold()),
                &origin_path.display().to_string(),
                &target_contents,
            );
        }
        if !cli.dry_run {
            symlink(&target_contents, origin_path)?;
        }
    }

    Ok(())
}

fn dereference_symlink(path: &Path) -> PathBuf {
    if !path.is_symlink() {
        return path.to_path_buf();
    }
    match fs::canonicalize(path) {
        Ok(resolved) => resolved,
        Err(_) => {
            let mut current = path.to_path_buf();
            while current.is_symlink() {
                if let Ok(next) = fs::read_link(&current) {
                    if next.is_absolute() {
                        current = next;
                    } else if let Some(parent) = current.parent() {
                        current = parent.join(next);
                    } else {
                        current = next;
                    }
                } else {
                    break;
                }
            }
            current
        }
    }
}