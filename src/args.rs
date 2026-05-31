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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransferIfExists {
    Overwrite,
    Skip,
    Fail,
}

impl TransferIfExists {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overwrite => "overwrite",
            Self::Skip => "skip",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvocation {
    pub path: Vec<String>,
    pub action: String,
    pub resolved_args: BTreeMap<String, String>,
    pub flags: Vec<String>,
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

    /// Request remote snapshot for introspection commands. Currently returns a degraded static snapshot; live device probing is not yet implemented.
    #[arg(long)]
    pub remote: bool,

    /// Mark the remote schema cache status as refresh. On-disk schema caching is not yet implemented, so this is currently a status marker only.
    #[arg(long)]
    pub refresh: bool,

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

    /// Expected RouterOS TLS certificate fingerprint (SHA256:<base64>) for api-ssl/rest.
    #[arg(long = "tls-cert-fingerprint")]
    pub tls_cert_fingerprint: Option<String>,

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

    /// Policy when transfer destination already exists.
    #[arg(long = "if-exists", value_enum, default_value_t = TransferIfExists::Overwrite)]
    pub if_exists: TransferIfExists,

    /// SSH/API connection timeout in seconds for transfer workflows.
    #[arg(long = "connect-timeout-seconds")]
    pub connect_timeout_seconds: Option<u64>,

    /// Timeout in seconds while waiting for RouterOS-generated workflow files.
    #[arg(long = "wait-timeout-seconds")]
    pub wait_timeout_seconds: Option<u64>,

    /// Socket read/write timeout in seconds for transfer data plane.
    #[arg(long = "transfer-timeout-seconds")]
    pub transfer_timeout_seconds: Option<u64>,

    /// Cleanup operation timeout in seconds for transfer workflows.
    #[arg(long = "cleanup-timeout-seconds")]
    pub cleanup_timeout_seconds: Option<u64>,

    /// Maximum retry count for retryable transfer workflow steps.
    #[arg(long, default_value_t = 0)]
    pub retries: u8,

    /// Delay in seconds between retry attempts.
    #[arg(long = "retry-delay-seconds", default_value_t = 0)]
    pub retry_delay_seconds: u64,

    /// Allow explicit raw RouterOS passthrough commands that may mutate state.
    #[arg(long = "allow-write")]
    pub allow_write: bool,

    /// Remove temporary remote files after transfer workflows.
    #[arg(long)]
    pub cleanup: bool,

    /// Remote path override for import/upload workflows.
    #[arg(long = "remote-path")]
    pub remote_path: Option<String>,

    /// RouterOS-generated backup/export base name.
    #[arg(long)]
    pub name: Option<String>,

    /// Local text source for script workflows. Use @<path>.
    #[arg(long)]
    pub source: Option<String>,

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

    if tokens.first().map(String::as_str) == Some("raw") {
        return parse_raw_invocation(tokens);
    }

    let action_index = tokens
        .iter()
        .position(|token| is_routeros_action(token))
        .or_else(|| legacy_action_index(tokens));
    let Some(action_index) = action_index else {
        return Err(Box::new(RosWireError::usage(
            "missing action: expected <path...> <action>",
        )));
    };

    if action_index == 0 {
        return Err(Box::new(RosWireError::usage(
            "missing path: expected <path...> <action>",
        )));
    }

    let path = tokens[..action_index].to_vec();
    let action = tokens[action_index].to_owned();
    let (resolved_args, flags) = parse_invocation_args(&tokens[action_index + 1..])?;

    Ok(ParsedInvocation {
        path,
        action,
        resolved_args,
        flags,
    })
}

fn parse_raw_invocation(tokens: &[String]) -> RosWireResult<ParsedInvocation> {
    let Some(raw_path) = tokens.get(1) else {
        return Err(Box::new(RosWireError::usage(
            "raw command requires a RouterOS API path, e.g. roswire raw /system/resource/print --json",
        )));
    };

    let (resolved_args, flags) = parse_invocation_args(&tokens[2..])?;

    Ok(ParsedInvocation {
        path: vec!["raw".to_owned()],
        action: raw_path.to_owned(),
        resolved_args,
        flags,
    })
}

fn parse_invocation_args(
    tokens: &[String],
) -> RosWireResult<(BTreeMap<String, String>, Vec<String>)> {
    let mut resolved_args = BTreeMap::new();
    let mut flags = Vec::new();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            if token.is_empty() {
                return Err(Box::new(RosWireError::usage(
                    "argument flag cannot be empty",
                )));
            }
            flags.push(token.to_owned());
            continue;
        };

        if key.is_empty() {
            return Err(Box::new(RosWireError::usage(
                "argument key cannot be empty",
            )));
        }

        resolved_args.insert(key.to_owned(), value.to_owned());
    }

    Ok((resolved_args, flags))
}

fn is_routeros_action(token: &str) -> bool {
    matches!(token, "print" | "add" | "set" | "remove")
}

fn legacy_action_index(tokens: &[String]) -> Option<usize> {
    let first_kv_index = tokens
        .iter()
        .position(|token| token.contains('='))
        .unwrap_or(tokens.len());
    first_kv_index.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{parse_invocation, Cli, ProtocolMode, TransferIfExists};
    use clap::Parser;

    #[test]
    fn parses_print_path_and_action() {
        let cli =
            Cli::try_parse_from(["roswire", "ip", "address", "print"]).expect("args should parse");
        let invocation = parse_invocation(&cli.tokens).expect("invocation should parse");

        assert_eq!(invocation.path, vec!["ip", "address"]);
        assert_eq!(invocation.action, "print");
        assert!(invocation.resolved_args.is_empty());
        assert!(invocation.flags.is_empty());
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
        assert!(invocation.flags.is_empty());
    }

    #[test]
    fn parses_print_bare_options_after_action() {
        let cli = Cli::try_parse_from(["roswire", "ip", "firewall", "filter", "print", "stats"])
            .expect("args should parse");
        let invocation = parse_invocation(&cli.tokens).expect("invocation should parse");

        assert_eq!(invocation.path, vec!["ip", "firewall", "filter"]);
        assert_eq!(invocation.action, "print");
        assert_eq!(invocation.flags, vec!["stats"]);
        assert!(invocation.resolved_args.is_empty());
    }

    #[test]
    fn parses_raw_path_before_bare_options() {
        let cli = Cli::try_parse_from([
            "roswire",
            "raw",
            "/ip/firewall/connection/print",
            "count-only",
        ])
        .expect("args should parse");
        let invocation = parse_invocation(&cli.tokens).expect("invocation should parse");

        assert_eq!(invocation.path, vec!["raw"]);
        assert_eq!(invocation.action, "/ip/firewall/connection/print");
        assert_eq!(invocation.flags, vec!["count-only"]);
        assert!(invocation.resolved_args.is_empty());
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
        assert!(!cli.refresh);
        assert!(!cli.dry_run);
        assert!(!cli.include_remote);
        assert!(!cli.stdin);
    }

    #[test]
    fn supports_remote_schema_refresh_flag() {
        let cli = Cli::try_parse_from(["roswire", "schema", "discover", "--remote", "--refresh"])
            .expect("refresh flag should parse");

        assert!(cli.remote);
        assert!(cli.refresh);
        assert_eq!(cli.tokens, vec!["schema", "discover"]);
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
            "--if-exists",
            "skip",
            "--connect-timeout-seconds",
            "5",
            "--wait-timeout-seconds",
            "6",
            "--transfer-timeout-seconds",
            "7",
            "--cleanup-timeout-seconds",
            "8",
            "--retries",
            "2",
            "--retry-delay-seconds",
            "1",
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
        assert_eq!(cli.if_exists, TransferIfExists::Skip);
        assert_eq!(cli.connect_timeout_seconds, Some(5));
        assert_eq!(cli.wait_timeout_seconds, Some(6));
        assert_eq!(cli.transfer_timeout_seconds, Some(7));
        assert_eq!(cli.cleanup_timeout_seconds, Some(8));
        assert_eq!(cli.retries, 2);
        assert_eq!(cli.retry_delay_seconds, 1);
        assert!(cli.cleanup);
        assert_eq!(
            cli.tokens,
            vec!["file", "upload", "setup.rsc", "flash/setup.rsc"]
        );
    }

    #[test]
    fn supports_tls_cert_fingerprint_flag() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--tls-cert-fingerprint",
            "SHA256:abc",
            "ip/address/print",
        ])
        .expect("tls cert fingerprint flag should parse");

        assert_eq!(cli.tls_cert_fingerprint.as_deref(), Some("SHA256:abc"));
    }

    #[test]
    fn supports_script_source_flag_after_tokens() {
        let cli = Cli::try_parse_from([
            "roswire",
            "script",
            "put",
            "bootstrap",
            "--source",
            "@setup.rsc",
            "--dry-run",
        ])
        .expect("script source flag should parse");

        assert_eq!(cli.source.as_deref(), Some("@setup.rsc"));
        assert!(cli.dry_run);
        assert_eq!(cli.tokens, vec!["script", "put", "bootstrap"]);
    }

    #[test]
    fn supports_raw_allow_write_flag_after_tokens() {
        let cli = Cli::try_parse_from([
            "roswire",
            "raw",
            "/ip/address/add",
            "address=192.0.2.10/24",
            "--allow-write",
        ])
        .expect("raw allow-write flag should parse");

        assert!(cli.allow_write);
        assert_eq!(
            cli.tokens,
            vec!["raw", "/ip/address/add", "address=192.0.2.10/24"]
        );
    }
}
