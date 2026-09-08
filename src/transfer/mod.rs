use crate::args::{Cli, TransferIfExists};
use crate::config;
use crate::error::{self, ErrorContext, RosWireError, RosWireResult};
use crate::protocol::classic::{
    transport::{ApiStream, TcpApiStream, TlsApiStream, TlsTrust},
    ClassicApiSession,
};
use crate::protocol::rest::RestClient;
use base64::{engine::general_purpose::STANDARD_NO_PAD as BASE64_NO_PAD, Engine as _};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::IpAddr;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

mod command;
mod plan;
mod policy;
mod ssh_data;
mod ssh_service;
mod workflow;

use command::*;
use plan::*;
use policy::*;
use ssh_data::*;
use ssh_service::*;
use workflow::*;

const PLAN_SCHEMA_VERSION: &str = "roswire.transfer.plan.v1";
const DEFAULT_TRANSFER_BACKEND: &str = "ssh";
const RESULT_SCHEMA_VERSION: &str = "roswire.transfer.result.v1";
const MAX_TRANSFER_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_WAIT_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_TRANSFER_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_CLEANUP_TIMEOUT_SECONDS: u64 = 10;
const MAX_TRANSFER_RETRIES: u8 = 5;
const WORKFLOW_FILE_WAIT_INTERVAL: Duration = Duration::from_secs(1);
const SSH_KEY_PASSPHRASE_SECRET: &str = "ssh_key_passphrase";

pub fn handle(tokens: &[String], cli: &Cli) -> Option<RosWireResult<String>> {
    let command = match parse_transfer_command(tokens)? {
        Ok(command) => command,
        Err(error) => return Some(Err(error)),
    };
    let env = read_env_map();
    Some(handle_transfer_for_env(command, cli, &env))
}

fn handle_transfer_for_env(
    command: TransferCommand,
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> RosWireResult<String> {
    if cli.dry_run {
        return build_plan_for_env(command, cli, env).and_then(|plan| render_json(&plan));
    }

    execute_transfer_for_env(command, cli, env).and_then(|payload| render_json(&payload))
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
    let profile_name = match config::select_active_profile(cli.profile.as_deref(), &config_file) {
        Ok(profile_name) => profile_name,
        Err(error) if cli.profile.is_some() => return Err(error),
        Err(_) => return Ok(None),
    };
    Ok(config_file.profiles.get(&profile_name).cloned())
}

fn selected_context(context: &ErrorContext, selected_protocol: &str) -> ErrorContext {
    let mut context = context.clone();
    context.selected_protocol = selected_protocol.to_owned();
    context
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DestinationDecision {
    Proceed,
    Skip,
}

fn prepare_local_destination(
    local: &str,
    policy: &TransferPolicy,
    context: &ErrorContext,
) -> RosWireResult<DestinationDecision> {
    if !Path::new(local).exists() {
        return Ok(DestinationDecision::Proceed);
    }

    match policy.if_exists {
        TransferIfExists::Overwrite => Ok(DestinationDecision::Proceed),
        TransferIfExists::Skip => Ok(DestinationDecision::Skip),
        TransferIfExists::Fail => Err(Box::new(
            RosWireError::file_transfer_failed(format!(
                "destination already exists and --if-exists=fail was requested: {}",
                redact_local_path(local)
            ))
            .with_context(context.clone()),
        )),
    }
}

fn generated_backup_base_name(cli: &Cli) -> String {
    cli.name
        .clone()
        .unwrap_or_else(|| "roswire-backup".to_owned())
}

fn generated_backup_name(cli: &Cli) -> String {
    format!("{}.backup", generated_backup_base_name(cli))
}

fn generated_export_base_name(cli: &Cli) -> String {
    cli.name
        .clone()
        .unwrap_or_else(|| "roswire-export".to_owned())
}

fn generated_export_name(cli: &Cli) -> String {
    format!("{}.rsc", generated_export_base_name(cli))
}

fn transfer_context(
    command: &TransferCommand,
    backend: &str,
    cli: &Cli,
    profile: Option<&config::ProfileConfig>,
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
            .or_else(|| profile.and_then(|profile| profile.protocol.clone()))
            .unwrap_or_else(|| "auto".to_owned()),
        selected_protocol: "unknown".to_owned(),
        transfer_backend: Some(backend.to_owned()),
        routeros_version: cli
            .routeros_version
            .map(|value| value.as_str().to_owned())
            .or_else(|| profile.and_then(|profile| profile.routeros_version.clone()))
            .unwrap_or_else(|| "auto".to_owned()),
        host: cli
            .host
            .clone()
            .or_else(|| profile.and_then(|profile| profile.host.clone()))
            .unwrap_or_default(),
        jump: crate::jump::resolve_jump_identities(cli, profile)
            .unwrap_or_default()
            .into_iter()
            .map(|hop| hop.host)
            .collect(),
        resolved_args: error::redact_resolved_args(&command.context_args()),
    }
}

fn temporary_remote_path(remote: &str) -> String {
    format!("{}.roswire.tmp", remote.trim_end_matches('/'))
}

fn temporary_local_path(local: &str) -> String {
    format!("{}.part", redact_local_path(local))
}

fn raw_temporary_local_path(local: &str) -> String {
    format!("{local}.part")
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
    serde_json::to_string_pretty(value).map_err(|error| {
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
    use super::{
        build_plan_for_env, copy_with_sha256, default_transfer_policy, execute_classic_control,
        execute_file_workflow, handle_transfer_for_env, host_key_matches, load_selected_profile,
        merge_ssh_allow_list, parse_port, parse_transfer_command, raw_temporary_local_path,
        resolve_control_runtime_config, resolve_ssh_key_passphrase, resolve_ssh_runtime_config,
        resolve_ssh_transfer_summary, resolve_transfer_backend, routeros_bool, selected_context,
        sftp_or_scp_fallback, sha256_fingerprint, ssh_service_snapshot_from_fields,
        ssh_service_snapshot_from_json, transfer_policy, validate_safe_cidr, ControlCommand,
        ControlRuntimeConfig, LiveWorkflowBackend, SshRuntimeConfig, SshServiceSnapshot,
        TransferCommand, WorkflowBackend, DEFAULT_CONNECT_TIMEOUT_SECONDS, MAX_TRANSFER_BYTES,
    };
    use crate::args::{Cli, TransferIfExists};
    use crate::error::{ErrorCode, ErrorContext};
    use crate::protocol::classic::sentence::{read_sentence, write_sentence};
    use clap::Parser;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Cursor, Read, Result as IoResult, Write};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
        assert_eq!(plan.policy.if_exists, "overwrite");
        assert_eq!(
            plan.policy.timeouts.connect_seconds,
            DEFAULT_CONNECT_TIMEOUT_SECONDS
        );
        assert_eq!(plan.policy.retry.max_retries, 0);
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
        assert!(plan
            .steps
            .iter()
            .any(|step| step.action == "snapshot-ssh-service"));
        assert!(plan
            .steps
            .iter()
            .any(|step| step.description.contains("append/merge allow-from")));
        assert!(plan.steps.iter().any(|step| step
            .description
            .contains("process interrupts are not trapped")));
    }

    #[test]
    fn ssh_service_snapshots_parse_classic_and_rest_shapes() {
        let classic = BTreeMap::from([
            (".id".to_owned(), "*A".to_owned()),
            ("disabled".to_owned(), "yes".to_owned()),
            (
                "address".to_owned(),
                "198.51.100.4/32, 203.0.113.10/32".to_owned(),
            ),
        ]);

        let snapshot = ssh_service_snapshot_from_fields(&classic);

        assert_eq!(snapshot.id.as_deref(), Some("*A"));
        assert!(snapshot.disabled);
        assert_eq!(snapshot.address, vec!["198.51.100.4/32", "203.0.113.10/32"]);

        let rest = serde_json::json!([
            { "name": "www", "disabled": "no" },
            { ".id": "*B", "name": "ssh", "disabled": false, "address": "203.0.113.10/32" }
        ]);
        let snapshot = ssh_service_snapshot_from_json(&rest).expect("rest snapshot should parse");

        assert_eq!(snapshot.id.as_deref(), Some("*B"));
        assert!(!snapshot.disabled);
        assert_eq!(snapshot.address, vec!["203.0.113.10/32"]);
    }

    #[test]
    fn ssh_allow_list_merge_preserves_existing_restrictions() {
        let existing = vec!["198.51.100.4/32".to_owned(), "203.0.113.10/32".to_owned()];
        let additions = vec!["203.0.113.10/32".to_owned(), "203.0.113.11/32".to_owned()];

        let merged = merge_ssh_allow_list(&existing, &additions);

        assert_eq!(
            merged,
            vec!["198.51.100.4/32", "203.0.113.10/32", "203.0.113.11/32"]
        );
        assert_eq!(
            merge_ssh_allow_list(&[], &["203.0.113.10/32".to_owned()]),
            vec!["203.0.113.10/32"]
        );
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
    fn ssh_transfer_summary_prefers_cli_then_profile_and_redacts_key_path() {
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
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);

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
    fn ssh_transfer_summary_uses_profile_fallbacks_and_ignores_ros_env() {
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
        assert_eq!(plan.preconditions.ssh.user, "profile-ssh");
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
    fn non_dry_run_plan_error_does_not_claim_runtime_is_unimplemented() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = build_plan_for_env(command, &cli, &isolated_env())
            .expect_err("plan builder should require dry-run");

        assert_eq!(error.error_code, ErrorCode::UsageError);
        assert!(error.message.contains("requires --dry-run"));
        assert!(!error.message.contains("not implemented"));
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
    fn runtime_transfer_requires_host_key_before_connecting() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--user",
            "admin",
            "--password",
            "test-value",
            "file",
            "upload",
            "setup.rsc",
            "flash/setup.rsc",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = handle_transfer_for_env(command, &cli, &isolated_env())
            .expect_err("host key should be required");

        assert_eq!(error.error_code, ErrorCode::SshHostKeyRequired);
        assert_eq!(error.context.command, "file/upload");
    }

    #[test]
    fn runtime_import_requires_host_key_before_connecting() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--user",
            "admin",
            "--password",
            "test-value",
            "import",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = handle_transfer_for_env(command, &cli, &isolated_env())
            .expect_err("host key should be required before import workflow connects");

        assert_eq!(error.error_code, ErrorCode::SshHostKeyRequired);
        assert_eq!(error.context.command, "import");
    }

    #[test]
    fn runtime_transfer_requires_password_when_key_is_absent() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--ssh-user",
            "admin",
            "--ssh-host-key",
            "SHA256:test",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = handle_transfer_for_env(command, &cli, &isolated_env())
            .expect_err("password should be required before SSH connect");

        assert_eq!(error.error_code, ErrorCode::ConfigError);
        assert!(error.message.contains("missing SSH transfer password"));
    }

    #[test]
    fn runtime_ensure_ssh_requires_allow_from_before_connecting() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--user",
            "api-user",
            "--password",
            "api-secret",
            "--ssh-user",
            "ssh-user",
            "--ssh-password",
            "ssh-secret",
            "--ssh-host-key",
            "SHA256:test",
            "file",
            "upload",
            "setup.rsc",
            "flash/setup.rsc",
            "--ensure-ssh",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = handle_transfer_for_env(command, &cli, &isolated_env())
            .expect_err("ensure-ssh should require allow-from");

        assert_eq!(error.error_code, ErrorCode::SshWhitelistRequired);
        assert!(error.message.contains("--ensure-ssh requires"));
    }

    #[test]
    fn runtime_upload_rejects_large_file_before_connecting() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let local = temp.path().join("large.rsc");
        fs::File::create(&local)
            .expect("file should be created")
            .set_len(MAX_TRANSFER_BYTES + 1)
            .expect("sparse file size should be set");
        let local = local.display().to_string();
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--ssh-user",
            "admin",
            "--ssh-password",
            "test-value",
            "--ssh-host-key",
            "SHA256:test",
            "file",
            "upload",
            &local,
            "flash/large.rsc",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");

        let error = handle_transfer_for_env(command, &cli, &isolated_env())
            .expect_err("large file should fail before SSH connect");

        assert_eq!(error.error_code, ErrorCode::FileTooLarge);
    }

    #[test]
    fn runtime_config_resolves_profile_secret_password() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "api-user"
ssh_user = "ssh-profile"
allow_plain_secrets = true

[profiles.studio.secrets.password]
type = "plain"
value = "profile-secret"

[profiles.studio.secrets.ssh_password]
type = "same-as"
target = "password"
"#,
        );
        let cli = Cli::try_parse_from([
            "roswire",
            "--ssh-host-key",
            "SHA256:test",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);
        let profile = load_selected_profile(&cli, &env)
            .expect("profile should load")
            .expect("profile should exist");

        let runtime = resolve_ssh_runtime_config(&cli, &env, Some(&profile))
            .expect("runtime config should resolve");

        assert_eq!(runtime.host, "198.51.100.10");
        assert_eq!(runtime.user, "ssh-profile");
        assert_eq!(runtime.password.as_deref(), Some("profile-secret"));
        assert_eq!(runtime.expected_host_key, "SHA256:test");
    }

    #[test]
    fn runtime_config_uses_key_auth_without_password() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--user",
            "api-user",
            "--ssh-host-key",
            "SHA256:test",
            "--ssh-key",
            "/Users/example/.ssh/id_ed25519",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");

        let runtime = resolve_ssh_runtime_config(&cli, &isolated_env(), None)
            .expect("runtime config should resolve");

        assert_eq!(runtime.host, "198.51.100.10");
        assert_eq!(runtime.user, "api-user");
        assert_eq!(runtime.password, None);
        assert_eq!(
            runtime.key_path.as_deref(),
            Some("/Users/example/.ssh/id_ed25519")
        );
        assert_eq!(runtime.expected_host_key, "SHA256:test");
    }

    #[test]
    fn runtime_config_ignores_ros_key_passphrase_env_without_leaking_to_summary() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--ssh-user",
            "admin",
            "--ssh-host-key",
            "SHA256:test",
            "--ssh-key",
            "/Users/example/.ssh/id_ed25519",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let env = BTreeMap::from([(
            "ROS_SSH_KEY_PASSPHRASE".to_owned(),
            "phrase-secret".to_owned(),
        )]);

        let summary =
            resolve_ssh_transfer_summary(&cli, &env, None).expect("summary should resolve");
        let runtime =
            resolve_ssh_runtime_config(&cli, &env, None).expect("runtime config should resolve");

        assert_eq!(summary.auth_method, "key");
        assert_eq!(summary.key_passphrase, "not-provided");
        assert_eq!(summary.data_plane, "sftp-with-scp-fallback");
        assert_eq!(runtime.key_passphrase, None);
    }

    #[test]
    fn runtime_config_resolves_key_passphrase_from_profile_secret() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "api-user"
ssh_user = "ssh-profile"
ssh_key = "/Users/profile/.ssh/id_profile"
allow_plain_secrets = true

[profiles.studio.secrets.ssh_key_passphrase]
type = "plain"
value = "profile-phrase"
"#,
        );
        let cli = Cli::try_parse_from([
            "roswire",
            "--ssh-host-key",
            "SHA256:test",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);
        let profile = load_selected_profile(&cli, &env)
            .expect("profile should load")
            .expect("profile should exist");

        let passphrase = resolve_ssh_key_passphrase(&env, Some(&profile))
            .expect("passphrase secret should resolve");
        let runtime = resolve_ssh_runtime_config(&cli, &env, Some(&profile))
            .expect("runtime config should resolve");

        assert_eq!(passphrase.as_deref(), Some("profile-phrase"));
        assert_eq!(runtime.key_passphrase.as_deref(), Some("profile-phrase"));
        assert_eq!(runtime.password, None);
    }

    #[test]
    fn sftp_or_scp_fallback_returns_sftp_result_without_scp_attempt() {
        let context = workflow_context("file/download");

        let result = sftp_or_scp_fallback(
            "download",
            Ok((5, "sftp-sha".to_owned())),
            || panic!("SCP fallback should not run after SFTP success"),
            &context,
        )
        .expect("SFTP result should be returned");

        assert_eq!(result, (5, "sftp-sha".to_owned()));
    }

    #[test]
    fn sftp_or_scp_fallback_prefers_scp_when_sftp_is_unavailable() {
        let context = workflow_context("file/download");
        let sftp_error = Box::new(
            crate::error::RosWireError::file_transfer_failed("SFTP subsystem is unavailable")
                .with_context(context.clone()),
        );

        let result = sftp_or_scp_fallback(
            "download",
            Err(sftp_error),
            || Ok((7, "scp-sha".to_owned())),
            &context,
        )
        .expect("SCP fallback should succeed");

        assert_eq!(result, (7, "scp-sha".to_owned()));
    }

    #[test]
    fn sftp_or_scp_fallback_combines_errors_when_both_are_unavailable() {
        let context = workflow_context("file/upload");
        let sftp_error = Box::new(
            crate::error::RosWireError::file_transfer_failed("SFTP subsystem is unavailable")
                .with_context(context.clone()),
        );

        let error = sftp_or_scp_fallback::<(u64, String), _>(
            "upload",
            Err(sftp_error),
            || {
                Err(Box::new(
                    crate::error::RosWireError::file_transfer_failed(
                        "SCP subsystem rejected channel",
                    )
                    .with_context(context.clone()),
                ))
            },
            &context,
        )
        .expect_err("combined fallback failure should be surfaced");

        assert_eq!(error.error_code, ErrorCode::FileTransferFailed);
        assert!(error.message.contains("SFTP upload is unavailable"));
        assert!(error.message.contains("SCP fallback failed"));
        assert!(error.message.contains("SCP subsystem rejected channel"));
    }

    #[test]
    fn transfer_backend_and_port_validation_are_structured() {
        let cli = Cli::try_parse_from(["roswire", "--transfer", "ssh", "file", "upload", "a", "b"])
            .expect("cli should parse");

        assert_eq!(
            resolve_transfer_backend(&cli, None).expect("ssh backend should resolve"),
            "ssh"
        );
        assert!(parse_port("not-a-port").is_err());
    }

    #[test]
    fn host_key_fingerprint_uses_routeros_sha256_format() {
        let fingerprint = sha256_fingerprint(b"12345678901234567890123456789012");

        assert!(fingerprint.starts_with("SHA256:"));
        assert!(host_key_matches(&fingerprint, &fingerprint));
        assert!(!host_key_matches("SHA256:wrong", &fingerprint));
    }

    #[test]
    fn copy_with_sha256_counts_bytes_and_hashes_content() {
        let mut reader = Cursor::new(b"routeros".to_vec());
        let mut writer = Vec::new();
        let context = ErrorContext::default();

        let (bytes, checksum) =
            copy_with_sha256(&mut reader, &mut writer, &context).expect("copy should work");

        assert_eq!(bytes, 8);
        assert_eq!(writer, b"routeros");
        assert_eq!(
            checksum,
            "777bb2ce0ca8318c55b28e4a9e676387cdafa753116b979531a1f71832c7a00b",
        );
    }

    #[test]
    fn cidr_validation_accepts_narrow_client_ranges() {
        validate_safe_cidr("203.0.113.10/32").expect("single IPv4 host should be safe");
        validate_safe_cidr("2001:db8::1/128").expect("single IPv6 host should be safe");
    }

    #[test]
    fn transfer_policy_parses_if_exists_timeout_and_retry_options() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/config.rsc",
            "config.rsc",
            "--if-exists",
            "skip",
            "--connect-timeout-seconds",
            "3",
            "--wait-timeout-seconds",
            "4",
            "--transfer-timeout-seconds",
            "5",
            "--cleanup-timeout-seconds",
            "6",
            "--retries",
            "2",
            "--retry-delay-seconds",
            "0",
        ])
        .expect("cli should parse");

        let policy = transfer_policy(&cli).expect("policy should parse");

        assert_eq!(policy.if_exists, TransferIfExists::Skip);
        assert_eq!(policy.timeouts.connect_seconds, 3);
        assert_eq!(policy.timeouts.wait_remote_file_seconds, 4);
        assert_eq!(policy.timeouts.transfer_seconds, 5);
        assert_eq!(policy.timeouts.cleanup_seconds, 6);
        assert_eq!(policy.retry.max_retries, 2);
    }

    #[test]
    fn transfer_policy_rejects_unbounded_retry_counts() {
        let cli = Cli::try_parse_from([
            "roswire",
            "file",
            "download",
            "flash/config.rsc",
            "config.rsc",
            "--retries",
            "99",
        ])
        .expect("cli should parse");

        let error = transfer_policy(&cli).expect_err("too many retries should fail");

        assert_eq!(error.error_code, ErrorCode::UsageError);
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

    #[test]
    fn import_workflow_uploads_temp_file_imports_and_cleans() {
        let cli = Cli::try_parse_from([
            "roswire",
            "import",
            "/Users/example/setup.rsc",
            "--remote-path",
            "flash/setup.rsc",
            "--cleanup",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let payload = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("import"),
        )
        .expect("workflow should succeed");

        assert_eq!(payload.operation, "import.plan");
        assert_eq!(payload.bytes, 12);
        assert_eq!(payload.checksum_sha256, "upload-sha");
        assert_eq!(
            payload.paths.local_path.as_deref(),
            Some("***REDACTED***/setup.rsc")
        );
        assert_eq!(
            payload.paths.temporary_remote_path.as_deref(),
            Some("flash/setup.rsc.roswire.tmp")
        );
        assert_eq!(
            backend.events,
            vec![
                "upload:/Users/example/setup.rsc->flash/setup.rsc.roswire.tmp",
                "control:/import =file-name=flash/setup.rsc.roswire.tmp",
                "remove:flash/setup.rsc.roswire.tmp",
            ]
        );
    }

    #[test]
    fn workflow_ensure_ssh_merges_allow_from_and_restore_snapshot() {
        let cli = Cli::try_parse_from([
            "roswire",
            "import",
            "/Users/example/setup.rsc",
            "--ensure-ssh",
            "--restore-ssh",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            ssh_snapshot: SshServiceSnapshot {
                id: Some("*A".to_owned()),
                disabled: true,
                address: vec!["198.51.100.4/32".to_owned()],
            },
            ..FakeWorkflowBackend::default()
        };

        let payload = execute_file_workflow(
            &command,
            &cli,
            &["203.0.113.10/32".to_owned()],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("import"),
        )
        .expect("workflow should succeed and restore SSH service");

        assert_eq!(payload.operation, "import.plan");
        assert_eq!(
            backend.events,
            vec![
                "snapshot-ssh",
                "set-ssh:disabled=no address=198.51.100.4/32,203.0.113.10/32",
                "upload:/Users/example/setup.rsc->flash/roswire-import-setup.rsc.roswire.tmp",
                "control:/import =file-name=flash/roswire-import-setup.rsc.roswire.tmp",
                "set-ssh:disabled=yes address=198.51.100.4/32",
            ]
        );
    }

    #[test]
    fn workflow_restore_failure_returns_structured_restore_error() {
        let cli = Cli::try_parse_from(["roswire", "import", "setup.rsc", "--restore-ssh"])
            .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            fail_ssh_apply_on: Some(1),
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("import"),
        )
        .expect_err("restore failure should be surfaced");

        assert_eq!(error.error_code, ErrorCode::SshRestoreFailed);
        assert!(error.message.contains("after successful transfer"));
    }

    #[test]
    fn workflow_operation_failure_still_attempts_restore() {
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            "backup.backup",
            "--ensure-ssh",
            "--restore-ssh",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            fail_wait: true,
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &["203.0.113.10/32".to_owned()],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect_err("workflow should return the original operation error after restore succeeds");

        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
        assert!(backend
            .events
            .iter()
            .any(|event| event == "set-ssh:disabled=no address=203.0.113.10/32"));
    }

    #[test]
    fn backup_workflow_generates_waits_downloads_finalizes_and_cleans() {
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            "/Users/example/pre-change.backup",
            "--name",
            "pre-change",
            "--cleanup",
        ])
        .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let payload = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect("workflow should succeed");

        assert_eq!(payload.operation, "backup.download");
        assert_eq!(payload.bytes, 24);
        assert_eq!(
            payload.paths.temporary_local_path.as_deref(),
            Some("***REDACTED***/pre-change.backup.part")
        );
        assert_eq!(
            backend.events,
            vec![
                "control:/system/backup/save =name=pre-change",
                "wait:pre-change.backup",
                "download:pre-change.backup->/Users/example/pre-change.backup.part",
                "finalize:/Users/example/pre-change.backup.part->/Users/example/pre-change.backup",
                "remove:pre-change.backup",
            ]
        );
    }

    #[test]
    fn generated_download_if_exists_fail_stops_before_side_effects() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let local = temp_dir.path().join("pre-change.backup");
        fs::write(&local, b"existing").expect("existing target should be written");
        let local = local.display().to_string();
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            &local,
            "--if-exists",
            "fail",
        ])
        .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect_err("existing local target should fail before side effects");

        assert_eq!(error.error_code, ErrorCode::FileTransferFailed);
        assert!(backend.events.is_empty());
    }

    #[test]
    fn generated_download_if_exists_skip_returns_skipped_payload() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let local = temp_dir.path().join("pre-change.backup");
        fs::write(&local, b"existing").expect("existing target should be written");
        let local = local.display().to_string();
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            &local,
            "--if-exists",
            "skip",
        ])
        .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let payload = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect("existing local target should be skipped");

        assert_eq!(payload.status, "skipped");
        assert_eq!(payload.bytes, 0);
        assert!(backend.events.is_empty());
    }

    #[test]
    fn generated_download_cleans_temp_part_on_finalize_failure() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let local = temp_dir.path().join("config.backup").display().to_string();
        let part = raw_temporary_local_path(&local);
        let cli = Cli::try_parse_from(["roswire", "backup", "download", &local])
            .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            materialize_download: true,
            fail_finalize: true,
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect_err("finalize failure should propagate");

        assert_eq!(error.error_code, ErrorCode::FileTransferFailed);
        assert!(error.message.contains("finalize"));
        assert!(
            !Path::new(&part).exists(),
            "temporary .part file must be cleaned up on failure: {part}",
        );
        assert!(
            backend
                .events
                .iter()
                .any(|event| event.starts_with("remove-local:")),
            "best-effort local cleanup should be attempted: {:?}",
            backend.events,
        );
    }

    #[test]
    fn generated_download_stale_part_respects_if_exists_fail() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let local = temp_dir.path().join("config.backup").display().to_string();
        let part = raw_temporary_local_path(&local);
        fs::write(&part, b"stale").expect("stale part should be written");
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            &local,
            "--if-exists",
            "fail",
        ])
        .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect_err("stale .part must trip --if-exists=fail before side effects");

        assert_eq!(error.error_code, ErrorCode::FileTransferFailed);
        assert!(
            backend.events.is_empty(),
            "no transfer side effects should run when destination check fails: {:?}",
            backend.events,
        );
    }

    #[test]
    fn generated_download_wait_is_not_retried_by_transfer_policy() {
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            "backup.backup",
            "--retries",
            "3",
            "--retry-delay-seconds",
            "0",
        ])
        .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            fail_wait: true,
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect_err("wait timeout should propagate without retry amplification");

        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
        assert_eq!(
            backend
                .events
                .iter()
                .filter(|event| event.as_str() == "wait:roswire-backup.backup")
                .count(),
            1,
            "wait already self-loops to timeout; it must not be retried: {:?}",
            backend.events,
        );
    }

    #[test]
    fn generated_download_wait_runs_once_on_success() {
        let cli = Cli::try_parse_from(["roswire", "backup", "download", "backup.backup"])
            .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let payload = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect("download should succeed");

        assert_eq!(payload.status, "ok");
        assert_eq!(
            backend
                .events
                .iter()
                .filter(|event| event.as_str() == "wait:roswire-backup.backup")
                .count(),
            1,
        );
    }

    #[test]
    fn generated_download_retries_transient_download_failure() {
        let cli = Cli::try_parse_from([
            "roswire",
            "backup",
            "download",
            "backup.backup",
            "--retries",
            "1",
            "--retry-delay-seconds",
            "0",
        ])
        .expect("cli should parse");
        let policy = transfer_policy(&cli).expect("policy should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            download_failures_remaining: 1,
            ..FakeWorkflowBackend::default()
        };

        let payload = execute_file_workflow(
            &command,
            &cli,
            &[],
            &policy,
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect("transient download failure should be retried");

        assert_eq!(payload.status, "ok");
        assert_eq!(
            backend
                .events
                .iter()
                .filter(|event| event.starts_with("download:"))
                .count(),
            2,
            "download should be retried once: {:?}",
            backend.events,
        );
        assert_eq!(
            backend
                .events
                .iter()
                .filter(|event| event.as_str() == "wait:roswire-backup.backup")
                .count(),
            1,
        );
    }

    #[test]
    fn export_workflow_supports_compact_control_command() {
        let cli = Cli::try_parse_from(["roswire", "export", "download", "config.rsc", "--compact"])
            .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend::default();

        let payload = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("export/download"),
        )
        .expect("workflow should succeed");

        assert_eq!(payload.operation, "export.download");
        assert_eq!(
            payload.paths.remote_path.as_deref(),
            Some("roswire-export.rsc")
        );
        assert!(backend
            .events
            .iter()
            .any(|event| event == "control:/export =file=roswire-export =compact=yes"));
    }

    #[test]
    fn workflow_wait_timeout_is_structured() {
        let cli = Cli::try_parse_from(["roswire", "backup", "download", "backup.backup"])
            .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            fail_wait: true,
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect_err("missing generated file should fail");

        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
        assert!(error.message.contains("timed out waiting"));
        assert_eq!(error.context.command, "backup/download");
    }

    #[test]
    fn cleanup_failure_is_not_ignored() {
        let cli = Cli::try_parse_from(["roswire", "import", "setup.rsc", "--cleanup"])
            .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            fail_remove: true,
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("import"),
        )
        .expect_err("cleanup failure should fail the workflow");

        assert_eq!(error.error_code, ErrorCode::FileTransferFailed);
        assert!(error.message.contains("cleanup failed"));
    }

    #[test]
    fn import_control_error_survives_cleanup_failure() {
        let cli = Cli::try_parse_from(["roswire", "import", "setup.rsc", "--cleanup"])
            .expect("cli should parse");
        let command = parse_transfer_command(&cli.tokens)
            .expect("transfer command should be detected")
            .expect("transfer command should parse");
        let mut backend = FakeWorkflowBackend {
            fail_control: true,
            fail_remove: true,
            ..FakeWorkflowBackend::default()
        };

        let error = execute_file_workflow(
            &command,
            &cli,
            &[],
            &default_transfer_policy(),
            &mut backend,
            &workflow_context("import"),
        )
        .expect_err("import control failure should propagate");

        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
        assert!(error.message.contains("import control command failed"));
        assert!(
            !error.message.contains("cleanup failed"),
            "cleanup failure must not mask the original control error: {}",
            error.message,
        );
        assert!(
            backend
                .events
                .iter()
                .any(|event| event.starts_with("remove:")),
            "best-effort cleanup should still be attempted: {:?}",
            backend.events,
        );
    }

    #[test]
    fn control_runtime_uses_api_credentials_separately_from_ssh_credentials() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--user",
            "api-user",
            "--password",
            "api-secret",
            "--ssh-user",
            "ssh-user",
            "--ssh-password",
            "ssh-secret",
            "--protocol",
            "api-ssl",
            "export",
            "download",
            "config.rsc",
        ])
        .expect("cli should parse");

        let runtime = resolve_control_runtime_config(&cli, &isolated_env(), None)
            .expect("control runtime should resolve");

        assert_eq!(runtime.user, "api-user");
        assert_eq!(runtime.password, "api-secret");
        assert_eq!(runtime.selected_protocol, "api-ssl");
        assert_eq!(runtime.port, 8729);
    }

    #[test]
    fn control_runtime_resolves_profile_and_ignores_ros_env() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "profile-api"
protocol = "rest"
allow_plain_secrets = true

[profiles.studio.secrets.password]
type = "plain"
value = "profile-secret"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "export", "download", "config.rsc"])
            .expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);

        let profile = load_selected_profile(&cli, &env)
            .expect("profile should load")
            .expect("profile should exist");
        let profile_runtime = resolve_control_runtime_config(&cli, &env, Some(&profile))
            .expect("profile runtime should resolve");

        assert_eq!(profile_runtime.host, "198.51.100.10");
        assert_eq!(profile_runtime.user, "profile-api");
        assert_eq!(profile_runtime.password, "profile-secret");
        assert_eq!(profile_runtime.selected_protocol, "rest");
        assert_eq!(profile_runtime.port, 443);

        let auto_with_port = Cli::try_parse_from([
            "roswire",
            "--host",
            "198.51.100.10",
            "--user",
            "api-user",
            "--password",
            "api-secret",
            "--port",
            "8728",
            "export",
            "download",
            "config.rsc",
        ])
        .expect("cli should parse");
        assert_eq!(
            resolve_control_runtime_config(&auto_with_port, &isolated_env(), None)
                .expect_err("auto + port should fail")
                .error_code,
            ErrorCode::ConfigError,
        );

        assert_eq!(
            resolve_control_runtime_config(&cli, &isolated_env(), None)
                .expect_err("missing host should fail")
                .error_code,
            ErrorCode::ConfigError,
        );
        assert_eq!(
            resolve_control_runtime_config(
                &cli,
                &BTreeMap::from([
                    ("ROS_HOST".to_owned(), "198.51.100.10".to_owned()),
                    ("ROS_USER".to_owned(), "env-api".to_owned()),
                    ("ROS_PASSWORD".to_owned(), "env-secret".to_owned()),
                    ("ROS_PROTOCOL".to_owned(), "bogus".to_owned()),
                ]),
                None,
            )
            .expect_err("ROS_* control env should be ignored")
            .error_code,
            ErrorCode::ConfigError,
        );
    }

    #[test]
    fn control_commands_have_classic_and_rest_shapes() {
        let import = ControlCommand::Import {
            file_name: "flash/setup.rsc.roswire.tmp".to_owned(),
        };
        let backup = ControlCommand::BackupSave {
            name: "pre-change".to_owned(),
        };
        let export = ControlCommand::Export {
            file: "roswire-export".to_owned(),
            compact: true,
        };

        assert_eq!(
            import.classic_words(),
            vec!["/import", "=file-name=flash/setup.rsc.roswire.tmp"]
        );
        assert_eq!(backup.rest_request().0, "/rest/system/backup/save");
        assert_eq!(backup.rest_request().1["name"], "pre-change");
        assert_eq!(export.rest_request().0, "/rest/export");
        assert_eq!(export.rest_request().1["compact"], "yes");
    }

    #[test]
    fn classic_control_logs_in_and_executes_words() {
        let (stream, tx) = SharedFakeApiStream::with_sentences(&[
            vec!["!done".to_owned()],
            vec!["!done".to_owned()],
        ]);
        let control = ControlRuntimeConfig {
            host: "198.51.100.10".to_owned(),
            port: 8728,
            user: "admin".to_owned(),
            password: "test-value".to_owned(),
            selected_protocol: "api".to_owned(),
            tls_cert_fingerprint: None,
            jump: Vec::new(),
        };

        execute_classic_control(
            stream,
            &ControlCommand::BackupSave {
                name: "pre-change".to_owned(),
            },
            &control,
            selected_context(&workflow_context("backup/download"), "api"),
        )
        .expect("classic control should execute");

        let sentences = written_sentences(&tx);
        assert_eq!(sentences[0][0], "/login");
        assert_eq!(
            sentences[1],
            vec!["/system/backup/save", "=name=pre-change"]
        );
    }

    #[test]
    fn live_backend_pre_network_paths_are_structured() {
        let context = workflow_context("export/download");
        let mut api = live_backend("api");
        let error = api
            .execute_control(
                &ControlCommand::Export {
                    file: "roswire-export".to_owned(),
                    compact: false,
                },
                &context,
            )
            .expect_err("port zero should fail before RouterOS side effects");
        assert_eq!(error.error_code, ErrorCode::NetworkError);
        assert_eq!(error.context.selected_protocol, "api");

        let mut api_ssl = live_backend("api-ssl");
        assert_eq!(
            api_ssl
                .execute_control(
                    &ControlCommand::BackupSave {
                        name: "pre-change".to_owned(),
                    },
                    &context,
                )
                .expect_err("port zero should fail")
                .context
                .selected_protocol,
            "api-ssl",
        );

        let mut rest = live_backend("rest");
        assert_eq!(
            rest.execute_control(
                &ControlCommand::Import {
                    file_name: "flash/setup.rsc".to_owned(),
                },
                &context,
            )
            .expect_err("port zero should fail")
            .context
            .selected_protocol,
            "rest",
        );

        let mut wait = live_backend("api");
        assert_eq!(
            wait.wait_remote_file("flash/missing.rsc", Duration::from_millis(0), &context)
                .expect_err("SSH port zero should fail")
                .error_code,
            ErrorCode::NetworkError,
        );
        assert_eq!(
            wait.remove_remote_file("flash/missing.rsc", &context)
                .expect_err("SSH port zero should fail")
                .error_code,
            ErrorCode::NetworkError,
        );
    }

    #[test]
    fn live_backend_upload_and_finalize_cover_pre_network_file_paths() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let large = temp.path().join("large.rsc");
        fs::File::create(&large)
            .expect("file should be created")
            .set_len(MAX_TRANSFER_BYTES + 1)
            .expect("sparse file size should be set");
        let context = workflow_context("file/upload");
        let mut backend = live_backend("api");

        let error = backend
            .upload(&large.display().to_string(), "flash/large.rsc", &context)
            .expect_err("large file should fail before SSH connect");
        assert_eq!(error.error_code, ErrorCode::FileTooLarge);

        let temporary = temp.path().join("download.rsc.part");
        let final_path = temp.path().join("download.rsc");
        fs::write(&temporary, "export").expect("temp file should be written");
        backend
            .finalize_local_download(
                &temporary.display().to_string(),
                &final_path.display().to_string(),
                &context,
            )
            .expect("finalize should rename");
        assert_eq!(
            fs::read_to_string(final_path).expect("final file should read"),
            "export"
        );
        assert_eq!(
            backend
                .finalize_local_download(
                    &temporary.display().to_string(),
                    &temp.path().join("missing.rsc").display().to_string(),
                    &context,
                )
                .expect_err("missing temp should fail")
                .error_code,
            ErrorCode::FileTransferFailed,
        );
    }

    #[derive(Default)]
    struct FakeWorkflowBackend {
        events: Vec<String>,
        fail_wait: bool,
        wait_failures_remaining: usize,
        fail_remove: bool,
        fail_control: bool,
        fail_download: bool,
        fail_finalize: bool,
        materialize_download: bool,
        download_failures_remaining: usize,
        ssh_snapshot: SshServiceSnapshot,
        ssh_apply_count: usize,
        fail_ssh_apply_on: Option<usize>,
    }

    impl WorkflowBackend for FakeWorkflowBackend {
        fn read_ssh_service(
            &mut self,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<SshServiceSnapshot> {
            self.events.push("snapshot-ssh".to_owned());
            Ok(self.ssh_snapshot.clone())
        }

        fn apply_ssh_service(
            &mut self,
            desired: &SshServiceSnapshot,
            context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.ssh_apply_count += 1;
            self.events.push(format!(
                "set-ssh:disabled={} address={}",
                routeros_bool(desired.disabled),
                desired.address.join(",")
            ));
            if self.fail_ssh_apply_on == Some(self.ssh_apply_count) {
                return Err(Box::new(
                    crate::error::RosWireError::ros_api_failure("failed to set ssh service")
                        .with_context(context.clone()),
                ));
            }
            Ok(())
        }

        fn upload(
            &mut self,
            local: &str,
            remote: &str,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<(u64, String)> {
            self.events.push(format!("upload:{local}->{remote}"));
            Ok((12, "upload-sha".to_owned()))
        }

        fn download(
            &mut self,
            remote: &str,
            local: &str,
            context: &ErrorContext,
        ) -> crate::error::RosWireResult<(u64, String)> {
            self.events.push(format!("download:{remote}->{local}"));
            if self.materialize_download {
                fs::write(local, b"partial-download").expect("part file should be writable");
            }
            if self.download_failures_remaining > 0 {
                self.download_failures_remaining -= 1;
                return Err(Box::new(
                    crate::error::RosWireError::file_transfer_failed(format!(
                        "transient download failure for {remote}"
                    ))
                    .with_context(context.clone()),
                ));
            }
            if self.fail_download {
                return Err(Box::new(
                    crate::error::RosWireError::file_transfer_failed(format!(
                        "download failed for {remote}"
                    ))
                    .with_context(context.clone()),
                ));
            }
            Ok((24, "download-sha".to_owned()))
        }

        fn finalize_local_download(
            &mut self,
            temporary_local: &str,
            local: &str,
            context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events
                .push(format!("finalize:{temporary_local}->{local}"));
            if self.fail_finalize {
                return Err(Box::new(
                    crate::error::RosWireError::file_transfer_failed(format!(
                        "failed to finalize local download: {local}"
                    ))
                    .with_context(context.clone()),
                ));
            }
            Ok(())
        }

        fn execute_control(
            &mut self,
            command: &ControlCommand,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events
                .push(format!("control:{}", command.classic_words().join(" ")));
            if self.fail_control {
                return Err(Box::new(
                    crate::error::RosWireError::ros_api_failure("import control command failed")
                        .with_context(_context.clone()),
                ));
            }
            Ok(())
        }

        fn wait_remote_file(
            &mut self,
            remote: &str,
            _timeout: Duration,
            context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events.push(format!("wait:{remote}"));
            if self.wait_failures_remaining > 0 {
                self.wait_failures_remaining -= 1;
                return Err(Box::new(
                    crate::error::RosWireError::ros_api_failure(format!(
                        "timed out waiting for remote file: {remote}"
                    ))
                    .with_context(context.clone()),
                ));
            }
            if self.fail_wait {
                return Err(Box::new(
                    crate::error::RosWireError::ros_api_failure(format!(
                        "timed out waiting for remote file: {remote}"
                    ))
                    .with_context(context.clone()),
                ));
            }
            Ok(())
        }

        fn remove_remote_file(
            &mut self,
            remote: &str,
            context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events.push(format!("remove:{remote}"));
            if self.fail_remove {
                return Err(Box::new(
                    crate::error::RosWireError::file_transfer_failed(format!(
                        "cleanup failed for {remote}"
                    ))
                    .with_context(context.clone()),
                ));
            }
            Ok(())
        }

        fn remove_local_file(
            &mut self,
            local: &str,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events.push(format!("remove-local:{local}"));
            let _ = fs::remove_file(local);
            Ok(())
        }
    }

    struct SharedFakeApiStream {
        rx: Cursor<Vec<u8>>,
        tx: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedFakeApiStream {
        fn with_sentences(sentences: &[Vec<String>]) -> (Self, Arc<Mutex<Vec<u8>>>) {
            let mut rx = Vec::new();
            for sentence in sentences {
                write_sentence(&mut rx, sentence).expect("fixture sentence should encode");
            }
            let tx = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    rx: Cursor::new(rx),
                    tx: Arc::clone(&tx),
                },
                tx,
            )
        }
    }

    impl Read for SharedFakeApiStream {
        fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
            self.rx.read(buffer)
        }
    }

    impl Write for SharedFakeApiStream {
        fn write(&mut self, buffer: &[u8]) -> IoResult<usize> {
            self.tx
                .lock()
                .expect("fake tx lock should not be poisoned")
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    fn written_sentences(tx: &Arc<Mutex<Vec<u8>>>) -> Vec<Vec<String>> {
        let bytes = tx
            .lock()
            .expect("fake tx lock should not be poisoned")
            .clone();
        let mut cursor = Cursor::new(bytes);
        let mut sentences = Vec::new();
        while (cursor.position() as usize) < cursor.get_ref().len() {
            sentences.push(read_sentence(&mut cursor).expect("written sentence should decode"));
        }
        sentences
    }

    fn live_backend(selected_protocol: &str) -> LiveWorkflowBackend {
        LiveWorkflowBackend::new(
            SshRuntimeConfig {
                host: "127.0.0.1".to_owned(),
                port: 0,
                user: "ssh-user".to_owned(),
                password: Some("ssh-secret".to_owned()),
                key_path: None,
                key_passphrase: None,
                expected_host_key: "SHA256:test".to_owned(),
                jump: Vec::new(),
            },
            ControlRuntimeConfig {
                host: "127.0.0.1".to_owned(),
                port: 0,
                user: "api-user".to_owned(),
                password: "api-secret".to_owned(),
                selected_protocol: selected_protocol.to_owned(),
                tls_cert_fingerprint: None,
                jump: Vec::new(),
            },
            default_transfer_policy(),
        )
    }

    fn workflow_context(command: &str) -> ErrorContext {
        ErrorContext {
            command: command.to_owned(),
            path: command.split('/').map(str::to_owned).collect(),
            action: command.to_owned(),
            requested_protocol: "auto".to_owned(),
            selected_protocol: "unknown".to_owned(),
            transfer_backend: Some("ssh".to_owned()),
            routeros_version: "auto".to_owned(),
            host: "198.51.100.10".to_owned(),
            jump: Vec::new(),
            resolved_args: BTreeMap::new(),
        }
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
