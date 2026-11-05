use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use slinky::cli::{SlinkyCli, SlinkyCommand};
use slinky::cmd::RunSlinkyCommand;
use slinky::logging;
use slinky::walk::{SlinkyCtx, SymlinkIter};

fn main() -> ExitCode {
    logging::init(env!("CARGO_BIN_NAME"));

    let cli = SlinkyCli::parse();

    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            logging::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: SlinkyCli) -> Result<ExitCode> {
    // check that PATH points at an existing directory entry. intentionally letting errors propagate
    // because we want the standard error messages from libc.
    // using std::fs instead of fs_err because we don't want "failed to query raw metadata" in the
    // error message, so we have to inclue the path in context ourselves.
    std::fs::symlink_metadata(&cli.path)
        .with_context(|| format!("cannot access {}", cli.path.display()))?;

    let iter = SymlinkIter::new(&cli)?;

    let ctx = SlinkyCtx::new(&cli, iter);

    match cli.command {
        SlinkyCommand::List(cmd) => cmd.run(&ctx),
        SlinkyCommand::TidyTarget(cmd) => cmd.run(&ctx),
        SlinkyCommand::Canonicalize(cmd) => cmd.run(&ctx),
        SlinkyCommand::ToRelative(cmd) => cmd.run(&ctx),
        SlinkyCommand::ToAbsolute(cmd) => cmd.run(&ctx),
        SlinkyCommand::EditTarget(cmd) => cmd.run(&ctx),
        SlinkyCommand::ToHardlink(cmd) => cmd.run(&ctx),
        SlinkyCommand::ToTree(cmd) => cmd.run(&ctx),
        SlinkyCommand::ReplaceWithTarget(cmd) => cmd.run(&ctx),
        SlinkyCommand::Remove(cmd) => cmd.run(&ctx),
        SlinkyCommand::Exec(cmd) => cmd.run(&ctx),
    }?;

    // per-link errors were already reported as they happened
    Ok(if ctx.failed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
