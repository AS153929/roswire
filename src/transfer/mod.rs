use crate::args::Cli;
use crate::config;
use crate::error::{self, ErrorContext, RosWireError, RosWireResult};
use crate::protocol::classic::{
    transport::{ApiStream, TcpApiStream, TlsApiStream},
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

const PLAN_SCHEMA_VERSION: &str = "roswire.transfer.plan.v1";
const DEFAULT_TRANSFER_BACKEND: &str = "ssh";
const RESULT_SCHEMA_VERSION: &str = "roswire.transfer.result.v1";
const MAX_TRANSFER_BYTES: u64 = 64 * 1024 * 1024;
const WORKFLOW_FILE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const WORKFLOW_FILE_WAIT_INTERVAL: Duration = Duration::from_secs(1);

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransferResultPayload {
    pub schema_version: &'static str,
    pub operation: String,
    pub transfer_backend: String,
    pub status: &'static str,
    pub bytes: u64,
    pub checksum_sha256: String,
    pub paths: TransferPaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SshRuntimeConfig {
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    key_path: Option<String>,
    expected_host_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlRuntimeConfig {
    host: String,
    port: u16,
    user: String,
    password: String,
    selected_protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlCommand {
    Import { file_name: String },
    BackupSave { name: String },
    Export { file: String, compact: bool },
}

impl ControlCommand {
    fn classic_words(&self) -> Vec<String> {
        match self {
            Self::Import { file_name } => {
                vec!["/import".to_owned(), format!("=file-name={file_name}")]
            }
            Self::BackupSave { name } => {
                vec!["/system/backup/save".to_owned(), format!("=name={name}")]
            }
            Self::Export { file, compact } => {
                let mut words = vec!["/export".to_owned(), format!("=file={file}")];
                if *compact {
                    words.push("=compact=yes".to_owned());
                }
                words
            }
        }
    }

    fn rest_request(&self) -> (&'static str, Value) {
        match self {
            Self::Import { file_name } => ("/rest/import", json!({ "file-name": file_name })),
            Self::BackupSave { name } => ("/rest/system/backup/save", json!({ "name": name })),
            Self::Export { file, compact } => {
                let mut body =
                    serde_json::Map::from_iter([("file".to_owned(), Value::String(file.clone()))]);
                if *compact {
                    body.insert("compact".to_owned(), Value::String("yes".to_owned()));
                }
                ("/rest/export", Value::Object(body))
            }
        }
    }
}

trait WorkflowBackend {
    fn upload(
        &mut self,
        local: &str,
        remote: &str,
        context: &ErrorContext,
    ) -> RosWireResult<(u64, String)>;
    fn download(
        &mut self,
        remote: &str,
        local: &str,
        context: &ErrorContext,
    ) -> RosWireResult<(u64, String)>;
    fn finalize_local_download(
        &mut self,
        temporary_local: &str,
        local: &str,
        context: &ErrorContext,
    ) -> RosWireResult<()>;
    fn execute_control(
        &mut self,
        command: &ControlCommand,
        context: &ErrorContext,
    ) -> RosWireResult<()>;
    fn wait_remote_file(
        &mut self,
        remote: &str,
        timeout: Duration,
        context: &ErrorContext,
    ) -> RosWireResult<()>;
    fn remove_remote_file(&mut self, remote: &str, context: &ErrorContext) -> RosWireResult<()>;
}

struct LiveWorkflowBackend {
    ssh: SshRuntimeConfig,
    control: ControlRuntimeConfig,
}

impl LiveWorkflowBackend {
    fn new(ssh: SshRuntimeConfig, control: ControlRuntimeConfig) -> Self {
        Self { ssh, control }
    }
}

impl WorkflowBackend for LiveWorkflowBackend {
    fn upload(
        &mut self,
        local: &str,
        remote: &str,
        context: &ErrorContext,
    ) -> RosWireResult<(u64, String)> {
        execute_upload(local, remote, &self.ssh, context)
    }

    fn download(
        &mut self,
        remote: &str,
        local: &str,
        context: &ErrorContext,
    ) -> RosWireResult<(u64, String)> {
        execute_download(remote, local, &self.ssh, context)
    }

    fn finalize_local_download(
        &mut self,
        temporary_local: &str,
        local: &str,
        context: &ErrorContext,
    ) -> RosWireResult<()> {
        fs::rename(temporary_local, local).map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!(
                    "failed to finalize local download: {error}"
                ))
                .with_context(context.clone()),
            )
        })
    }

    fn execute_control(
        &mut self,
        command: &ControlCommand,
        context: &ErrorContext,
    ) -> RosWireResult<()> {
        let context = selected_context(context, &self.control.selected_protocol);
        match self.control.selected_protocol.as_str() {
            "rest" => execute_rest_control(command, &self.control, context),
            "api-ssl" => {
                let stream = TlsApiStream::connect(
                    &self.control.host,
                    self.control.port,
                    Duration::from_secs(10),
                )
                .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
                execute_classic_control(stream, command, &self.control, context)
            }
            _ => {
                let stream = TcpApiStream::connect(
                    &self.control.host,
                    self.control.port,
                    Duration::from_secs(10),
                )
                .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
                execute_classic_control(stream, command, &self.control, context)
            }
        }
    }

    fn wait_remote_file(
        &mut self,
        remote: &str,
        timeout: Duration,
        context: &ErrorContext,
    ) -> RosWireResult<()> {
        let session = open_ssh_session(&self.ssh, context)?;
        let sftp = session.sftp().map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!("failed to open SFTP session: {error}"))
                    .with_context(context.clone()),
            )
        })?;
        let deadline = Instant::now() + timeout;
        loop {
            if sftp.stat(Path::new(remote)).is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Box::new(
                    RosWireError::ros_api_failure(format!(
                        "timed out waiting for remote file: {}",
                        redact_remote_path(remote)
                    ))
                    .with_context(context.clone()),
                ));
            }
            thread::sleep(WORKFLOW_FILE_WAIT_INTERVAL);
        }
    }

    fn remove_remote_file(&mut self, remote: &str, context: &ErrorContext) -> RosWireResult<()> {
        let session = open_ssh_session(&self.ssh, context)?;
        let sftp = session.sftp().map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!("failed to open SFTP session: {error}"))
                    .with_context(context.clone()),
            )
        })?;
        sftp.unlink(Path::new(remote)).map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!(
                    "failed to remove remote file: {error}"
                ))
                .with_context(context.clone()),
            )
        })
    }
}

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

fn execute_transfer_for_env(
    command: TransferCommand,
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> RosWireResult<TransferResultPayload> {
    let backend = resolve_transfer_backend(cli, env)?;
    if backend != DEFAULT_TRANSFER_BACKEND {
        return Err(Box::new(RosWireError::usage(format!(
            "unsupported transfer backend: {backend}",
        ))));
    }
    let context = transfer_context(&command, &backend, cli, env);
    let profile = load_selected_profile(cli, env)?;
    let ssh_runtime = resolve_ssh_runtime_config(cli, env, profile.as_ref())
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;

    match &command {
        TransferCommand::FileUpload { local, remote } => {
            execute_upload(local, remote, &ssh_runtime, &context).map(|(bytes, checksum_sha256)| {
                TransferResultPayload {
                    schema_version: RESULT_SCHEMA_VERSION,
                    operation: command.operation().to_owned(),
                    transfer_backend: backend,
                    status: "ok",
                    bytes,
                    checksum_sha256,
                    paths: TransferPaths {
                        local_path: Some(redact_local_path(local)),
                        remote_path: Some(redact_remote_path(remote)),
                        temporary_remote_path: None,
                        temporary_local_path: None,
                    },
                }
            })
        }
        TransferCommand::FileDownload { remote, local } => {
            execute_download(remote, local, &ssh_runtime, &context).map(
                |(bytes, checksum_sha256)| TransferResultPayload {
                    schema_version: RESULT_SCHEMA_VERSION,
                    operation: command.operation().to_owned(),
                    transfer_backend: backend,
                    status: "ok",
                    bytes,
                    checksum_sha256,
                    paths: TransferPaths {
                        local_path: Some(redact_local_path(local)),
                        remote_path: Some(redact_remote_path(remote)),
                        temporary_remote_path: None,
                        temporary_local_path: None,
                    },
                },
            )
        }
        TransferCommand::Import { .. }
        | TransferCommand::BackupDownload { .. }
        | TransferCommand::ExportDownload { .. } => {
            let control_runtime = resolve_control_runtime_config(cli, env, profile.as_ref())
                .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
            let mut backend = LiveWorkflowBackend::new(ssh_runtime, control_runtime);
            execute_file_workflow(&command, cli, &mut backend, &context)
        }
    }
}

fn execute_file_workflow<B: WorkflowBackend>(
    command: &TransferCommand,
    cli: &Cli,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    match command {
        TransferCommand::Import { local } => execute_import_workflow(local, cli, backend, context),
        TransferCommand::BackupDownload { local } => execute_generated_download_workflow(
            command.operation(),
            local,
            generated_backup_name(cli),
            ControlCommand::BackupSave {
                name: generated_backup_base_name(cli),
            },
            cli,
            backend,
            context,
        ),
        TransferCommand::ExportDownload { local } => execute_generated_download_workflow(
            command.operation(),
            local,
            generated_export_name(cli),
            ControlCommand::Export {
                file: generated_export_base_name(cli),
                compact: cli.compact,
            },
            cli,
            backend,
            context,
        ),
        TransferCommand::FileUpload { .. } | TransferCommand::FileDownload { .. } => unreachable!(
            "direct file upload/download workflows are executed before workflow dispatch"
        ),
    }
}

fn execute_import_workflow<B: WorkflowBackend>(
    local: &str,
    cli: &Cli,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    let remote = cli
        .remote_path
        .clone()
        .unwrap_or_else(|| format!("flash/roswire-import-{}", file_name(local)));
    let temporary_remote = temporary_remote_path(&remote);
    let (bytes, checksum_sha256) = backend.upload(local, &temporary_remote, context)?;
    let control = ControlCommand::Import {
        file_name: temporary_remote.clone(),
    };

    if let Err(error) = backend.execute_control(&control, context) {
        if cli.cleanup {
            backend.remove_remote_file(&temporary_remote, context)?;
        }
        return Err(error);
    }
    if cli.cleanup {
        backend.remove_remote_file(&temporary_remote, context)?;
    }

    Ok(TransferResultPayload {
        schema_version: RESULT_SCHEMA_VERSION,
        operation: "import.plan".to_owned(),
        transfer_backend: DEFAULT_TRANSFER_BACKEND.to_owned(),
        status: "ok",
        bytes,
        checksum_sha256,
        paths: TransferPaths {
            local_path: Some(redact_local_path(local)),
            remote_path: Some(redact_remote_path(&remote)),
            temporary_remote_path: Some(redact_remote_path(&temporary_remote)),
            temporary_local_path: None,
        },
    })
}

fn execute_generated_download_workflow<B: WorkflowBackend>(
    operation: &str,
    local: &str,
    remote: String,
    control: ControlCommand,
    cli: &Cli,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    backend.execute_control(&control, context)?;
    backend.wait_remote_file(&remote, WORKFLOW_FILE_WAIT_TIMEOUT, context)?;
    let temporary_local = raw_temporary_local_path(local);
    let (bytes, checksum_sha256) = backend.download(&remote, &temporary_local, context)?;
    backend.finalize_local_download(&temporary_local, local, context)?;
    if cli.cleanup {
        backend.remove_remote_file(&remote, context)?;
    }

    Ok(TransferResultPayload {
        schema_version: RESULT_SCHEMA_VERSION,
        operation: operation.to_owned(),
        transfer_backend: DEFAULT_TRANSFER_BACKEND.to_owned(),
        status: "ok",
        bytes,
        checksum_sha256,
        paths: TransferPaths {
            local_path: Some(redact_local_path(local)),
            remote_path: Some(redact_remote_path(&remote)),
            temporary_remote_path: Some(redact_remote_path(&remote)),
            temporary_local_path: Some(temporary_local_path(local)),
        },
    })
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
            RosWireError::usage(
                "transfer plan generation requires --dry-run; omit --dry-run to execute the SSH transfer runtime",
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

fn resolve_ssh_runtime_config(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<SshRuntimeConfig> {
    let host = cli
        .host
        .clone()
        .or_else(|| env.get("ROS_HOST").cloned())
        .or_else(|| profile.and_then(|profile| profile.host.clone()))
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing SSH transfer host; set --host, ROS_HOST, or profile host",
            ))
        })?;
    config::validate_remote_host(&host)?;

    let summary = resolve_ssh_transfer_summary(cli, env, profile)?;
    if summary.user == "reuse-api-user" {
        return Err(Box::new(RosWireError::config(
            "missing SSH transfer user; set --ssh-user, ROS_SSH_USER, --user, ROS_USER, or profile user",
        )));
    }

    let key_path = cli
        .ssh_key
        .clone()
        .or_else(|| env.get("ROS_SSH_KEY").cloned())
        .or_else(|| profile.and_then(|profile| profile.ssh_key.clone()))
        .filter(|value| !value.trim().is_empty());
    let password = if key_path.is_some() {
        None
    } else {
        Some(resolve_ssh_password(cli, env, profile)?)
    };
    let expected_host_key = cli
        .ssh_host_key
        .clone()
        .or_else(|| env.get("ROS_SSH_HOST_KEY").cloned())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            Box::new(RosWireError::ssh_host_key_required(
                "SSH transfer requires an expected RouterOS SSH host key fingerprint",
            ))
        })?;

    Ok(SshRuntimeConfig {
        host,
        port: summary.port,
        user: summary.user,
        password,
        key_path,
        expected_host_key,
    })
}

fn resolve_ssh_password(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<String> {
    if let Some(password) = cli
        .ssh_password
        .clone()
        .or_else(|| env.get("ROS_SSH_PASSWORD").cloned())
        .or_else(|| cli.password.clone())
        .or_else(|| env.get("ROS_PASSWORD").cloned())
    {
        return Ok(password);
    }

    let Some(profile) = profile else {
        return Err(Box::new(RosWireError::config(
            "missing SSH transfer password; set --ssh-password, ROS_SSH_PASSWORD, --password, ROS_PASSWORD, or profile secret ssh_password/password",
        )));
    };

    config::resolve_profile_secret_value(profile, "ssh_password", env)?
        .or_else(|| config::resolve_profile_secret_value(profile, "password", env).ok().flatten())
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing SSH transfer password; set --ssh-password, ROS_SSH_PASSWORD, or profile secret ssh_password/password",
            ))
        })
}

fn resolve_control_runtime_config(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<ControlRuntimeConfig> {
    let host = cli
        .host
        .clone()
        .or_else(|| env.get("ROS_HOST").cloned())
        .or_else(|| profile.and_then(|profile| profile.host.clone()))
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing RouterOS control host; set --host, ROS_HOST, or profile host",
            ))
        })?;
    config::validate_remote_host(&host)?;

    let user = cli
        .user
        .clone()
        .or_else(|| env.get("ROS_USER").cloned())
        .or_else(|| profile.and_then(|profile| profile.user.clone()))
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing RouterOS control user; set --user, ROS_USER, or profile user",
            ))
        })?;
    let password = resolve_control_password(cli, env, profile)?;
    let requested_protocol = cli
        .protocol
        .map(|value| value.as_str().to_owned())
        .or_else(|| env.get("ROS_PROTOCOL").cloned())
        .or_else(|| profile.and_then(|profile| profile.protocol.clone()))
        .unwrap_or_else(|| "auto".to_owned());
    validate_control_protocol(&requested_protocol)?;

    let env_port = match env.get("ROS_PORT") {
        Some(value) => Some(parse_port(value)?),
        None => None,
    };
    let explicit_port = cli
        .port
        .or(env_port)
        .or_else(|| profile.and_then(|profile| profile.port));
    if requested_protocol == "auto" && explicit_port.is_some() {
        return Err(Box::new(RosWireError::config(
            "port cannot be used with --protocol auto",
        )));
    }
    let selected_protocol = match requested_protocol.as_str() {
        "auto" => "api",
        value => value,
    }
    .to_owned();
    let port = explicit_port.unwrap_or_else(|| default_control_port(&selected_protocol));

    Ok(ControlRuntimeConfig {
        host,
        port,
        user,
        password,
        selected_protocol,
    })
}

fn resolve_control_password(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<String> {
    if let Some(password) = cli
        .password
        .clone()
        .or_else(|| env.get("ROS_PASSWORD").cloned())
    {
        return Ok(password);
    }

    let Some(profile) = profile else {
        return Err(Box::new(RosWireError::config(
            "missing RouterOS control password; set --password, ROS_PASSWORD, or profile secret password",
        )));
    };
    config::resolve_profile_secret_value(profile, "password", env)?.ok_or_else(|| {
        Box::new(RosWireError::config(
            "missing RouterOS control password; set --password, ROS_PASSWORD, or profile secret password",
        ))
    })
}

fn validate_control_protocol(value: &str) -> RosWireResult<()> {
    match value {
        "auto" | "api" | "api-ssl" | "rest" => Ok(()),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid protocol value: {value}",
        )))),
    }
}

fn default_control_port(protocol: &str) -> u16 {
    match protocol {
        "api-ssl" => 8729,
        "rest" => 443,
        _ => 8728,
    }
}

fn execute_rest_control(
    command: &ControlCommand,
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<()> {
    let (path, body) = command.rest_request();
    RestClient::https(
        &control.host,
        control.port,
        &control.user,
        &control.password,
    )
    .post_json(path, body)
    .map(|_| ())
    .map_err(|error| Box::new((*error).clone().with_context(context)))
}

fn execute_classic_control<S: ApiStream>(
    stream: S,
    command: &ControlCommand,
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<()> {
    let mut session = ClassicApiSession::new(stream);
    session
        .login(&control.user, &control.password)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    session
        .execute_words(&command.classic_words())
        .map(|_| ())
        .map_err(|error| Box::new((*error).clone().with_context(context)))
}

fn selected_context(context: &ErrorContext, selected_protocol: &str) -> ErrorContext {
    let mut context = context.clone();
    context.selected_protocol = selected_protocol.to_owned();
    context
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

fn execute_upload(
    local: &str,
    remote: &str,
    config: &SshRuntimeConfig,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    let metadata = fs::metadata(local).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to inspect local file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    if metadata.len() > MAX_TRANSFER_BYTES {
        return Err(Box::new(
            RosWireError::file_too_large(format!(
                "local file exceeds transfer limit of {MAX_TRANSFER_BYTES} bytes",
            ))
            .with_context(context.clone()),
        ));
    }

    let session = open_ssh_session(config, context)?;
    let sftp = session.sftp().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to open SFTP session: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let mut source = File::open(local).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to open local file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let mut target = sftp.create(Path::new(remote)).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to create remote file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    copy_with_sha256(&mut source, &mut target, context)
}

fn execute_download(
    remote: &str,
    local: &str,
    config: &SshRuntimeConfig,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    let session = open_ssh_session(config, context)?;
    let sftp = session.sftp().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to open SFTP session: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let mut source = sftp.open(Path::new(remote)).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to open remote file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let mut target = File::create(local).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to create local file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    copy_with_sha256(&mut source, &mut target, context)
}

fn open_ssh_session(
    config: &SshRuntimeConfig,
    context: &ErrorContext,
) -> RosWireResult<ssh2::Session> {
    let address = format!("{}:{}", config.host, config.port);
    let socket_addr = address
        .to_socket_addrs()
        .map_err(|error| {
            Box::new(
                RosWireError::network(format!("failed to resolve SSH host: {error}"))
                    .with_context(context.clone()),
            )
        })?
        .next()
        .ok_or_else(|| {
            Box::new(
                RosWireError::network("failed to resolve SSH host").with_context(context.clone()),
            )
        })?;
    let tcp =
        TcpStream::connect_timeout(&socket_addr, Duration::from_secs(10)).map_err(|error| {
            Box::new(
                RosWireError::network(format!("failed to connect to SSH service: {error}"))
                    .with_context(context.clone()),
            )
        })?;
    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(30))).ok();

    let mut session = ssh2::Session::new().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to create SSH session: {error}"))
                .with_context(context.clone()),
        )
    })?;
    session.set_tcp_stream(tcp);
    session.handshake().map_err(|error| {
        Box::new(
            RosWireError::network(format!("SSH handshake failed: {error}"))
                .with_context(context.clone()),
        )
    })?;
    verify_host_key(&session, &config.expected_host_key, context)?;

    if let Some(key_path) = &config.key_path {
        session
            .userauth_pubkey_file(&config.user, None, Path::new(key_path), None)
            .map_err(|error| {
                Box::new(
                    RosWireError::auth_failed(format!("SSH key authentication failed: {error}"))
                        .with_context(context.clone()),
                )
            })?;
    } else {
        let password = config.password.as_deref().ok_or_else(|| {
            Box::new(RosWireError::config("missing SSH password").with_context(context.clone()))
        })?;
        session
            .userauth_password(&config.user, password)
            .map_err(|error| {
                Box::new(
                    RosWireError::auth_failed(format!(
                        "SSH password authentication failed: {error}"
                    ))
                    .with_context(context.clone()),
                )
            })?;
    }

    if !session.authenticated() {
        return Err(Box::new(
            RosWireError::auth_failed("SSH authentication failed").with_context(context.clone()),
        ));
    }

    Ok(session)
}

fn verify_host_key(
    session: &ssh2::Session,
    expected: &str,
    context: &ErrorContext,
) -> RosWireResult<()> {
    let actual = session
        .host_key_hash(ssh2::HashType::Sha256)
        .map(sha256_fingerprint)
        .ok_or_else(|| {
            Box::new(
                RosWireError::ssh_host_key_mismatch("SSH host key fingerprint is unavailable")
                    .with_context(context.clone()),
            )
        })?;
    if !host_key_matches(expected, &actual) {
        return Err(Box::new(
            RosWireError::ssh_host_key_mismatch(
                "SSH host key fingerprint does not match expected value",
            )
            .with_context(context.clone()),
        ));
    }

    Ok(())
}

fn host_key_matches(expected: &str, actual: &str) -> bool {
    expected.trim() == actual
}

fn sha256_fingerprint(bytes: &[u8]) -> String {
    format!("SHA256:{}", BASE64_NO_PAD.encode(bytes))
}

fn copy_with_sha256<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    let mut bytes = 0_u64;
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!(
                    "failed to read transfer stream: {error}"
                ))
                .with_context(context.clone()),
            )
        })?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > MAX_TRANSFER_BYTES {
            return Err(Box::new(
                RosWireError::file_too_large(format!(
                    "transfer exceeds limit of {MAX_TRANSFER_BYTES} bytes",
                ))
                .with_context(context.clone()),
            ));
        }
        hasher.update(&buffer[..read]);
        writer.write_all(&buffer[..read]).map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!(
                    "failed to write transfer stream: {error}"
                ))
                .with_context(context.clone()),
            )
        })?;
    }

    Ok((bytes, format!("{:x}", hasher.finalize())))
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
    use super::{
        build_plan_for_env, copy_with_sha256, execute_classic_control, execute_file_workflow,
        handle_transfer_for_env, host_key_matches, load_selected_profile, parse_port,
        parse_transfer_command, resolve_control_runtime_config, resolve_ssh_runtime_config,
        resolve_transfer_backend, selected_context, sha256_fingerprint, validate_safe_cidr,
        ControlCommand, ControlRuntimeConfig, LiveWorkflowBackend, SshRuntimeConfig,
        TransferCommand, WorkflowBackend, MAX_TRANSFER_BYTES,
    };
    use crate::args::Cli;
    use crate::error::{ErrorCode, ErrorContext};
    use crate::protocol::classic::sentence::{read_sentence, write_sentence};
    use clap::Parser;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Cursor, Read, Result as IoResult, Write};
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
            "--ssh-key",
            "/Users/example/.ssh/id_ed25519",
            "file",
            "download",
            "flash/setup.rsc",
            "setup.rsc",
        ])
        .expect("cli should parse");
        let env = BTreeMap::from([("ROS_SSH_HOST_KEY".to_owned(), "SHA256:from-env".to_owned())]);

        let runtime =
            resolve_ssh_runtime_config(&cli, &env, None).expect("runtime config should resolve");

        assert_eq!(runtime.host, "198.51.100.10");
        assert_eq!(runtime.user, "api-user");
        assert_eq!(runtime.password, None);
        assert_eq!(
            runtime.key_path.as_deref(),
            Some("/Users/example/.ssh/id_ed25519")
        );
        assert_eq!(runtime.expected_host_key, "SHA256:from-env");
    }

    #[test]
    fn transfer_backend_and_port_validation_are_structured() {
        let cli = Cli::try_parse_from(["roswire", "--transfer", "ssh", "file", "upload", "a", "b"])
            .expect("cli should parse");

        assert_eq!(
            resolve_transfer_backend(&cli, &isolated_env()).expect("ssh backend should resolve"),
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

        let payload =
            execute_file_workflow(&command, &cli, &mut backend, &workflow_context("import"))
                .expect("workflow should succeed");

        assert_eq!(payload.operation, "import.plan");
        assert_eq!(payload.bytes, 11);
        assert_eq!(payload.checksum_sha256, "upload-sha256");
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
            &mut backend,
            &workflow_context("backup/download"),
        )
        .expect("workflow should succeed");

        assert_eq!(payload.operation, "backup.download");
        assert_eq!(payload.bytes, 17);
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

        let error =
            execute_file_workflow(&command, &cli, &mut backend, &workflow_context("import"))
                .expect_err("cleanup failure should fail the workflow");

        assert_eq!(error.error_code, ErrorCode::FileTransferFailed);
        assert!(error.message.contains("cleanup failed"));
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
    fn control_runtime_resolves_env_profile_and_validation_paths() {
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

        let env_runtime = resolve_control_runtime_config(
            &cli,
            &BTreeMap::from([
                ("ROS_HOST".to_owned(), "203.0.113.10".to_owned()),
                ("ROS_USER".to_owned(), "env-api".to_owned()),
                ("ROS_PASSWORD".to_owned(), "env-secret".to_owned()),
                ("ROS_PROTOCOL".to_owned(), "api".to_owned()),
                ("ROS_PORT".to_owned(), "8728".to_owned()),
            ]),
            None,
        )
        .expect("env runtime should resolve");
        assert_eq!(env_runtime.selected_protocol, "api");
        assert_eq!(env_runtime.port, 8728);

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
            .expect_err("bad protocol should fail")
            .error_code,
            ErrorCode::UsageError,
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
        fail_remove: bool,
    }

    impl WorkflowBackend for FakeWorkflowBackend {
        fn upload(
            &mut self,
            local: &str,
            remote: &str,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<(u64, String)> {
            self.events.push(format!("upload:{local}->{remote}"));
            Ok((11, "upload-sha256".to_owned()))
        }

        fn download(
            &mut self,
            remote: &str,
            local: &str,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<(u64, String)> {
            self.events.push(format!("download:{remote}->{local}"));
            Ok((17, "download-sha256".to_owned()))
        }

        fn finalize_local_download(
            &mut self,
            temporary_local: &str,
            local: &str,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events
                .push(format!("finalize:{temporary_local}->{local}"));
            Ok(())
        }

        fn execute_control(
            &mut self,
            command: &ControlCommand,
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events
                .push(format!("control:{}", command.classic_words().join(" ")));
            Ok(())
        }

        fn wait_remote_file(
            &mut self,
            remote: &str,
            _timeout: Duration,
            context: &ErrorContext,
        ) -> crate::error::RosWireResult<()> {
            self.events.push(format!("wait:{remote}"));
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
                expected_host_key: "SHA256:test".to_owned(),
            },
            ControlRuntimeConfig {
                host: "127.0.0.1".to_owned(),
                port: 0,
                user: "api-user".to_owned(),
                password: "api-secret".to_owned(),
                selected_protocol: selected_protocol.to_owned(),
            },
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
