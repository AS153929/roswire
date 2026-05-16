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
use error::{ErrorContext, RosWireResult};

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

    let invocation = args::parse_invocation(&cli.tokens)?;

    Err(Box::new(
        error::RosWireError::unsupported_action(format!(
            "RouterOS action is not implemented: {}",
            unsupported_command_name(&invocation),
        ))
        .with_context(unsupported_action_context(&cli, &invocation)),
    ))
}

fn unsupported_action_context(cli: &Cli, invocation: &args::ParsedInvocation) -> ErrorContext {
    ErrorContext {
        command: unsupported_command_name(invocation),
        path: invocation.path.clone(),
        action: invocation.action.clone(),
        requested_protocol: cli
            .protocol
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| "auto".to_owned()),
        selected_protocol: "unknown".to_owned(),
        transfer_backend: cli.transfer.map(|value| value.as_str().to_owned()),
        routeros_version: cli
            .routeros_version
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| "auto".to_owned()),
        host: cli
            .host
            .clone()
            .or_else(|| std::env::var("ROS_HOST").ok())
            .unwrap_or_default(),
        resolved_args: error::redact_resolved_args(&invocation.resolved_args),
    }
}

fn unsupported_command_name(invocation: &args::ParsedInvocation) -> String {
    invocation
        .path
        .iter()
        .chain(std::iter::once(&invocation.action))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("/")
}
