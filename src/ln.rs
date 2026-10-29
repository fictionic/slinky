use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::{
    cli::SlinkyLnCli,
    fs::{
        canonicalize_nofollow, create_hard_link, create_hard_link_tree, create_symlink,
        create_symlink_tree, dereference_symlink, is_cross_device,
    },
    logging::log_link,
    path::{diff_paths, dirpath},
    tidy::tidied_absolute_path,
};

pub enum SymlinkForm {
    Relative,
    Absolute,
}

#[derive(strum::Display)]
pub enum LnLinkType {
    #[strum(serialize = "symlink")]
    Symlink(SymlinkForm),
    #[strum(serialize = "symlink tree")]
    SymlinkTree(SymlinkForm),
    // literal symlinks get their own value, for easier polymorphism
    #[strum(serialize = "symlink")]
    SymlinkLiteral,
    #[strum(serialize = "hardlink")]
    Hardlink,
    #[strum(serialize = "hardlink tree")]
    HardlinkTree,
}

impl LnLinkType {
    pub fn new(cli: &SlinkyLnCli) -> Self {
        let is_relative = cli.relative || (!cli.absolute && cli.target.is_relative());
        let symlink_form = match is_relative {
            true => SymlinkForm::Relative,
            false => SymlinkForm::Absolute,
        };
        let &SlinkyLnCli {
            hard,
            tree,
            literal,
            ..
        } = cli;
        match (hard, tree, literal) {
            (false, false, false) => Self::Symlink(symlink_form),
            (false, true, _) => Self::SymlinkTree(symlink_form),
            (false, false, true) => Self::SymlinkLiteral,
            (true, false, _) => Self::Hardlink,
            (true, true, _) => Self::HardlinkTree,
        }
    }

    fn is_hard(&self) -> bool {
        matches!(self, Self::Hardlink | Self::HardlinkTree)
    }

    fn is_tree(&self) -> bool {
        matches!(self, Self::SymlinkTree(_) | Self::HardlinkTree)
    }

    fn is_literal_symlink(&self) -> bool {
        matches!(self, Self::SymlinkLiteral)
    }

    fn is_relative_symlink(&self) -> bool {
        matches!(
            self,
            Self::Symlink(SymlinkForm::Relative) | Self::SymlinkTree(SymlinkForm::Relative)
        )
    }

    fn effective_target_path(
        &self,
        target_path_resolved: &Path,
        origin_dirpath_canonical: &Path,
    ) -> Result<PathBuf> {
        let target_path = if self.is_hard() {
            // we could pass the target path through as-is, but a canonical path is clearer in the
            // verbose log. like fs::hard_link, do not follow a final symlink.
            canonicalize_nofollow(target_path_resolved).context("Could not resolve target")?
        } else {
            // symlink targets can be anything. we might be writing an absolute path, a path relative to
            // the origin, or just an opaque string.
            if self.is_literal_symlink() {
                // the user wants to write exactly the string they gave us
                target_path_resolved.to_path_buf()
            } else {
                self.effective_symlink_target_path(target_path_resolved, origin_dirpath_canonical)?
            }
        };
        Ok(target_path)
    }

    // convert the provided target path into a relative/absolute path that resolves as
    // intended from the provided origin path
    fn effective_symlink_target_path(
        &self,
        target_path: &Path,
        origin_dirpath_canonical: &Path,
    ) -> Result<PathBuf> {
        // even if we need to return a relative path, we'll need to convert the given target path to an absolute path, because
        // the relative rebasing requires a pathdiff against two absolute paths.
        let target_path_abs = tidied_absolute_path(target_path)?; // this fails only when cwd is inaccessible.
        let effective_target_path = if self.is_relative_symlink() {
            // convert the target into a relative path that resolves to `target_path` when evaluated from `origin_dirpath`
            diff_paths(&target_path_abs, origin_dirpath_canonical)
                .expect("base dir must be canonical")
        } else {
            target_path_abs
        };
        Ok(effective_target_path)
    }

    fn executor(&self) -> fn(target_path: &Path, origin_path: &Path) -> Result<()> {
        match self {
            Self::Symlink(_) | Self::SymlinkLiteral => create_symlink,
            Self::SymlinkTree(_) => create_symlink_tree,
            Self::Hardlink => create_hard_link,
            Self::HardlinkTree => create_hard_link_tree,
        }
    }
}

pub struct SlinkyLinker {
    link_type: LnLinkType,
    dereference: bool,
    force: bool,
    verbose: bool,
    dry_run: bool,
}

impl SlinkyLinker {
    pub fn new(cli: &SlinkyLnCli) -> Self {
        let &SlinkyLnCli {
            dereference,
            force,
            verbose,
            dry_run,
            ..
        } = cli;
        SlinkyLinker {
            link_type: LnLinkType::new(cli),
            dereference,
            force,
            verbose,
            dry_run,
        }
    }

    pub fn link(&self, target: &Path, origin: &Path) -> Result<()> {
        // dereference target path if necessary
        let target_resolved = if self.dereference {
            dereference_symlink(target)?
        } else {
            target.to_path_buf()
        };

        self.validate_target_exists(&target_resolved)?;

        // compute the effective dirname and basename of the origin based on what the user provided.
        let origin_path_data = OriginPathData::from(
            origin,
            &target_resolved,
            self.link_type.is_literal_symlink(),
        )?;
        let OriginPathData {
            path: effective_origin_path,
            dirpath_canonical: origin_dirpath_canonical,
        } = origin_path_data;

        self.validate_same_device(&target_resolved, &origin_dirpath_canonical)?;

        // compute the target path to use in creating the link. for hardlinks this doesn't matter,
        // as we're only selecting an inode, but for symlinks it affects what's written into the
        // link
        let effective_target_path = self
            .link_type
            .effective_target_path(&target_resolved, &origin_dirpath_canonical)?;

        // get the link-creating function based on the link type
        let create_link = self.link_type.executor();

        // create the link, respecting --force and --dry-run
        self.run(create_link, &effective_target_path, &effective_origin_path)?;

        // log
        if self.verbose {
            let link_type_str = self.link_type.to_string();
            log_link(
                Some(format!("create {link_type_str}").bold()),
                effective_origin_path,
                effective_target_path,
            );
        }
        Ok(())
    }

    fn validate_target_exists(&self, target: &Path) -> Result<()> {
        if !target.exists() {
            if self.link_type.is_hard() || self.link_type.is_tree() {
                anyhow::bail!("Target does not exist; cannot create {}", self.link_type);
            } else if !self.link_type.is_literal_symlink() {
                anyhow::bail!(
                    "Target does not exist; refusing to create dangling symlink without --literal"
                );
            }
        }
        Ok(())
    }

    fn validate_same_device(&self, target: &Path, origin: &Path) -> Result<()> {
        if self.link_type.is_hard() && is_cross_device(origin, target)? {
            anyhow::bail!(
                "Cannot hardlink across filesystems: {} and {} are on different devices",
                origin.display(),
                target.display(),
            );
        }
        Ok(())
    }

    fn run(
        &self,
        create_link: fn(target: &Path, origin: &Path) -> Result<()>,
        target: &Path,
        origin: &Path,
    ) -> Result<()> {
        let existing_origin_err = |existing_file_type: &str| -> anyhow::Error {
            anyhow::anyhow!(
                "Refusing to overwrite existing {} at origin: {}",
                existing_file_type,
                origin.display()
            )
        };

        let log_remove_origin = || {
            println!(
                "{}: {}",
                "Remove existing file at origin".bold().red(),
                origin.display()
            );
        };

        let attempt = || -> Result<()> {
            if origin.exists() {
                if origin.is_dir() {
                    return Err(existing_origin_err("directory"));
                }

                if self.dry_run {
                    if self.force {
                        if self.verbose {
                            log_remove_origin();
                        }
                    } else {
                        // we aren't going to be attempting to create the link, so we won't get the real
                        // AlreadyExists error. just bail early.
                        return Err(existing_origin_err("file"));
                    }
                }
            }

            if !self.dry_run {
                create_link(target, origin)?;
            }
            Ok(())
        };

        attempt().or_else(|e| {
            if is_already_exists(&e) {
                if self.force {
                    if !self.dry_run {
                        // should be impossible for dry_run to be true here, but check again just in
                        // case
                        fs::remove_file(origin)?;
                    }
                    if self.verbose {
                        log_remove_origin();
                    }
                    // try again!
                    attempt()
                } else {
                    Err(existing_origin_err("file"))
                }
            } else {
                Err(e)
            }
        })?;

        Ok(())
    }
}

fn is_already_exists(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists)
}

struct OriginPathData {
    // the path, perhaps relative to cwd, that will resolve to the link
    path: PathBuf,
    // the canonical path to the directory that will contain the link
    dirpath_canonical: PathBuf,
}

impl OriginPathData {
    pub fn from(
        origin_arg: &Path,
        target_arg_resolved: &Path, // resolved according to -L
        literal: bool,
    ) -> Result<Self> {
        // we need to know what directory the link will live in, and what the basename of the link should be.
        let (path, dirpath) = if origin_arg.is_dir() {
            // the origin path from the user might be a directory.
            // in this case, we infer the origin basename from the target basename if possible.
            let basename = target_arg_resolved
                .file_name()
                .map(OsStr::to_os_string)
                .or_else(|| {
                    // the target path provided by the user ends in '..', or is '.' or '/'.
                    // canonicalize it so we can grab a proper basename if possible.
                    let target_path_resolvable = if literal {
                        // in literal mode, the target is evaluated from the perspective of the origin
                        origin_arg.join(target_arg_resolved)
                    } else {
                        target_arg_resolved.to_path_buf()
                    };
                    target_path_resolvable
                        .canonicalize()
                        .ok()?
                        .file_name()
                        .map(OsStr::to_os_string)
                })
                // this happens only when the user provided the root path '/' as the origin, or when
                // the provided target path cannot be canonicalized
                .context("Could not infer basename from provided target path")?;
            (origin_arg.join(basename), origin_arg.to_path_buf())
        } else {
            // the origin path from the user either does not resolve to anything, or it resolves to
            // a non-directory file. check to see if the parent path resolves to a directory.
            let origin_parent = dirpath(origin_arg);
            if !origin_parent.is_dir() {
                // the user gave us a path that ends in '.../<nonexistent>/<nonexistent>'. can't help em.
                anyhow::bail!("Origin directory does not exist");
            }
            (origin_arg.to_path_buf(), origin_parent.to_path_buf())
        };
        let dirpath_canonical = dirpath
            .canonicalize()
            .context("Cannot resolve origin directory")?;
        // assemble the pieces
        Ok(Self {
            path,
            dirpath_canonical,
        })
    }
}
