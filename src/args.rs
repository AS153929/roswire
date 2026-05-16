use crate::error::{RosWireError, RosWireResult};
use clap::{Parser, ValueEnum};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProtocolMode {
    Auto,
    Api,
    ApiSsl,
    Rest,
}

impl ProtocolMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Api => "api",
            Self::ApiSsl => "api-ssl",
            Self::Rest => "rest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RouterOsVersionMode {
    Auto,
    V6,
    V7,
}

impl RouterOsVersionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::V6 => "v6",
            Self::V7 => "v7",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransferMode {
    Ssh,
}

impl TransferMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvocation {
    pub path: Vec<String>,
    pub action: String,
    pub resolved_args: BTreeMap<String, String>,
}

#[derive(Debug, Parser)]
#[command(
    name = "roswire",
    version,
    about = "JSON-first RouterOS CLI bridge for AI agents and automation.",
    long_about = None
)]
pub struct Cli {
    #[arg(long)]
    pub profile: Option<String>,

    #[arg(long)]
    pub host: Option<String>,

    #[arg(long)]
    pub user: Option<String>,

    #[arg(long)]
    pub password: Option<String>,

    #[arg(long, value_enum)]
    pub protocol: Option<ProtocolMode>,

    #[arg(long = "routeros-version", value_enum)]
    pub routeros_version: Option<RouterOsVersionMode>,

    #[arg(long, value_enum)]
    pub transfer: Option<TransferMode>,

    #[arg(long)]
    pub port: Option<u16>,

    /// Output machine-readable JSON.
    #[arg(long)]
    pub json: bool,

    /// Enable debug diagnostics on stderr.
    #[arg(long)]
    pub debug: bool,

    /// Internal test hook to exercise structured error output paths.
    #[arg(long, hide = true)]
    pub simulate_error: bool,

    /// Raw command tokens passed after global options.
    #[arg(value_name = "TOKEN")]
    pub tokens: Vec<String>,
}

pub fn parse_invocation(tokens: &[String]) -> RosWireResult<ParsedInvocation> {
    if tokens.is_empty() {
        return Err(Box::new(RosWireError::usage(
            "missing action: expected <path...> <action>",
        )));
    }

    let first_kv_index = tokens
        .iter()
        .position(|token| token.contains('='))
        .unwrap_or(tokens.len());
    let command_tokens = &tokens[..first_kv_index];

    if command_tokens.is_empty() {
        return Err(Box::new(RosWireError::usage(
            "missing action: expected <path...> <action>",
        )));
    }

    let action = command_tokens
        .last()
        .expect("command_tokens cannot be empty")
        .to_owned();
    let path = command_tokens[..command_tokens.len().saturating_sub(1)].to_vec();

    let mut resolved_args = BTreeMap::new();
    for token in &tokens[first_kv_index..] {
        let Some((key, value)) = token.split_once('=') else {
            return Err(Box::new(RosWireError::usage(format!(
                "argument after key=value section must also be key=value: {token}",
            ))));
        };

        if key.is_empty() {
            return Err(Box::new(RosWireError::usage(
                "argument key cannot be empty",
            )));
        }

        resolved_args.insert(key.to_owned(), value.to_owned());
    }

    Ok(ParsedInvocation {
        path,
        action,
        resolved_args,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_invocation, Cli, ProtocolMode};
    use clap::Parser;

    #[test]
    fn parses_print_path_and_action() {
        let cli =
            Cli::try_parse_from(["roswire", "ip", "address", "print"]).expect("args should parse");
        let invocation = parse_invocation(&cli.tokens).expect("invocation should parse");

        assert_eq!(invocation.path, vec!["ip", "address"]);
        assert_eq!(invocation.action, "print");
        assert!(invocation.resolved_args.is_empty());
    }

    #[test]
    fn parses_key_value_args_including_dot_id() {
        let cli = Cli::try_parse_from([
            "roswire",
            "ip",
            "address",
            "remove",
            ".id=*1",
            "comment=wan uplink",
        ])
        .expect("args should parse");

        let invocation = parse_invocation(&cli.tokens).expect("invocation should parse");

        assert_eq!(invocation.path, vec!["ip", "address"]);
        assert_eq!(invocation.action, "remove");
        assert_eq!(
            invocation.resolved_args.get(".id").map(String::as_str),
            Some("*1")
        );
        assert_eq!(
            invocation.resolved_args.get("comment").map(String::as_str),
            Some("wan uplink")
        );
    }

    #[test]
    fn missing_action_returns_usage_error() {
        let error = parse_invocation(&[]).expect_err("missing action should fail");
        assert_eq!(error.error_code, crate::error::ErrorCode::UsageError);
    }

    #[test]
    fn supports_protocol_value_enum() {
        let cli = Cli::try_parse_from(["roswire", "--protocol", "rest", "ip", "address", "print"])
            .expect("protocol enum should parse");

        assert_eq!(cli.protocol, Some(ProtocolMode::Rest));
    }
}
