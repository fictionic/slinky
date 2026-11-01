use colored::*;
use std::path::Path;

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

// "msg [origin -> target]", with msg already colored by severity. without a
// target, just "msg: origin"
fn fmt_link_msg(msg: ColoredString, origin_path: &Path, target_path: Option<&Path>) -> String {
    let link = match target_path {
        Some(t) => fmt_link(origin_path, t),
        None => origin_path.display().to_string().cyan().to_string(),
    };
    format!("{} [{}]", msg, link)
}

// prints "Warning: msg [origin -> target]" to stderr.
// if there is a SlinkyCtx, use SlinkyCtx::warn_on_link, which adds the command name.
pub fn warn_on_link(msg: &str, origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) {
    eprintln!(
        "Warning: {}",
        fmt_link_msg(
            msg.yellow(),
            origin_path.as_ref(),
            Some(target_path.as_ref())
        )
    );
}

pub fn warn(msg: &str) {
    eprintln!("Warning: {}", msg.yellow());
}

// per-command logging. each method adds the command name, and the log_*
// methods print only under --verbose
impl SlinkyCtx {
    /// Prints "cmd: origin -> target" on stdout, if verbose.
    pub fn log_link(&self, origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) {
        if self.verbose {
            log_link(Some(self.cmd_name.bold()), origin_path, target_path);
        }
    }

    /// Prints "cmd: origin -> (old => new)" on stdout, if verbose.
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

    /// Prints "cmd: Warning: msg [origin -> target]" on stderr, unconditional of verbosity.
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
                format!("Warning: {}", msg).yellow(),
                origin_path.as_ref(),
                Some(target_path.as_ref())
            )
        );
    }

    /// Prints "cmd: Error: msg [origin -> target]" on stderr, or "cmd: Error: msg [origin]" if there
    /// is no target, unconditional of verbosity.
    ///
    /// Also marks the run as failed, so the process exits nonzero once every link has been tried.
    pub fn error_on_link(&self, msg: &str, origin_path: &Path, target_path: Option<&Path>) {
        self.failed.set(true);
        eprintln!(
            "{}: {}",
            self.cmd_name.bold(),
            fmt_link_msg(format!("Error: {msg}").red(), origin_path, target_path)
        );
    }
}
