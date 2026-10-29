use anyhow::Result;
use clap::Parser;
use slinky::cli::SlinkyLnCli;
use slinky::ln::SlinkyLinker;

fn main() -> Result<()> {
    let cli = SlinkyLnCli::parse();

    let linker = SlinkyLinker::new(&cli);

    linker.link(&cli.target, &cli.origin)?;

    Ok(())
}
