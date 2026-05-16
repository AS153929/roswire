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
    let _cli = Cli::parse();
    Ok(())
}
