use crate::args::Cli;
use crate::config;
use crate::error::{self, ErrorContext, RosWireError, RosWireResult};
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;

const PLAN_SCHEMA_VERSION: &str = "roswire.transfer.plan.v1";
const DEFAULT_TRANSFER_BACKEND: &str = "ssh";

#[derive(Debug, Clone, PartialEq, Eq)]
enum TransferCommand {
    FileUpload { local: String, remote: String },
    FileDownload { remote: String, local: String },
    Import { local: String },
    BackupDownload { local: String },
    ExportDownload { local: String },
}

impl TransferCommand {
    fn operation(&self) -> &'static str {
        match self {
            Self::FileUpload { .. } => "file.upload",
            Self::FileDownload { .. } => "file.download",
            Self::Import { .. } => "import.plan",
            Self::BackupDownload { .. } => "backup.download",
            Self::ExportDownload { .. } => "export.download",
        }
    }

    fn command_name(&self) -> &'static str {
        match self {
            Self::FileUpload { .. } => "file/upload",
            Self::FileDownload { .. } => "file/download",
            Self::Import { .. } => "import",
            Self::BackupDownload { .. } => "backup/download",
            Self::ExportDownload { .. } => "export/download",
        }
    }

    fn context_args(&self) -> BTreeMap<String, String> {
        match self {
            Self::FileUpload { local, remote } => BTreeMap::from([
                ("local_path".to_owned(), redact_local_path(local)),
                ("remote_path".to_owned(), redact_remote_path(remote)),
            ]),
            Self::FileDownload { remote, local } => BTreeMap::from([
                ("remote_path".to_owned(), redact_remote_path(remote)),
                ("local_path".to_owned(), redact_local_path(local)),
            ]),
            Self::Import { local }
            | Self::BackupDownload { local }
            | Self::ExportDownload { local } => {
                BTreeMap::from([("local_path".to_owned(), redact_local_path(local))])
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransferPlan {
    pub schema_version: &'static str,
    pub operation: String,
    pub dry_run: bool,
    pub transfer_backend: String,
    pub preconditions: TransferPreconditions,
    pub paths: TransferPaths,
    pub cleanup: TransferCleanup,
    pub steps: Vec<TransferStep>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransferPreconditions {
    pub device_access: &'static str,
    pub ssh_host_key: &'static str,
    pub ssh: SshTransferSummary,
    pub allow_from: Vec<String>,
    pub ensure_ssh: bool,
    pub restore_ssh: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SshTransferSummary {
    pub port: u16,
    pub user: String,
    pub auth_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransferPaths {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary_remote_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary_local_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransferCleanup {
    pub strategy: String,
    pub remote_paths: Vec<String>,
    pub local_paths: Vec<String>,
    pub restore_ssh: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransferStep {
    pub order: u8,
    pub action: String,
    pub description: String,
    pub dry_run_side_effects: &'static str,
}

pub fn handle(tokens: &[String], cli: &Cli) -> Option<RosWireResult<String>> {
    let command = match parse_transfer_command(tokens)? {
        Ok(command) => command,
        Err(error) => return Some(Err(error)),
    };
    let env = read_env_map();
    Some(build_plan_for_env(command, cli, &env).and_then(|plan| render_json(&plan)))
}

fn parse_transfer_command(tokens: &[String]) -> Option<RosWireResult<TransferCommand>> {
    match tokens {
        [file, action, local, remote] if file == "file" && action == "upload" => {
            Some(Ok(TransferCommand::FileUpload {
                local: local.clone(),
                remote: remote.clone(),
            }))
        }
        [file, action, remote, local] if file == "file" && action == "download" => {
            Some(Ok(TransferCommand::FileDownload {
                remote: remote.clone(),
                local: local.clone(),
            }))
        }
        [command, local] if command == "import" => Some(Ok(TransferCommand::Import {
            local: local.clone(),
        })),
        [command, action, local] if command == "backup" && action == "download" => {
            Some(Ok(TransferCommand::BackupDownload {
                local: local.clone(),
            }))
        }
        [command, action, local] if command == "export" && action == "download" => {
            Some(Ok(TransferCommand::ExportDownload {
                local: local.clone(),
            }))
        }
        [command, ..] if matches!(command.as_str(), "file" | "import" | "backup" | "export") => {
            Some(Err(Box::new(RosWireError::usage(
                "transfer commands require one of: file upload <local> <remote>, file download <remote> <local>, import <local>, backup download <local>, export download <local>",
            ))))
        }
        _ => None,
    }
}

fn build_plan_for_env(
    command: TransferCommand,
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> RosWireResult<TransferPlan> {
    if let Some(host) = cli
        .host
        .as_deref()
        .or_else(|| env.get("ROS_HOST").map(String::as_str))
    {
        config::validate_remote_host(host)?;
    }

    let backend = resolve_transfer_backend(cli, env)?;
    if backend != DEFAULT_TRANSFER_BACKEND {
        return Err(Box::new(RosWireError::usage(format!(
            "unsupported transfer backend: {backend}",
        ))));
    }

    let context = transfer_context(&command, &backend, cli, env);
    if !cli.dry_run {
        return Err(Box::new(
            RosWireError::unsupported_action(
                "real SSH file transfer is not implemented yet; rerun with --dry-run --json",
            )
            .with_context(context),
        ));
    }

    let host_key = cli
        .ssh_host_key
        .clone()
        .or_else(|| env.get("ROS_SSH_HOST_KEY").cloned())
        .filter(|value| !value.trim().is_empty());
    if host_key.is_none() {
        return Err(Box::new(
            RosWireError::ssh_host_key_required(
                "SSH transfer dry-run requires an expected RouterOS SSH host key fingerprint",
            )
            .with_context(context),
        ));
    }

    let allow_from = resolve_allow_from(cli, env).map_err(|error| {
        Box::new(
            (*error)
                .clone()
                .with_context(transfer_context(&command, &backend, cli, env)),
        )
    })?;
    if allow_from.is_empty() {
        return Err(Box::new(
            RosWireError::ssh_whitelist_required(
                "SSH transfer dry-run requires at least one allow-from CIDR",
            )
            .with_context(context),
        ));
    }

    let profile = load_selected_profile(cli, env)?;
    let ssh = resolve_ssh_transfer_summary(cli, env, profile.as_ref())?;

    Ok(plan_from_command(command, backend, allow_from, ssh, cli))
}

fn resolve_transfer_backend(cli: &Cli, env: &BTreeMap<String, String>) -> RosWireResult<String> {
    let backend = cli
        .transfer
        .map(|value| value.as_str().to_owned())
        .or_else(|| env.get("ROS_TRANSFER").cloned())
        .unwrap_or_else(|| DEFAULT_TRANSFER_BACKEND.to_owned());
    match backend.as_str() {
        DEFAULT_TRANSFER_BACKEND => Ok(backend),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid transfer value: {backend}",
        )))),
    }
}

fn resolve_allow_from(cli: &Cli, env: &BTreeMap<String, String>) -> RosWireResult<Vec<String>> {
    let mut values = cli.allow_from.clone();
    if values.is_empty() {
        if let Some(env_value) = env.get("ROS_SSH_ALLOW_FROM") {
            values.extend(env_value.split(',').map(str::to_owned));
        }
    }

    let mut cidrs = Vec::new();
    for value in values {
        for cidr in value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            validate_safe_cidr(cidr)?;
            cidrs.push(cidr.to_owned());
        }
    }

    Ok(cidrs)
}

fn validate_safe_cidr(cidr: &str) -> RosWireResult<()> {
    let (addr, prefix) = cidr.split_once('/').ok_or_else(|| {
        Box::new(RosWireError::usage(format!(
            "allow-from must be CIDR notation: {cidr}",
        )))
    })?;
    let address = addr.parse::<IpAddr>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid allow-from address `{addr}`: {error}",
        )))
    })?;
    let prefix = prefix.parse::<u8>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid allow-from prefix `{prefix}`: {error}",
        )))
    })?;

    match address {
        IpAddr::V4(_) if prefix > 32 => Err(Box::new(RosWireError::usage(format!(
            "invalid IPv4 allow-from prefix: {prefix}",
        )))),
        IpAddr::V4(_) if prefix < 24 => Err(Box::new(RosWireError::ssh_whitelist_unsafe(
            "SSH allow-from IPv4 CIDR is too broad",
        ))),
        IpAddr::V6(_) if prefix > 128 => Err(Box::new(RosWireError::usage(format!(
            "invalid IPv6 allow-from prefix: {prefix}",
        )))),
        IpAddr::V6(_) if prefix < 64 => Err(Box::new(RosWireError::ssh_whitelist_unsafe(
            "SSH allow-from IPv6 CIDR is too broad",
        ))),
        _ => Ok(()),
    }
}

fn plan_from_command(
    command: TransferCommand,
    backend: String,
    allow_from: Vec<String>,
    ssh: SshTransferSummary,
    cli: &Cli,
) -> TransferPlan {
    let mut cleanup_remote_paths = Vec::new();
    let mut cleanup_local_paths = Vec::new();
    let paths = match &command {
        TransferCommand::FileUpload { local, remote } => {
            let temporary_remote = temporary_remote_path(remote);
            if cli.cleanup {
                cleanup_remote_paths.push(redact_remote_path(&temporary_remote));
            }
            TransferPaths {
                local_path: Some(redact_local_path(local)),
                remote_path: Some(redact_remote_path(remote)),
                temporary_remote_path: Some(redact_remote_path(&temporary_remote)),
                temporary_local_path: None,
            }
        }
        TransferCommand::FileDownload { remote, local } => {
            let temporary_local = temporary_local_path(local);
            if cli.cleanup {
                cleanup_local_paths.push(temporary_local.clone());
            }
            TransferPaths {
                local_path: Some(redact_local_path(local)),
                remote_path: Some(redact_remote_path(remote)),
                temporary_remote_path: None,
                temporary_local_path: Some(temporary_local),
            }
        }
        TransferCommand::Import { local } => {
            let remote = cli
                .remote_path
                .clone()
                .unwrap_or_else(|| format!("flash/roswire-import-{}", file_name(local)));
            let temporary_remote = temporary_remote_path(&remote);
            if cli.cleanup {
                cleanup_remote_paths.push(redact_remote_path(&temporary_remote));
            }
            TransferPaths {
                local_path: Some(redact_local_path(local)),
                remote_path: Some(redact_remote_path(&remote)),
                temporary_remote_path: Some(redact_remote_path(&temporary_remote)),
                temporary_local_path: None,
            }
        }
        TransferCommand::BackupDownload { local } => {
            let name = cli.name.as_deref().unwrap_or("roswire-backup");
            let remote = format!("{name}.backup");
            let temporary_local = temporary_local_path(local);
            if cli.cleanup {
                cleanup_remote_paths.push(redact_remote_path(&remote));
                cleanup_local_paths.push(temporary_local.clone());
            }
            TransferPaths {
                local_path: Some(redact_local_path(local)),
                remote_path: Some(redact_remote_path(&remote)),
                temporary_remote_path: Some(redact_remote_path(&remote)),
                temporary_local_path: Some(temporary_local),
            }
        }
        TransferCommand::ExportDownload { local } => {
            let name = cli.name.as_deref().unwrap_or("roswire-export");
            let remote = format!("{name}.rsc");
            let temporary_local = temporary_local_path(local);
            if cli.cleanup {
                cleanup_remote_paths.push(redact_remote_path(&remote));
                cleanup_local_paths.push(temporary_local.clone());
            }
            TransferPaths {
                local_path: Some(redact_local_path(local)),
                remote_path: Some(redact_remote_path(&remote)),
                temporary_remote_path: Some(redact_remote_path(&remote)),
                temporary_local_path: Some(temporary_local),
            }
        }
    };

    TransferPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        operation: command.operation().to_owned(),
        dry_run: true,
        transfer_backend: backend,
        preconditions: TransferPreconditions {
            device_access: "none",
            ssh_host_key: "provided",
            ssh,
            allow_from,
            ensure_ssh: cli.ensure_ssh,
            restore_ssh: cli.restore_ssh,
        },
        cleanup: TransferCleanup {
            strategy: if cli.cleanup {
                "cleanup-temporary-files".to_owned()
            } else {
                "preserve-temporary-files".to_owned()
            },
            remote_paths: cleanup_remote_paths,
            local_paths: cleanup_local_paths,
            restore_ssh: cli.restore_ssh,
        },
        steps: plan_steps(&command, cli),
        paths,
    }
}

fn load_selected_profile(
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> RosWireResult<Option<config::ProfileConfig>> {
    let paths = config::ConfigPaths::from_home(config::resolve_home_path(
        env.get("ROSWIRE_HOME").map(String::as_str),
    ));
    if !paths.config.exists() {
        return Ok(None);
    }

    config::ensure_secure_directory_permissions(&paths.home)?;
    config::ensure_secure_file_permissions(&paths.config)?;
    let config_file = config::load_config_file(&paths.config)?;
    let profile_name = config::select_active_profile(
        cli.profile.as_deref(),
        env.get("ROS_PROFILE").map(String::as_str),
        &config_file,
    )?;
    Ok(config_file.profiles.get(&profile_name).cloned())
}

fn resolve_ssh_transfer_summary(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<SshTransferSummary> {
    let port = match cli
        .ssh_port
        .map(Ok)
        .or_else(|| env.get("ROS_SSH_PORT").map(|value| parse_port(value)))
        .or_else(|| profile.and_then(|profile| profile.ssh_port.map(Ok)))
    {
        Some(port) => port?,
        None => 22,
    };

    let user = cli
        .ssh_user
        .clone()
        .or_else(|| env.get("ROS_SSH_USER").cloned())
        .or_else(|| profile.and_then(|profile| profile.ssh_user.clone()))
        .or_else(|| cli.user.clone())
        .or_else(|| env.get("ROS_USER").cloned())
        .or_else(|| profile.and_then(|profile| profile.user.clone()))
        .unwrap_or_else(|| "reuse-api-user".to_owned());

    let key_path = cli
        .ssh_key
        .clone()
        .or_else(|| env.get("ROS_SSH_KEY").cloned())
        .or_else(|| profile.and_then(|profile| profile.ssh_key.clone()))
        .filter(|value| !value.trim().is_empty())
        .map(|value| redact_local_path(&value));
    let auth_method = if key_path.is_some() {
        "key".to_owned()
    } else if cli.ssh_password.is_some()
        || env.get("ROS_SSH_PASSWORD").is_some()
        || profile.is_some_and(|profile| profile.secrets.contains_key("ssh_password"))
    {
        "password".to_owned()
    } else {
        "password-reuses-api".to_owned()
    };

    Ok(SshTransferSummary {
        port,
        user,
        auth_method,
        key_path,
    })
}

fn parse_port(value: &str) -> RosWireResult<u16> {
    value.parse::<u16>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid SSH port value `{value}`: {error}",
        )))
    })
}

fn plan_steps(command: &TransferCommand, cli: &Cli) -> Vec<TransferStep> {
    let mut steps = vec![
        TransferStep {
            order: 1,
            action: "verify-ssh-host-key".to_owned(),
            description: "Verify RouterOS SSH host key fingerprint before any transfer".to_owned(),
            dry_run_side_effects: "none",
        },
        TransferStep {
            order: 2,
            action: "verify-ssh-whitelist".to_owned(),
            description: "Use allow-from CIDR as the only planned SSH client whitelist".to_owned(),
            dry_run_side_effects: "none",
        },
    ];

    if cli.ensure_ssh {
        steps.push(TransferStep {
            order: 3,
            action: "ensure-ssh-service".to_owned(),
            description: "Plan RouterOS /ip service ssh enable/address update before transfer"
                .to_owned(),
            dry_run_side_effects: "none",
        });
    }

    let transfer_order = if cli.ensure_ssh { 4 } else { 3 };
    steps.push(TransferStep {
        order: transfer_order,
        action: command.operation().to_owned(),
        description: transfer_description(command, cli),
        dry_run_side_effects: "none",
    });

    let mut next_order = transfer_order + 1;
    if cli.cleanup {
        steps.push(TransferStep {
            order: next_order,
            action: "cleanup-temporary-files".to_owned(),
            description: "Remove only temporary files listed in the cleanup policy".to_owned(),
            dry_run_side_effects: "none",
        });
        next_order += 1;
    }

    if cli.restore_ssh {
        steps.push(TransferStep {
            order: next_order,
            action: "restore-ssh-service".to_owned(),
            description: "Restore RouterOS SSH service state captured before ensure-ssh".to_owned(),
            dry_run_side_effects: "none",
        });
    }

    steps
}

fn transfer_description(command: &TransferCommand, cli: &Cli) -> String {
    match command {
        TransferCommand::FileUpload { .. } => {
            "Upload local file to temporary remote path, then move into final remote path"
                .to_owned()
        }
        TransferCommand::FileDownload { .. } => {
            "Download remote file to a temporary local path, then move into final local path"
                .to_owned()
        }
        TransferCommand::Import { .. } => {
            "Upload local .rsc to a temporary remote path, then execute /import file-name=<temp>"
                .to_owned()
        }
        TransferCommand::BackupDownload { .. } => {
            "Execute /system/backup/save name=<name>, wait for .backup, then download".to_owned()
        }
        TransferCommand::ExportDownload { .. } if cli.compact => {
            "Execute compact /export file=<name>, wait for .rsc, then download".to_owned()
        }
        TransferCommand::ExportDownload { .. } => {
            "Execute /export file=<name>, wait for .rsc, then download".to_owned()
        }
    }
}

fn transfer_context(
    command: &TransferCommand,
    backend: &str,
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> ErrorContext {
    ErrorContext {
        command: command.command_name().to_owned(),
        path: command
            .command_name()
            .split('/')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        action: command.operation().to_owned(),
        requested_protocol: cli
            .protocol
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| {
                env.get("ROS_PROTOCOL")
                    .cloned()
                    .unwrap_or_else(|| "auto".to_owned())
            }),
        selected_protocol: "unknown".to_owned(),
        transfer_backend: Some(backend.to_owned()),
        routeros_version: cli
            .routeros_version
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| {
                env.get("ROS_ROUTEROS_VERSION")
                    .cloned()
                    .unwrap_or_else(|| "auto".to_owned())
            }),
        host: cli
            .host
            .clone()
            .or_else(|| env.get("ROS_HOST").cloned())
            .unwrap_or_default(),
        resolved_args: error::redact_resolved_args(&command.context_args()),
    }
}

fn temporary_remote_path(remote: &str) -> String {
    format!("{}.roswire.tmp", remote.trim_end_matches('/'))
}

fn temporary_local_path(local: &str) -> String {
    format!("{}.part", redact_local_path(local))
}

fn redact_local_path(path: &str) -> String {
    let path_ref = Path::new(path);
    let value = if path_ref.is_absolute() {
        format!("***REDACTED***/{}", file_name(path))
    } else {
        path.to_owned()
    };
    redact_sensitive_path(&value)
}

fn redact_remote_path(path: &str) -> String {
    redact_sensitive_path(path)
}

fn redact_sensitive_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if error::is_sensitive_key(segment) {
                "***REDACTED***".to_owned()
            } else {
                segment.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .or_else(|| path.rsplit('/').find(|part| !part.is_empty()))
        .unwrap_or("roswire-file")
        .to_owned()
}

fn render_json<T: Serialize>(value: &T) -> RosWireResult<String> {
    serde_json::to_string(value).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize transfer plan: {error}",
        )))
    })
}

fn read_env_map() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

#[cfg(test)]
mod tests {
    use super::{build_plan_for_env, parse_transfer_command, validate_safe_cidr, TransferCommand};
    use crate::args::Cli;
    use crate::error::ErrorCode;
    use clap::Parser;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn file_upload_plan_contains_safe_preconditions_and_paths() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "upload",
            "/Users/example/private/setup.rsc",
            "flash/setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--allow-from",
            "203.0.113.10/32",
            "--ensure-ssh",
            "--restore-ssh",
            "--cleanup",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let plan = build_plan_for_env(command, &cli, &isolated_env()).expect("plan should build");

        assert_eq!(plan.schema_version, "roswire.transfer.plan.v1");
        assert_eq!(plan.operation, "file.upload");
        assert!(plan.dry_run);
        assert_eq!(plan.preconditions.ssh_host_key, "provided");
        assert_eq!(plan.preconditions.ssh.port, 22);
        assert_eq!(plan.preconditions.ssh.user, "reuse-api-user");
        assert_eq!(plan.preconditions.ssh.auth_method, "password-reuses-api");
        assert_eq!(plan.preconditions.allow_from, vec!["203.0.113.10/32"]);
        assert_eq!(
            plan.paths.local_path.as_deref(),
            Some("***REDACTED***/setup.rsc")
        );
        assert_eq!(plan.paths.remote_path.as_deref(), Some("flash/setup.rsc"));
        assert_eq!(
            plan.paths.temporary_remote_path.as_deref(),
            Some("flash/setup.rsc.roswire.tmp")
        );
        assert_eq!(
            plan.cleanup.remote_paths,
            vec!["flash/setup.rsc.roswire.tmp"]
        );
        assert!(plan
            .steps
            .iter()
            .all(|step| step.dry_run_side_effects == "none"));
    }

    #[test]
    fn import_plan_uses_remote_path_override() {
        let cli = Cli::try_parse_from([
            "roswire",
            "import",
            "setup.rsc",
            "--remote-path",
            "flash/import/setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--allow-from",
            "203.0.113.10/32",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let plan = build_plan_for_env(command, &cli, &isolated_env()).expect("plan should build");

        assert_eq!(plan.operation, "import.plan");
        assert_eq!(
            plan.paths.remote_path.as_deref(),
            Some("flash/import/setup.rsc")
        );
        assert!(plan
            .steps
            .iter()
            .any(|step| step.description.contains("/import")));
    }

    #[test]
    fn ssh_transfer_summary_prefers_cli_then_env_then_profile_and_redacts_key_path() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "api-profile"
ssh_port = 2200
ssh_user = "profile-ssh"
ssh_key = "/Users/profile/.ssh/id_profile"

[profiles.studio.secrets.ssh_password]
type = "same-as"
target = "password"
"#,
        );
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--ssh-port",
            "2022",
            "--ssh-user",
            "cli-ssh",
            "--ssh-key",
            "/Users/cli/.ssh/id_cli",
            "--allow-from",
            "203.0.113.10/32",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let env = BTreeMap::from([
            ("ROSWIRE_HOME".to_owned(), temp.path().display().to_string()),
            ("ROS_SSH_PORT".to_owned(), "2222".to_owned()),
            ("ROS_SSH_USER".to_owned(), "env-ssh".to_owned()),
            (
                "ROS_SSH_KEY".to_owned(),
                "/Users/env/.ssh/id_env".to_owned(),
            ),
        ]);

        let plan = build_plan_for_env(command, &cli, &env).expect("plan should build");

        assert_eq!(plan.preconditions.ssh.port, 2022);
        assert_eq!(plan.preconditions.ssh.user, "cli-ssh");
        assert_eq!(plan.preconditions.ssh.auth_method, "key");
        assert_eq!(
            plan.preconditions.ssh.key_path.as_deref(),
            Some("***REDACTED***/id_cli"),
        );
    }

    #[test]
    fn ssh_transfer_summary_uses_env_then_profile_fallbacks() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "api-profile"
ssh_port = 2200
ssh_user = "profile-ssh"

[profiles.studio.secrets.ssh_password]
type = "same-as"
target = "password"
"#,
        );
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--allow-from",
            "203.0.113.10/32",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let env = BTreeMap::from([
            ("ROSWIRE_HOME".to_owned(), temp.path().display().to_string()),
            ("ROS_SSH_USER".to_owned(), "env-ssh".to_owned()),
            ("ROS_SSH_PASSWORD".to_owned(), "env-secret".to_owned()),
        ]);

        let plan = build_plan_for_env(command, &cli, &env).expect("plan should build");

        assert_eq!(plan.preconditions.ssh.port, 2200);
        assert_eq!(plan.preconditions.ssh.user, "env-ssh");
        assert_eq!(plan.preconditions.ssh.auth_method, "password");
        assert_eq!(plan.preconditions.ssh.key_path, None);
    }

    #[test]
    fn backup_and_export_plans_use_generated_remote_artifacts() {
        let backup_cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            "backup.backup",
            "--name",
            "pre-change",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--allow-from",
            "203.0.113.10/32",
        ])
        .expect("cli should parse");
        let backup = build_plan_for_env(
            parse_transfer_command(&backup_cli.tokens)
                .expect("transfer command should be detected")
                .expect("transfer command should parse"),
            &backup_cli,
            &BTreeMap::new(),
        )
        .expect("backup plan should build");

        let export_cli = Cli::try_parse_from([
            "roswire",
            "export",
            "download",
            "config.rsc",
            "--compact",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--allow-from",
            "203.0.113.10/32",
        ])
        .expect("cli should parse");
        let export = build_plan_for_env(
            parse_transfer_command(&export_cli.tokens)
                .expect("transfer command should be detected")
                .expect("transfer command should parse"),
            &export_cli,
            &BTreeMap::new(),
        )
        .expect("export plan should build");

        assert_eq!(
            backup.paths.remote_path.as_deref(),
            Some("pre-change.backup")
        );
        assert_eq!(
            export.paths.remote_path.as_deref(),
            Some("roswire-export.rsc")
        );
        assert!(export
            .steps
            .iter()
            .any(|step| step.description.contains("compact /export")));
    }

    #[test]
    fn missing_host_key_returns_structured_error() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
            "--dry-run",
            "--allow-from",
            "203.0.113.10/32",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = build_plan_for_env(command, &cli, &isolated_env())
            .expect_err("host key should be required");

        assert_eq!(error.error_code, ErrorCode::SshHostKeyRequired);
        assert_eq!(error.context.transfer_backend.as_deref(), Some("ssh"));
        assert_eq!(error.context.command, "file/download");
    }

    #[test]
    fn missing_allow_from_returns_structured_error() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = build_plan_for_env(command, &cli, &isolated_env())
            .expect_err("allow-from should be required");

        assert_eq!(error.error_code, ErrorCode::SshWhitelistRequired);
    }

    #[test]
    fn unsafe_allow_from_returns_structured_error() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
            "--dry-run",
            "--ssh-host-key",
            "SHA256:test",
            "--allow-from",
            "0.0.0.0/0",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = build_plan_for_env(command, &cli, &isolated_env())
            .expect_err("wide allow-from should fail");

        assert_eq!(error.error_code, ErrorCode::SshWhitelistUnsafe);
    }

    #[test]
    fn cidr_validation_accepts_narrow_client_ranges() {
        validate_safe_cidr("203.0.113.10/32").expect("single IPv4 host should be safe");
        validate_safe_cidr("2001:db8::1/128").expect("single IPv6 host should be safe");
    }

    #[test]
    fn non_transfer_tokens_are_ignored() {
        assert!(parse_transfer_command(&["ip".to_owned(), "address".to_owned()]).is_none());
    }

    #[test]
    fn transfer_command_usage_is_structured() {
        let result = parse_transfer_command(&["file".to_owned(), "upload".to_owned()])
            .expect("file command should be handled");

        assert!(result.is_err());
    }

    #[test]
    fn command_names_are_stable() {
        let command = TransferCommand::FileUpload {
            local: "setup.rsc".to_owned(),
            remote: "flash/setup.rsc".to_owned(),
        };

        assert_eq!(command.command_name(), "file/upload");
        assert_eq!(command.operation(), "file.upload");
    }

    fn write_config(home: &std::path::Path, contents: &str) {
        fs::write(home.join("config.toml"), contents).expect("config should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(home, fs::Permissions::from_mode(0o700))
                .expect("home permissions should be set");
            fs::set_permissions(home.join("config.toml"), fs::Permissions::from_mode(0o600))
                .expect("config permissions should be set");
        }
    }

    fn isolated_env() -> BTreeMap<String, String> {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        BTreeMap::from([(
            "ROSWIRE_HOME".to_owned(),
            temp.path().join("missing-home").display().to_string(),
        )])
    }
}
