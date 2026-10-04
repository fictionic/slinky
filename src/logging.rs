use colored::*;
use std::path::Path;

use anyhow::Result;
use colored::ColoredString;

use crate::walk::{SlinkyCtx, Symlink};

// the colored "origin -> target" text that ends every link log line
pub fn fmt_link(origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) -> String {
    format!(
        "{} -> {}",
        origin_path.as_ref().display().to_string().cyan(),
        target_path.as_ref().display().to_string().magenta()
    )
}

// "msg: origin -> target", with msg already colored by severity
fn fmt_link_msg(
    msg: ColoredString,
    origin_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) -> String {
    format!("{}: {}", msg, fmt_link(origin_path, target_path))
}

// prints "prefix: origin -> target" to stdout
pub fn log_link(
    prefix: Option<ColoredString>,
    origin_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) {
    match prefix {
        Some(p) => println!("{}: {}", p, fmt_link(origin_path, target_path)),
        None => println!("{}", fmt_link(origin_path, target_path)),
    }
}

// prints "msg: origin -> target" to stderr.
// if there is a SlinkyCtx, use SlinkyCtx::warn_on_link, which adds the command name.
pub fn warn_on_link(msg: &str, origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) {
    eprintln!("{}", fmt_link_msg(msg.yellow(), origin_path, target_path));
}

// per-command logging. each method adds the command name, and the log_*
// methods print only under --verbose
impl SlinkyCtx {
    // "cmd: origin -> target"
    pub fn log_link(&self, origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) {
        if self.verbose {
            log_link(Some(self.cmd_name.bold()), origin_path, target_path);
        }
    }

    // "cmd: origin -> (old => new)"
    pub fn log_transformation(&self, link: &Symlink, new_target_path: impl AsRef<Path>) {
        if self.verbose {
            println!(
                "{}: {} -> ({} {} {})",
                self.cmd_name.bold(),
                link.origin_path.display().to_string().cyan(),
                link.target_path.display().to_string().dimmed(),
                "=>".bright_white(),
                new_target_path.as_ref().display().to_string().magenta()
            );
        }
    }

    // "cmd: msg: origin -> target" on stderr, verbose or not
    pub fn warn_on_link(
        &self,
        msg: &str,
        origin_path: impl AsRef<Path>,
        target_path: impl AsRef<Path>,
    ) {
        eprintln!(
            "{}: {}",
            self.cmd_name.bold(),
            fmt_link_msg(msg.yellow(), origin_path, target_path)
        );
    }

    // "cmd: Error: msg: origin -> target" on stderr. also marks the run as
    // failed, so the process exits nonzero once every link has been tried
    pub fn error_on_link(
        &self,
        msg: &str,
        origin_path: impl AsRef<Path>,
        target_path: impl AsRef<Path>,
    ) {
        self.failed.set(true);
        eprintln!(
            "{}: {}",
            self.cmd_name.bold(),
            fmt_link_msg(format!("Error: {msg}").red(), origin_path, target_path)
        );
    }

    // runs op for one link. an error is reported with the link it came from,
    // and does not stop the caller from moving on to the next link
    pub fn run_for_link<F>(&self, link: &Symlink, op: F)
    where
        F: FnOnce() -> Result<()>,
    {
        if let Err(e) = op() {
            self.error_on_link(&format!("{e:#}"), &link.origin_path, &link.target_path);
        }
    }
}
