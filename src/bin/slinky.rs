use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use slinky::cli::{SlinkyCli, SlinkyCommand};
use slinky::cmd::RunSlinkyCommand;
use slinky::walk::{SlinkyCtx, SymlinkIter};

fn main() -> Result<ExitCode> {
    let cli = SlinkyCli::parse();

    if !cli.path.exists() {
        anyhow::bail!("{}: No such file or directory", cli.path.display());
    }

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
