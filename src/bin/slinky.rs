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

    let ctx = SlinkyCtx::new(&cli);

    match cli.command {
        SlinkyCommand::List(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::TidyTarget(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::Canonicalize(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::ToRelative(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::ToAbsolute(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::EditTarget(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::ToHardlink(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::ToTree(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::ReplaceWithTarget(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::Remove(cmd) => cmd.run(&ctx, iter),
        SlinkyCommand::Exec(cmd) => cmd.run(&ctx, iter),
    }?;

    // per-link errors were already reported as they happened
    Ok(if ctx.failed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
