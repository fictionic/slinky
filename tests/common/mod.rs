use assert_cmd::assert::Assert;
use assert_cmd::prelude::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{tempdir, TempDir};

pub struct TestContext {
    temp_dir: TempDir,
}

impl TestContext {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            temp_dir: tempdir()?,
        })
    }

    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    #[allow(dead_code)]
    pub fn slinky_cmd(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
        cmd.current_dir(self.path());
        cmd
    }

    #[allow(dead_code)]
    pub fn slinky_ln_cmd(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky-ln"));
        cmd.current_dir(self.path());
        cmd
    }

    #[allow(dead_code)]
    pub fn run_slinky(&self, args: &[&str]) -> Assert {
        let mut cmd = self.slinky_cmd();
        cmd.args(args);
        cmd.assert()
    }

    #[allow(dead_code)]
    pub fn run_slinky_ln(&self, args: &[&str]) -> Assert {
        let mut cmd = self.slinky_ln_cmd();
        cmd.args(args);
        cmd.assert()
    }

    pub fn create_file(&self, name: &str, content: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let file_path = self.path().join(name);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, content)?;
        Ok(file_path)
    }

    pub fn create_symlink(&self, target: &str, link_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let link_path = self.path().join(link_name);
        if let Some(parent) = link_path.parent() {
            fs::create_dir_all(parent)?;
        }
        symlink(target, &link_path)?;
        Ok(link_path)
    }
}
