use colored::*;
use std::path::Path;

use anyhow::Result;
use colored::ColoredString;

use crate::walk::{SlinkyCtx, Symlink, SymlinkIter};

// the colored "origin -> target" text that ends every link log line
pub fn fmt_link(origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) -> String {
    format!(
        "{} -> {}",
        origin_path.as_ref().display().to_string().cyan(),
        target_path.as_ref().display().to_string().magenta()
    )
}

// "msg: origin -> target", with msg already colored by severity. without a
// target, just "msg: origin"
fn fmt_link_msg(msg: ColoredString, origin_path: &Path, target_path: Option<&Path>) -> String {
    let link = match target_path {
        Some(t) => fmt_link(origin_path, t),
        None => origin_path.display().to_string().cyan().to_string(),
    };
    format!("{}: {}", msg, link)
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
    eprintln!(
        "{}",
        fmt_link_msg(
            msg.yellow(),
            origin_path.as_ref(),
            Some(target_path.as_ref())
        )
    );
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
            fmt_link_msg(
                msg.yellow(),
                origin_path.as_ref(),
                Some(target_path.as_ref())
            )
        );
    }

    // "cmd: Error: msg: origin -> target" on stderr, or "cmd: Error: msg: origin"
    // without a target. also marks the run as failed, so the process exits
    // nonzero once every link has been tried
    pub fn error_on_link(&self, msg: &str, origin_path: &Path, target_path: Option<&Path>) {
        self.failed.set(true);
        eprintln!(
            "{}: {}",
            self.cmd_name.bold(),
            fmt_link_msg(format!("Error: {msg}").red(), origin_path, target_path)
        );
    }

    // runs op on each link the walk yields. an error, from the walk or from
    // op, is reported with the path it came from, and the loop moves on to
    // the next item
    pub fn for_each_link<F>(&self, iter: SymlinkIter, mut op: F)
    where
        F: FnMut(&Symlink) -> Result<()>,
    {
        for item in iter {
            match item {
                Ok(link) => {
                    if let Err(e) = op(&link) {
                        self.error_on_link(
                            &format!("{e:#}"),
                            &link.origin_path,
                            Some(&link.target_path),
                        );
                    }
                }
                Err(e) => self.error_on_link(
                    &e.source.to_string(),
                    &e.origin_path,
                    e.target_path.as_deref(),
                ),
            }
        }
    }
}
