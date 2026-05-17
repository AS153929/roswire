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

    /// Enable remote capability/schema probing for introspection commands.
    #[arg(long)]
    pub remote: bool,

    /// Build a plan without connecting to or modifying RouterOS.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Include read-only remote RouterOS diagnostics for doctor.
    #[arg(long = "include-remote")]
    pub include_remote: bool,

    /// Read a secret value from stdin for secret set commands.
    #[arg(long)]
    pub stdin: bool,

    /// Expected RouterOS SSH host key fingerprint for transfer workflows.
    #[arg(long = "ssh-host-key")]
    pub ssh_host_key: Option<String>,

    /// RouterOS SSH service port for transfer workflows.
    #[arg(long = "ssh-port")]
    pub ssh_port: Option<u16>,

    /// SSH username for transfer workflows; defaults to RouterOS API user.
    #[arg(long = "ssh-user")]
    pub ssh_user: Option<String>,

    /// SSH password for transfer workflows; defaults to RouterOS API password.
    #[arg(long = "ssh-password")]
    pub ssh_password: Option<String>,

    /// SSH private key path for transfer workflows.
    #[arg(long = "ssh-key")]
    pub ssh_key: Option<String>,

    /// Allow-list CIDR for SSH access during transfer workflows.
    #[arg(long = "allow-from")]
    pub allow_from: Vec<String>,

    /// Permit the plan to ensure RouterOS SSH service state.
    #[arg(long = "ensure-ssh")]
    pub ensure_ssh: bool,

    /// Restore RouterOS SSH service state after transfer workflows.
    #[arg(long = "restore-ssh")]
    pub restore_ssh: bool,

    /// Remove temporary remote files after transfer workflows.
    #[arg(long)]
    pub cleanup: bool,

    /// Remote path override for import/upload workflows.
    #[arg(long = "remote-path")]
    pub remote_path: Option<String>,

    /// RouterOS-generated backup/export base name.
    #[arg(long)]
    pub name: Option<String>,

    /// Request compact RouterOS export output.
    #[arg(long)]
    pub compact: bool,

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
        assert!(!cli.remote);
        assert!(!cli.dry_run);
        assert!(!cli.include_remote);
        assert!(!cli.stdin);
    }

    #[test]
    fn supports_secret_stdin_flag() {
        let cli = Cli::try_parse_from([
            "roswire",
            "secret",
            "set",
            "studio",
            "password",
            "type=plain",
            "--stdin",
        ])
        .expect("stdin flag should parse");

        assert!(cli.stdin);
        assert_eq!(
            cli.tokens,
            vec!["secret", "set", "studio", "password", "type=plain"]
        );
    }

    #[test]
    fn supports_transfer_dry_run_flags() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "upload",
            "setup.rsc",
            "flash/setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--ssh-port",
            "2222",
            "--ssh-user",
            "backup",
            "--ssh-password",
            "transfer-value",
            "--ssh-key",
            "/Users/example/.ssh/id_ed25519",
            "--allow-from",
            "203.0.113.10/32",
            "--ensure-ssh",
            "--restore-ssh",
            "--cleanup",
        ])
        .expect("transfer flags should parse");

        assert!(cli.dry_run);
        assert_eq!(cli.ssh_host_key.as_deref(), Some("SHA256:test"));
        assert_eq!(cli.ssh_port, Some(2222));
        assert_eq!(cli.ssh_user.as_deref(), Some("backup"));
        assert_eq!(cli.ssh_password.as_deref(), Some("transfer-value"));
        assert_eq!(
            cli.ssh_key.as_deref(),
            Some("/Users/example/.ssh/id_ed25519")
        );
        assert_eq!(cli.allow_from, vec!["203.0.113.10/32"]);
        assert!(cli.ensure_ssh);
        assert!(cli.restore_ssh);
        assert!(cli.cleanup);
        assert_eq!(
            cli.tokens,
            vec!["file", "upload", "setup.rsc", "flash/setup.rsc"]
        );
    }
}
