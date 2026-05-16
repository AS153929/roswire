pub mod args;
pub mod config;
pub mod error;
pub mod introspect;
pub mod mapping;
pub mod protocol;
pub mod transfer;
pub mod workflow;

use args::Cli;
use clap::Parser;
use error::RosWireResult;

pub fn run() -> RosWireResult<()> {
    let cli = Cli::parse();

    if cli.simulate_error {
        return Err(Box::new(
            error::RosWireError::usage("simulated usage error for contract tests")
                .with_hint("remove --simulate-error to continue"),
        ));
    }

    if let Some(result) = config::handle(&cli.tokens, &cli) {
        let payload = result?;
        println!("{payload}");
        return Ok(());
    }

    if let Some(result) = introspect::handle(&cli.tokens, cli.remote) {
        let payload = result?;
        println!("{payload}");
        return Ok(());
    }

    let _invocation = args::parse_invocation(&cli.tokens)?;

    Ok(())
}
