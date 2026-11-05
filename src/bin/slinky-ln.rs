use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use slinky::cli::SlinkyLnCli;
use slinky::ln::SlinkyLinker;
use slinky::logging;

fn main() -> ExitCode {
    logging::init(env!("CARGO_BIN_NAME"));

    let cli = SlinkyLnCli::parse();

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            logging::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &SlinkyLnCli) -> Result<()> {
    let linker = SlinkyLinker::new(cli);

    linker.link(&cli.target, &cli.origin)?;

    Ok(())
}
