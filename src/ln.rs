use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use colored::Colorize;
use fs_err as fs;

use crate::{
    cli::SlinkyLnCli,
    fs::{
        canonicalize_nofollow, create_hard_link, create_hard_link_tree, create_symlink,
        create_symlink_tree, dereference_symlink, is_cross_device, metadata_nofollow,
        replace_file_with,
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
            canonicalize_nofollow(target_path_resolved).context("could not resolve target")?
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
                anyhow::bail!("target does not exist; cannot create {}", self.link_type);
            } else if !self.link_type.is_literal_symlink() {
                anyhow::bail!(
                    "target does not exist; refusing to create dangling symlink without --literal"
                );
            }
        }
        Ok(())
    }

    fn validate_same_device(&self, target: &Path, origin: &Path) -> Result<()> {
        if self.link_type.is_hard() && is_cross_device(origin, target)? {
            anyhow::bail!(
                "cannot hardlink across filesystems: {} and {} are on different devices",
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
                "refusing to overwrite existing {} at origin: {}",
                existing_file_type,
                origin.display()
            )
        };

        let origin_meta = metadata_nofollow(origin)?;

        match origin_meta {
            None => {
                if !self.dry_run {
                    create_link(target, origin)?;
                }
            }
            Some(m) if m.is_dir() => {
                // never delete an existing directory, even with --force
                return Err(existing_origin_err("directory"));
            }
            Some(_) if !self.force => {
                return Err(existing_origin_err("file"));
            }
            Some(_) => {
                if self.verbose {
                    println!(
                        "{}: {}",
                        "replacing existing file at origin".bold().red(),
                        origin.display()
                    );
                }
                if !self.dry_run {
                    replace_file_with(origin, |temp_path| create_link(target, temp_path))?;
                }
            }
        }

        Ok(())
    }
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
                    fs::canonicalize(target_path_resolvable)
                        .ok()?
                        .file_name()
                        .map(OsStr::to_os_string)
                })
                // this happens only when the user provided the root path '/' as the origin, or when
                // the provided target path cannot be canonicalized
                .context("could not infer basename from provided target path")?;
            (origin_arg.join(basename), origin_arg.to_path_buf())
        } else {
            // the origin path from the user either does not resolve to anything, or it resolves to
            // a non-directory file. check to see if the parent path resolves to a directory.
            let origin_parent = dirpath(origin_arg);
            if !origin_parent.is_dir() {
                // the user gave us a path that ends in '.../<nonexistent>/<nonexistent>'. can't help em.
                anyhow::bail!("origin directory does not exist");
            }
            (origin_arg.to_path_buf(), origin_parent.to_path_buf())
        };
        let dirpath_canonical =
            fs::canonicalize(&dirpath).context("cannot resolve origin directory")?;
        // assemble the pieces
        Ok(Self {
            path,
            dirpath_canonical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::io::Write;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn link_type(args: &[&str]) -> LnLinkType {
        let cli = SlinkyLnCli::parse_from(std::iter::once("slinky-ln").chain(args.iter().copied()));
        LnLinkType::new(&cli)
    }

    // LnLinkType::new

    #[test]
    fn link_type_keeps_form_of_relative_target() {
        assert!(matches!(
            link_type(&["t"]),
            LnLinkType::Symlink(SymlinkForm::Relative)
        ));
    }

    #[test]
    fn link_type_keeps_form_of_absolute_target() {
        assert!(matches!(
            link_type(&["/t"]),
            LnLinkType::Symlink(SymlinkForm::Absolute)
        ));
    }

    #[test]
    fn link_type_absolute_flag_overrides_relative_target() {
        assert!(matches!(
            link_type(&["-b", "t"]),
            LnLinkType::Symlink(SymlinkForm::Absolute)
        ));
    }

    #[test]
    fn link_type_relative_flag_overrides_absolute_target() {
        assert!(matches!(
            link_type(&["-r", "/t"]),
            LnLinkType::Symlink(SymlinkForm::Relative)
        ));
    }

    #[test]
    fn link_type_literal() {
        assert!(matches!(
            link_type(&["--literal", "t"]),
            LnLinkType::SymlinkLiteral
        ));
    }

    #[test]
    fn link_type_tree_keeps_symlink_form() {
        assert!(matches!(
            link_type(&["-T", "t"]),
            LnLinkType::SymlinkTree(SymlinkForm::Relative)
        ));
        assert!(matches!(
            link_type(&["-T", "-b", "t"]),
            LnLinkType::SymlinkTree(SymlinkForm::Absolute)
        ));
    }

    #[test]
    fn link_type_hard() {
        assert!(matches!(link_type(&["-H", "t"]), LnLinkType::Hardlink));
        assert!(matches!(
            link_type(&["-H", "-T", "t"]),
            LnLinkType::HardlinkTree
        ));
    }

    // SlinkyLinker::run

    // run does not read the link type, so any type does
    fn linker(force: bool, dry_run: bool) -> SlinkyLinker {
        SlinkyLinker {
            link_type: LnLinkType::SymlinkLiteral,
            dereference: false,
            force,
            verbose: false,
            dry_run,
        }
    }

    // a fake create_link. like the real ones, it fails with AlreadyExists if the origin exists
    fn create_file(_target: &Path, origin: &Path) -> Result<()> {
        std::fs::File::create_new(origin)?.write_all(b"new")?;
        Ok(())
    }

    fn create_fails(_target: &Path, _origin: &Path) -> Result<()> {
        anyhow::bail!("create failed")
    }

    fn entry_count(dir: &Path) -> usize {
        fs::read_dir(dir).unwrap().count()
    }

    #[test]
    fn run_creates_link_when_origin_is_missing() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        linker(false, false)
            .run(create_file, Path::new("t"), &origin)
            .unwrap();
        assert_eq!(fs::read_to_string(&origin).unwrap(), "new");
    }

    #[test]
    fn run_refuses_existing_file_without_force() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::write(&origin, "old").unwrap();
        let err = linker(false, false)
            .run(create_file, Path::new("t"), &origin)
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("refusing to overwrite existing file")
        );
        assert_eq!(fs::read_to_string(&origin).unwrap(), "old");
    }

    #[test]
    fn run_refuses_broken_symlink_without_force() {
        // lstat sees the link, though nothing is at its target
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        symlink("missing", &origin).unwrap();
        assert!(
            linker(false, false)
                .run(create_file, Path::new("t"), &origin)
                .is_err()
        );
        assert_eq!(fs::read_link(&origin).unwrap(), Path::new("missing"));
    }

    #[test]
    fn run_force_replaces_existing_file() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::write(&origin, "old").unwrap();
        linker(true, false)
            .run(create_file, Path::new("t"), &origin)
            .unwrap();
        assert_eq!(fs::read_to_string(&origin).unwrap(), "new");
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[test]
    fn run_force_replaces_symlink_to_directory() {
        // only the link is replaced, as with `ln -sfn`. the directory stays
        let dir = tempdir().unwrap();
        let real_dir = dir.path().join("dir");
        fs::create_dir(&real_dir).unwrap();
        fs::write(real_dir.join("f"), "keep").unwrap();
        let origin = dir.path().join("origin");
        symlink("dir", &origin).unwrap();
        linker(true, false)
            .run(create_file, Path::new("t"), &origin)
            .unwrap();
        assert_eq!(fs::read_to_string(&origin).unwrap(), "new");
        assert_eq!(fs::read_to_string(real_dir.join("f")).unwrap(), "keep");
    }

    #[test]
    fn run_refuses_directory_even_with_force() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::create_dir(&origin).unwrap();
        fs::write(origin.join("f"), "keep").unwrap();
        let err = linker(true, false)
            .run(create_file, Path::new("t"), &origin)
            .unwrap_err();
        assert!(err.to_string().contains("directory"));
        assert_eq!(fs::read_to_string(origin.join("f")).unwrap(), "keep");
    }

    #[test]
    fn run_force_create_failure_keeps_existing_file() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::write(&origin, "old").unwrap();
        let err = linker(true, false)
            .run(create_fails, Path::new("t"), &origin)
            .unwrap_err();
        assert!(format!("{err:#}").contains("create failed"));
        assert_eq!(fs::read_to_string(&origin).unwrap(), "old");
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[test]
    fn run_dry_run_creates_nothing() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        linker(false, true)
            .run(create_file, Path::new("t"), &origin)
            .unwrap();
        assert_eq!(entry_count(dir.path()), 0);
    }

    #[test]
    fn run_dry_run_refuses_existing_file_without_force() {
        // a dry run fails exactly when a real run would
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::write(&origin, "old").unwrap();
        assert!(
            linker(false, true)
                .run(create_file, Path::new("t"), &origin)
                .is_err()
        );
    }

    #[test]
    fn run_dry_run_force_keeps_existing_file() {
        let dir = tempdir().unwrap();
        let origin = dir.path().join("origin");
        fs::write(&origin, "old").unwrap();
        linker(true, true)
            .run(create_file, Path::new("t"), &origin)
            .unwrap();
        assert_eq!(fs::read_to_string(&origin).unwrap(), "old");
        assert_eq!(entry_count(dir.path()), 1);
    }
}
