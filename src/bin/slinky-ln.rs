use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use slinky::cli::SlinkyLnCli;
use slinky::fs::{
    create_hard_link, create_hard_link_tree, create_symlink_tree, dereference_symlink, is_cross_device,
};
use slinky::logging::log_link_with_prefix;
use std::fs;
use std::os::unix;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let cli = SlinkyLnCli::parse();

    // read target string from CLI input
    let raw_target_string = &cli.target;

    // dereference target string if necessary
    let base_target_path = if cli.dereference {
        dereference_symlink(Path::new(raw_target_string))?
    } else {
        PathBuf::from(raw_target_string)
    };

    // determine where the new link will be created (the 'origin')
    let origin_input = cli
        .origin
        .as_deref()
        .map(Path::new)
        .unwrap_or(Path::new("."));
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
            // TODO: ^ can't we just traverse path segments backwards?
        origin_path_buf = origin_input.join(file_name);
        &origin_path_buf
    } else {
        origin_input
    };

    if !base_target_path.exists() {
        if cli.tree || cli.hard {
            anyhow::bail!(
                "Target does not exist; cannot create {}",
                if cli.tree { "tree" } else { "hardlink" }
            );
        } else if !cli.allow_dangling {
            anyhow::bail!(
                "Target does not exist; refusing to create dangling symlink without --allow-dangling"
            );
        }
    }

    if cli.hard && is_cross_device(origin_path, &base_target_path)? {
        anyhow::bail!(
            "Cannot hardlink across filesystems: {} and {} are on different devices",
            origin_path.display(),
            base_target_path.display(),
        );
    }

    let existing_origin_err = |existing_file_type: &str| -> anyhow::Error {
        anyhow::anyhow!(
            "Refusing to overwrite existing {} at origin: {}",
            existing_file_type,
            origin_path.display()
        )
    };

    let log_remove_origin = || {
        println!(
            "{}: {}",
            "Remove existing file at origin".bold().red(),
            origin_path.display()
        );
    };

    if origin_path.exists() {
        if origin_path.is_dir() {
            return Err(existing_origin_err("directory"));
        }

        if cli.dry_run {
            if cli.force {
                if cli.verbose {
                    log_remove_origin();
                }
            } else {
                // we aren't going to be attempting to create the link, so we won't get the real
                // AlreadyExists error. just bail early.
                return Err(existing_origin_err("file"));
            }
        }
    }

    struct LogRecord {
        label: &'static str,
        origin_path: String,
        target_path: String,
    }

    impl LogRecord {
        fn emit(&self) {
            log_link_with_prefix(
                Some(self.label.bold()),
                &self.origin_path,
                &self.target_path,
            );
        }
    }

    let attempt_create_link = || -> anyhow::Result<LogRecord> {
        let (label, target_path) = if cli.tree && cli.hard {
            if !cli.dry_run {
                create_hard_link_tree(&base_target_path, origin_path)?;
            }
            ("create hardlink tree", raw_target_string.clone())
        } else if cli.tree {
            if !cli.dry_run {
                create_symlink_tree(&base_target_path, origin_path)?;
            }
            ("create symlink tree", raw_target_string.clone())
        } else if cli.hard {
            if !cli.dry_run {
                create_hard_link(&base_target_path, origin_path)?;
            }
            ("create hardlink", raw_target_string.clone())
        } else {
            // transform target string for --relative and --absolute if necessary
            let symlink_target = if cli.absolute {
                fs::canonicalize(&base_target_path)?
            } else if cli.relative {
                let abs_target = fs::canonicalize(&base_target_path)?;
                let origin_parent = origin_path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let abs_origin_parent = fs::canonicalize(origin_parent)?;
                pathdiff::diff_paths(&abs_target, &abs_origin_parent)
                    .context("Failed to calculate relative path")?
            } else {
                base_target_path.clone()
            };
            if !cli.dry_run {
                unix::fs::symlink(&symlink_target, origin_path)?;
            }
            ("create symlink", symlink_target.display().to_string())
        };
        let origin_path = origin_path.display().to_string();
        Ok(LogRecord {
            label,
            origin_path,
            target_path,
        })
    };

    fn is_already_exists(e: &anyhow::Error) -> bool {
        e.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists)
    }

    let record = attempt_create_link().or_else(|e| {
        if is_already_exists(&e) {
            if cli.force {
                if !cli.dry_run {
                    // should be impossible for dry_run to be true here, but check again just in
                    // case
                    fs::remove_file(origin_path)?;
                }
                if cli.verbose {
                    log_remove_origin();
                }
                // try again!
                attempt_create_link()
            } else {
                Err(existing_origin_err("file"))
            }
        } else {
            Err(e)
        }
    })?;

    if cli.verbose {
        record.emit();
    }

    Ok(())
}
