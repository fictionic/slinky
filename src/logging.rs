use colored::*;
use std::path::Path;
use std::sync::OnceLock;

use colored::ColoredString;

use crate::walk::{SlinkyCtx, Symlink};

static PROGRAM_NAME: OnceLock<&'static str> = OnceLock::new();

pub fn init(program_name: &'static str) {
    let _ = PROGRAM_NAME.set(program_name);
}

fn program_name() -> &'static str {
    // fallback should only happen in unit tests
    PROGRAM_NAME.get().copied().unwrap_or("slinky")
}

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

// prints "program: error: msg" to stderr
pub fn error(msg: &str) {
    eprintln!("{}: {}", program_name(), format!("error: {msg}").red());
}

// prints "program: warning: msg" to stderr
pub fn warn(msg: &str) {
    eprintln!("{}: {}", program_name(), fmt_warn(msg).yellow());
}

// prints "program: warning: msg [origin -> target]" to stderr
pub fn warn_on_link(msg: &str, origin_path: impl AsRef<Path>, target_path: impl AsRef<Path>) {
    eprintln!(
        "{}: {}",
        program_name(),
        fmt_link_msg(
            fmt_warn(msg).yellow(),
            origin_path.as_ref(),
            Some(target_path.as_ref())
        )
    );
}

fn fmt_warn(msg: &str) -> String {
    format!("warning: {msg}")
}

// per-command logging. the log_* methods add the command name to stdout lines, and print only
// under --verbose
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

    /// Prints "program: warning: msg [origin -> target]" on stderr, unconditional of verbosity.
    pub fn warn_on_link(
        &self,
        msg: &str,
        origin_path: impl AsRef<Path>,
        target_path: impl AsRef<Path>,
    ) {
        warn_on_link(msg, origin_path, target_path);
    }

    /// Prints "program: error: msg [origin -> target]" on stderr, or "program: error: msg [origin]"
    /// if there is no target, unconditional of verbosity.
    ///
    /// Also marks the run as failed, so the process exits nonzero once every link has been tried.
    pub fn error_on_link(&self, msg: &str, origin_path: &Path, target_path: Option<&Path>) {
        self.failed.set(true);
        eprintln!(
            "{}: {}",
            program_name(),
            fmt_link_msg(format!("error: {msg}").red(), origin_path, target_path)
        );
    }
}
