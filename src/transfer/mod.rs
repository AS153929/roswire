use crate::args::{Cli, TransferIfExists};
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
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_WAIT_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_TRANSFER_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_CLEANUP_TIMEOUT_SECONDS: u64 = 10;
const MAX_TRANSFER_RETRIES: u8 = 5;
const WORKFLOW_FILE_WAIT_INTERVAL: Duration = Duration::from_secs(1);
const SSH_KEY_PASSPHRASE_SECRET: &str = "ssh_key_passphrase";

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransferPolicy {
    if_exists: TransferIfExists,
    timeouts: TransferTimeouts,
    retry: TransferRetryPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferTimeouts {
    connect_seconds: u64,
    wait_remote_file_seconds: u64,
    transfer_seconds: u64,
    cleanup_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransferRetryPolicy {
    max_retries: u8,
    delay_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferRetryPlan {
    max_retries: u8,
    delay_seconds: u64,
    retryable_error_codes: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferPolicyPlan {
    if_exists: String,
    timeouts: TransferTimeouts,
    retry: TransferRetryPlan,
}

impl TransferPolicy {
    fn plan(&self) -> TransferPolicyPlan {
        TransferPolicyPlan {
            if_exists: self.if_exists.as_str().to_owned(),
            timeouts: self.timeouts.clone(),
            retry: TransferRetryPlan {
                max_retries: self.retry.max_retries,
                delay_seconds: self.retry.delay_seconds,
                retryable_error_codes: vec![
                    "NETWORK_ERROR",
                    "FILE_TRANSFER_FAILED",
                    "ROS_API_FAILURE",
                ],
            },
        }
    }

    fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.timeouts.connect_seconds)
    }

    fn transfer_timeout(&self) -> Duration {
        Duration::from_secs(self.timeouts.transfer_seconds)
    }

    fn wait_timeout(&self) -> Duration {
        Duration::from_secs(self.timeouts.wait_remote_file_seconds)
    }

    fn retry_delay(&self) -> Duration {
        Duration::from_secs(self.retry.delay_seconds)
    }
}

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
    pub policy: TransferPolicyPlan,
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
    pub key_passphrase: String,
    pub data_plane: String,
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
    key_passphrase: Option<String>,
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
struct SshServiceSnapshot {
    id: Option<String>,
    disabled: bool,
    address: Vec<String>,
}

impl Default for SshServiceSnapshot {
    fn default() -> Self {
        Self {
            id: Some("ssh".to_owned()),
            disabled: false,
            address: Vec::new(),
        }
    }
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
    fn read_ssh_service(&mut self, context: &ErrorContext) -> RosWireResult<SshServiceSnapshot>;
    fn apply_ssh_service(
        &mut self,
        desired: &SshServiceSnapshot,
        context: &ErrorContext,
    ) -> RosWireResult<()>;
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
    policy: TransferPolicy,
}

impl LiveWorkflowBackend {
    fn new(ssh: SshRuntimeConfig, control: ControlRuntimeConfig, policy: TransferPolicy) -> Self {
        Self {
            ssh,
            control,
            policy,
        }
    }
}

impl WorkflowBackend for LiveWorkflowBackend {
    fn read_ssh_service(&mut self, context: &ErrorContext) -> RosWireResult<SshServiceSnapshot> {
        let context = selected_context(context, &self.control.selected_protocol);
        read_ssh_service(&self.control, context)
    }

    fn apply_ssh_service(
        &mut self,
        desired: &SshServiceSnapshot,
        context: &ErrorContext,
    ) -> RosWireResult<()> {
        let context = selected_context(context, &self.control.selected_protocol);
        apply_ssh_service(&self.control, desired, context)
    }

    fn upload(
        &mut self,
        local: &str,
        remote: &str,
        context: &ErrorContext,
    ) -> RosWireResult<(u64, String)> {
        execute_upload(local, remote, &self.ssh, &self.policy, context)
    }

    fn download(
        &mut self,
        remote: &str,
        local: &str,
        context: &ErrorContext,
    ) -> RosWireResult<(u64, String)> {
        execute_download(remote, local, &self.ssh, &self.policy, context)
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
        let session = open_ssh_session(&self.ssh, &self.policy, context)?;
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
        let session = open_ssh_session(&self.ssh, &self.policy, context)?;
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
    let profile = load_selected_profile(cli, env)?;
    let backend = resolve_transfer_backend(cli, profile.as_ref())?;
    if backend != DEFAULT_TRANSFER_BACKEND {
        return Err(Box::new(RosWireError::usage(format!(
            "unsupported transfer backend: {backend}",
        ))));
    }
    let context = transfer_context(&command, &backend, cli, profile.as_ref());
    let policy = transfer_policy(cli)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    let ssh_runtime = resolve_ssh_runtime_config(cli, env, profile.as_ref())
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    let allow_from = resolve_allow_from_for_runtime(cli, profile.as_ref(), &context)?;

    match &command {
        TransferCommand::FileUpload { local, remote } => {
            if cli.ensure_ssh || cli.restore_ssh {
                let control_runtime = resolve_control_runtime_config(cli, env, profile.as_ref())
                    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
                let mut workflow_backend =
                    LiveWorkflowBackend::new(ssh_runtime, control_runtime, policy.clone());
                execute_with_ssh_service_guard(
                    &command,
                    cli,
                    &allow_from,
                    &mut workflow_backend,
                    &context,
                    |backend| {
                        backend
                            .upload(local, remote, &context)
                            .map(|(bytes, checksum_sha256)| {
                                direct_transfer_payload(
                                    command.operation(),
                                    backend_name(&backend),
                                    bytes,
                                    checksum_sha256,
                                    Some(redact_local_path(local)),
                                    Some(redact_remote_path(remote)),
                                )
                            })
                    },
                )
            } else {
                execute_upload(local, remote, &ssh_runtime, &policy, &context).map(
                    |(bytes, checksum_sha256)| {
                        direct_transfer_payload(
                            command.operation(),
                            &backend,
                            bytes,
                            checksum_sha256,
                            Some(redact_local_path(local)),
                            Some(redact_remote_path(remote)),
                        )
                    },
                )
            }
        }
        TransferCommand::FileDownload { remote, local } => {
            if prepare_local_destination(local, &policy, &context)? == DestinationDecision::Skip {
                return Ok(skipped_transfer_payload(
                    command.operation(),
                    &backend,
                    Some(redact_local_path(local)),
                    Some(redact_remote_path(remote)),
                ));
            }
            if cli.ensure_ssh || cli.restore_ssh {
                let control_runtime = resolve_control_runtime_config(cli, env, profile.as_ref())
                    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
                let mut workflow_backend =
                    LiveWorkflowBackend::new(ssh_runtime, control_runtime, policy.clone());
                execute_with_ssh_service_guard(
                    &command,
                    cli,
                    &allow_from,
                    &mut workflow_backend,
                    &context,
                    |backend| {
                        backend
                            .download(remote, local, &context)
                            .map(|(bytes, checksum_sha256)| {
                                direct_transfer_payload(
                                    command.operation(),
                                    backend_name(&backend),
                                    bytes,
                                    checksum_sha256,
                                    Some(redact_local_path(local)),
                                    Some(redact_remote_path(remote)),
                                )
                            })
                    },
                )
            } else {
                execute_download(remote, local, &ssh_runtime, &policy, &context).map(
                    |(bytes, checksum_sha256)| {
                        direct_transfer_payload(
                            command.operation(),
                            &backend,
                            bytes,
                            checksum_sha256,
                            Some(redact_local_path(local)),
                            Some(redact_remote_path(remote)),
                        )
                    },
                )
            }
        }
        TransferCommand::Import { .. }
        | TransferCommand::BackupDownload { .. }
        | TransferCommand::ExportDownload { .. } => {
            let control_runtime = resolve_control_runtime_config(cli, env, profile.as_ref())
                .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
            let mut backend =
                LiveWorkflowBackend::new(ssh_runtime, control_runtime, policy.clone());
            execute_file_workflow(&command, cli, &allow_from, &policy, &mut backend, &context)
        }
    }
}

fn backend_name<B>(_backend: &B) -> &'static str {
    DEFAULT_TRANSFER_BACKEND
}

fn direct_transfer_payload(
    operation: &str,
    backend: &str,
    bytes: u64,
    checksum_sha256: String,
    local_path: Option<String>,
    remote_path: Option<String>,
) -> TransferResultPayload {
    TransferResultPayload {
        schema_version: RESULT_SCHEMA_VERSION,
        operation: operation.to_owned(),
        transfer_backend: backend.to_owned(),
        status: "ok",
        bytes,
        checksum_sha256,
        paths: TransferPaths {
            local_path,
            remote_path,
            temporary_remote_path: None,
            temporary_local_path: None,
        },
    }
}

fn skipped_transfer_payload(
    operation: &str,
    backend: &str,
    local_path: Option<String>,
    remote_path: Option<String>,
) -> TransferResultPayload {
    TransferResultPayload {
        schema_version: RESULT_SCHEMA_VERSION,
        operation: operation.to_owned(),
        transfer_backend: backend.to_owned(),
        status: "skipped",
        bytes: 0,
        checksum_sha256: "".to_owned(),
        paths: TransferPaths {
            local_path,
            remote_path,
            temporary_remote_path: None,
            temporary_local_path: None,
        },
    }
}

fn execute_file_workflow<B: WorkflowBackend>(
    command: &TransferCommand,
    cli: &Cli,
    allow_from: &[String],
    policy: &TransferPolicy,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    execute_with_ssh_service_guard(command, cli, allow_from, backend, context, |backend| {
        execute_file_workflow_inner(command, cli, policy, backend, context)
    })
}

fn execute_file_workflow_inner<B: WorkflowBackend>(
    command: &TransferCommand,
    cli: &Cli,
    policy: &TransferPolicy,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    match command {
        TransferCommand::Import { local } => execute_import_workflow(local, cli, backend, context),
        TransferCommand::BackupDownload { local } => execute_generated_download_workflow(
            GeneratedDownloadWorkflow {
                operation: command.operation(),
                local,
                remote: generated_backup_name(cli),
                control: ControlCommand::BackupSave {
                    name: generated_backup_base_name(cli),
                },
                cleanup_remote: cli.cleanup,
            },
            policy,
            backend,
            context,
        ),
        TransferCommand::ExportDownload { local } => execute_generated_download_workflow(
            GeneratedDownloadWorkflow {
                operation: command.operation(),
                local,
                remote: generated_export_name(cli),
                control: ControlCommand::Export {
                    file: generated_export_base_name(cli),
                    compact: cli.compact,
                },
                cleanup_remote: cli.cleanup,
            },
            policy,
            backend,
            context,
        ),
        TransferCommand::FileUpload { .. } | TransferCommand::FileDownload { .. } => unreachable!(
            "direct file upload/download workflows are executed before workflow dispatch"
        ),
    }
}

fn execute_with_ssh_service_guard<B, F>(
    _command: &TransferCommand,
    cli: &Cli,
    allow_from: &[String],
    backend: &mut B,
    context: &ErrorContext,
    operation: F,
) -> RosWireResult<TransferResultPayload>
where
    B: WorkflowBackend,
    F: FnOnce(&mut B) -> RosWireResult<TransferResultPayload>,
{
    let snapshot = if cli.ensure_ssh || cli.restore_ssh {
        Some(backend.read_ssh_service(context)?)
    } else {
        None
    };

    if cli.ensure_ssh {
        let snapshot = snapshot
            .as_ref()
            .expect("snapshot is captured whenever ensure_ssh is set");
        let desired = desired_ssh_service_state(snapshot, allow_from);
        if &desired != snapshot {
            backend.apply_ssh_service(&desired, context)?;
        }
    }

    let result = operation(backend);

    if cli.restore_ssh {
        if let Some(snapshot) = &snapshot {
            let original_error = result.as_ref().err().map(|error| error.message.clone());
            if let Err(restore_error) = backend.apply_ssh_service(snapshot, context) {
                return Err(Box::new(
                    ssh_restore_failed_error(&restore_error, original_error.as_deref())
                        .with_context(context.clone()),
                ));
            }
        }
    }

    result
}

fn desired_ssh_service_state(
    snapshot: &SshServiceSnapshot,
    allow_from: &[String],
) -> SshServiceSnapshot {
    SshServiceSnapshot {
        id: snapshot.id.clone(),
        disabled: false,
        address: merge_ssh_allow_list(&snapshot.address, allow_from),
    }
}

fn ssh_restore_failed_error(
    restore_error: &RosWireError,
    original_error: Option<&str>,
) -> RosWireError {
    let message = match original_error {
        Some(original) => format!(
            "failed to restore RouterOS SSH service state after transfer error `{original}`: {}",
            restore_error.message
        ),
        None => format!(
            "failed to restore RouterOS SSH service state after successful transfer: {}",
            restore_error.message
        ),
    };
    RosWireError::ssh_restore_failed(message)
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

struct GeneratedDownloadWorkflow<'a> {
    operation: &'a str,
    local: &'a str,
    remote: String,
    control: ControlCommand,
    cleanup_remote: bool,
}

fn execute_generated_download_workflow<B: WorkflowBackend>(
    spec: GeneratedDownloadWorkflow<'_>,
    policy: &TransferPolicy,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    if prepare_local_destination(spec.local, policy, context)? == DestinationDecision::Skip {
        return Ok(skipped_transfer_payload(
            spec.operation,
            DEFAULT_TRANSFER_BACKEND,
            Some(redact_local_path(spec.local)),
            Some(redact_remote_path(&spec.remote)),
        ));
    }

    backend.execute_control(&spec.control, context)?;
    retry_transfer_step(policy, || {
        backend.wait_remote_file(&spec.remote, policy.wait_timeout(), context)
    })?;

    let tmp_local = raw_temporary_local_path(spec.local);
    let (bytes, checksum_sha256) = backend.download(&spec.remote, &tmp_local, context)?;
    backend.finalize_local_download(&tmp_local, spec.local, context)?;

    if spec.cleanup_remote {
        backend.remove_remote_file(&spec.remote, context)?;
    }

    Ok(TransferResultPayload {
        schema_version: RESULT_SCHEMA_VERSION,
        operation: spec.operation.to_owned(),
        transfer_backend: DEFAULT_TRANSFER_BACKEND.to_owned(),
        status: "ok",
        bytes,
        checksum_sha256,
        paths: TransferPaths {
            local_path: Some(redact_local_path(spec.local)),
            remote_path: Some(redact_remote_path(&spec.remote)),
            temporary_remote_path: Some(redact_remote_path(&spec.remote)),
            temporary_local_path: Some(temporary_local_path(spec.local)),
        },
    })
}

fn build_plan_for_env(
    command: TransferCommand,
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> RosWireResult<TransferPlan> {
    let profile = load_selected_profile(cli, env)?;
    if let Some(host) = cli
        .host
        .as_deref()
        .or_else(|| profile.as_ref().and_then(|profile| profile.host.as_deref()))
    {
        config::validate_remote_host(host)?;
    }

    let backend = resolve_transfer_backend(cli, profile.as_ref())?;
    if backend != DEFAULT_TRANSFER_BACKEND {
        return Err(Box::new(RosWireError::usage(format!(
            "unsupported transfer backend: {backend}",
        ))));
    }

    let context = transfer_context(&command, &backend, cli, profile.as_ref());
    let policy = transfer_policy(cli)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
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
        .or_else(|| {
            profile
                .as_ref()
                .and_then(|profile| profile.ssh_host_key.clone())
        })
        .filter(|value| !value.trim().is_empty());
    if host_key.is_none() {
        return Err(Box::new(
            RosWireError::ssh_host_key_required(
                "SSH transfer dry-run requires an expected RouterOS SSH host key fingerprint",
            )
            .with_context(context),
        ));
    }

    let allow_from = resolve_allow_from(cli, profile.as_ref()).map_err(|error| {
        Box::new((*error).clone().with_context(transfer_context(
            &command,
            &backend,
            cli,
            profile.as_ref(),
        )))
    })?;
    if allow_from.is_empty() {
        return Err(Box::new(
            RosWireError::ssh_whitelist_required(
                "SSH transfer dry-run requires at least one allow-from CIDR",
            )
            .with_context(context),
        ));
    }

    let ssh = resolve_ssh_transfer_summary(cli, env, profile.as_ref())?;

    Ok(plan_from_command(
        command, backend, allow_from, ssh, cli, &policy,
    ))
}

fn resolve_transfer_backend(
    cli: &Cli,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<String> {
    let backend = cli
        .transfer
        .map(|value| value.as_str().to_owned())
        .or_else(|| profile.and_then(|profile| profile.transfer.clone()))
        .unwrap_or_else(|| DEFAULT_TRANSFER_BACKEND.to_owned());
    match backend.as_str() {
        DEFAULT_TRANSFER_BACKEND => Ok(backend),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid transfer value: {backend}",
        )))),
    }
}

fn resolve_allow_from(
    cli: &Cli,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<Vec<String>> {
    let values = if !cli.allow_from.is_empty() {
        cli.allow_from.clone()
    } else {
        profile
            .map(|profile| profile.allow_from.clone())
            .unwrap_or_default()
    };

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

fn resolve_allow_from_for_runtime(
    cli: &Cli,
    profile: Option<&config::ProfileConfig>,
    context: &ErrorContext,
) -> RosWireResult<Vec<String>> {
    if !cli.ensure_ssh {
        return Ok(Vec::new());
    }

    let allow_from = resolve_allow_from(cli, profile)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    if allow_from.is_empty() {
        return Err(Box::new(
            RosWireError::ssh_whitelist_required(
                "--ensure-ssh requires at least one allow-from CIDR from --allow-from or profile allow_from to merge into /ip service ssh address",
            )
            .with_context(context.clone()),
        ));
    }
    Ok(allow_from)
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
    policy: &TransferPolicy,
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
        policy: policy.plan(),
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
    let profile_name = match config::select_active_profile(cli.profile.as_deref(), &config_file) {
        Ok(profile_name) => profile_name,
        Err(error) if cli.profile.is_some() => return Err(error),
        Err(_) => return Ok(None),
    };
    Ok(config_file.profiles.get(&profile_name).cloned())
}

fn resolve_ssh_transfer_summary(
    cli: &Cli,
    _env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<SshTransferSummary> {
    let port = cli
        .ssh_port
        .or_else(|| profile.and_then(|profile| profile.ssh_port))
        .unwrap_or(22);

    let user = cli
        .ssh_user
        .clone()
        .or_else(|| profile.and_then(|profile| profile.ssh_user.clone()))
        .or_else(|| cli.user.clone())
        .or_else(|| profile.and_then(|profile| profile.user.clone()))
        .unwrap_or_else(|| "reuse-api-user".to_owned());

    let key_path = cli
        .ssh_key
        .clone()
        .or_else(|| profile.and_then(|profile| profile.ssh_key.clone()))
        .filter(|value| !value.trim().is_empty())
        .map(|value| redact_local_path(&value));
    let key_passphrase = key_passphrase_status(key_path.is_some(), profile);
    let auth_method = if key_path.is_some() && key_passphrase == "provided" {
        "key-encrypted".to_owned()
    } else if key_path.is_some() {
        "key".to_owned()
    } else if cli.ssh_password.is_some()
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
        key_passphrase,
        data_plane: "sftp-with-scp-fallback".to_owned(),
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
        .or_else(|| profile.and_then(|profile| profile.host.clone()))
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing SSH transfer host; set --host or profile host",
            ))
        })?;
    config::validate_remote_host(&host)?;

    let summary = resolve_ssh_transfer_summary(cli, env, profile)?;
    if summary.user == "reuse-api-user" {
        return Err(Box::new(RosWireError::config(
            "missing SSH transfer user; set --ssh-user, --user, or profile user",
        )));
    }

    let key_path = cli
        .ssh_key
        .clone()
        .or_else(|| profile.and_then(|profile| profile.ssh_key.clone()))
        .filter(|value| !value.trim().is_empty());
    let password = if key_path.is_some() {
        None
    } else {
        Some(resolve_ssh_password(cli, env, profile)?)
    };
    let key_passphrase = if key_path.is_some() {
        resolve_ssh_key_passphrase(env, profile)?
    } else {
        None
    };
    let expected_host_key = cli
        .ssh_host_key
        .clone()
        .or_else(|| profile.and_then(|profile| profile.ssh_host_key.clone()))
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
        key_passphrase,
        expected_host_key,
    })
}

fn key_passphrase_status(has_key_path: bool, profile: Option<&config::ProfileConfig>) -> String {
    if !has_key_path {
        return "not-applicable".to_owned();
    }

    if profile.is_some_and(|profile| profile.secrets.contains_key(SSH_KEY_PASSPHRASE_SECRET)) {
        "provided".to_owned()
    } else {
        "not-provided".to_owned()
    }
}

fn resolve_ssh_key_passphrase(
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<Option<String>> {
    let Some(profile) = profile else {
        return Ok(None);
    };

    config::resolve_profile_secret_value(profile, SSH_KEY_PASSPHRASE_SECRET, env)
}

fn resolve_ssh_password(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<String> {
    if let Some(password) = cli.ssh_password.clone().or_else(|| cli.password.clone()) {
        return Ok(password);
    }

    let Some(profile) = profile else {
        return Err(Box::new(RosWireError::config(
            "missing SSH transfer password; set --ssh-password, --password, or profile secret ssh_password/password",
        )));
    };

    config::resolve_profile_secret_value(profile, "ssh_password", env)?
        .or_else(|| config::resolve_profile_secret_value(profile, "password", env).ok().flatten())
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing SSH transfer password; set --ssh-password, --password, or profile secret ssh_password/password",
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
        .or_else(|| profile.and_then(|profile| profile.host.clone()))
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing RouterOS control host; set --host or profile host",
            ))
        })?;
    config::validate_remote_host(&host)?;

    let user = cli
        .user
        .clone()
        .or_else(|| profile.and_then(|profile| profile.user.clone()))
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing RouterOS control user; set --user or profile user",
            ))
        })?;
    let password = resolve_control_password(cli, env, profile)?;
    let requested_protocol = cli
        .protocol
        .map(|value| value.as_str().to_owned())
        .or_else(|| profile.and_then(|profile| profile.protocol.clone()))
        .unwrap_or_else(|| "auto".to_owned());
    validate_control_protocol(&requested_protocol)?;

    let explicit_port = cli
        .port
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
    if let Some(password) = cli.password.clone() {
        return Ok(password);
    }

    let Some(profile) = profile else {
        return Err(Box::new(RosWireError::config(
            "missing RouterOS control password; set --password or profile secret password",
        )));
    };
    config::resolve_profile_secret_value(profile, "password", env)?.ok_or_else(|| {
        Box::new(RosWireError::config(
            "missing RouterOS control password; set --password or profile secret password",
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

fn read_ssh_service(
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<SshServiceSnapshot> {
    match control.selected_protocol.as_str() {
        "rest" => read_rest_ssh_service(control, context),
        "api-ssl" => {
            let stream =
                TlsApiStream::connect(&control.host, control.port, Duration::from_secs(10))
                    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
            read_classic_ssh_service(stream, control, context)
        }
        _ => {
            let stream =
                TcpApiStream::connect(&control.host, control.port, Duration::from_secs(10))
                    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
            read_classic_ssh_service(stream, control, context)
        }
    }
}

fn apply_ssh_service(
    control: &ControlRuntimeConfig,
    desired: &SshServiceSnapshot,
    context: ErrorContext,
) -> RosWireResult<()> {
    match control.selected_protocol.as_str() {
        "rest" => apply_rest_ssh_service(control, desired, context),
        "api-ssl" => {
            let stream =
                TlsApiStream::connect(&control.host, control.port, Duration::from_secs(10))
                    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
            apply_classic_ssh_service(stream, control, desired, context)
        }
        _ => {
            let stream =
                TcpApiStream::connect(&control.host, control.port, Duration::from_secs(10))
                    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
            apply_classic_ssh_service(stream, control, desired, context)
        }
    }
}

fn read_classic_ssh_service<S: ApiStream>(
    stream: S,
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<SshServiceSnapshot> {
    let mut session = ClassicApiSession::new(stream);
    session
        .login(&control.user, &control.password)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    let rows = session
        .execute_words(&["/ip/service/print".to_owned(), "?name=ssh".to_owned()])
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    let row = rows.into_iter().next().ok_or_else(|| {
        Box::new(RosWireError::ros_api_failure(
            "RouterOS SSH service was not found",
        ))
        .with_context(context.clone())
    })?;

    Ok(ssh_service_snapshot_from_fields(&row))
}

fn apply_classic_ssh_service<S: ApiStream>(
    stream: S,
    control: &ControlRuntimeConfig,
    desired: &SshServiceSnapshot,
    context: ErrorContext,
) -> RosWireResult<()> {
    let mut session = ClassicApiSession::new(stream);
    session
        .login(&control.user, &control.password)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;

    let mut words = vec!["/ip/service/set".to_owned()];
    if let Some(id) = &desired.id {
        words.push(format!("=.id={id}"));
    } else {
        words.push("=numbers=ssh".to_owned());
    }
    words.push(format!("=disabled={}", routeros_bool(desired.disabled)));
    words.push(format!("=address={}", desired.address.join(",")));

    session
        .execute_words(&words)
        .map(|_| ())
        .map_err(|error| Box::new((*error).clone().with_context(context)))
}

fn read_rest_ssh_service(
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<SshServiceSnapshot> {
    let client = RestClient::https(
        &control.host,
        control.port,
        &control.user,
        &control.password,
    );
    let value = client
        .get("/rest/ip/service")
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    ssh_service_snapshot_from_json(&value).map_err(|error| Box::new((*error).with_context(context)))
}

fn apply_rest_ssh_service(
    control: &ControlRuntimeConfig,
    desired: &SshServiceSnapshot,
    context: ErrorContext,
) -> RosWireResult<()> {
    let id = desired.id.as_deref().unwrap_or("ssh");
    let client = RestClient::https(
        &control.host,
        control.port,
        &control.user,
        &control.password,
    );
    client
        .patch_json(
            &format!("/rest/ip/service/{id}"),
            json!({
                "disabled": routeros_bool(desired.disabled),
                "address": desired.address.join(","),
            }),
        )
        .map(|_| ())
        .map_err(|error| Box::new((*error).clone().with_context(context)))
}

fn ssh_service_snapshot_from_fields(fields: &BTreeMap<String, String>) -> SshServiceSnapshot {
    SshServiceSnapshot {
        id: fields
            .get(".id")
            .cloned()
            .or_else(|| fields.get("id").cloned())
            .or_else(|| Some("ssh".to_owned())),
        disabled: fields
            .get("disabled")
            .is_some_and(|value| routeros_bool_is_true(value)),
        address: fields
            .get("address")
            .map(|value| parse_address_list(value))
            .unwrap_or_default(),
    }
}

fn ssh_service_snapshot_from_json(value: &Value) -> RosWireResult<SshServiceSnapshot> {
    let entry = match value {
        Value::Array(items) => items
            .iter()
            .find(|item| {
                value_string(item.get("name").unwrap_or(&Value::Null)).as_deref() == Some("ssh")
            })
            .ok_or_else(|| {
                Box::new(RosWireError::ros_api_failure(
                    "RouterOS SSH service was not found",
                ))
            })?,
        Value::Object(_) => value,
        _ => {
            return Err(Box::new(RosWireError::ros_api_failure(
                "RouterOS SSH service response has unexpected shape",
            )))
        }
    };
    let object = entry.as_object().ok_or_else(|| {
        Box::new(RosWireError::ros_api_failure(
            "RouterOS SSH service response item has unexpected shape",
        ))
    })?;

    Ok(SshServiceSnapshot {
        id: object
            .get(".id")
            .and_then(value_string)
            .or_else(|| object.get("id").and_then(value_string))
            .or_else(|| Some("ssh".to_owned())),
        disabled: object
            .get("disabled")
            .and_then(value_string)
            .is_some_and(|value| routeros_bool_is_true(&value)),
        address: object
            .get("address")
            .and_then(value_string)
            .map(|value| parse_address_list(&value))
            .unwrap_or_default(),
    })
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(true) => Some("yes".to_owned()),
        Value::Bool(false) => Some("no".to_owned()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_address_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn routeros_bool(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn routeros_bool_is_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "1"
    )
}

fn merge_ssh_allow_list(existing: &[String], allow_from: &[String]) -> Vec<String> {
    let mut merged = if existing.is_empty() {
        Vec::new()
    } else {
        existing.to_vec()
    };
    for cidr in allow_from {
        if !merged.iter().any(|item| item == cidr) {
            merged.push(cidr.clone());
        }
    }
    merged
}

fn selected_context(context: &ErrorContext, selected_protocol: &str) -> ErrorContext {
    let mut context = context.clone();
    context.selected_protocol = selected_protocol.to_owned();
    context
}

fn transfer_policy(cli: &Cli) -> RosWireResult<TransferPolicy> {
    if cli.retries > MAX_TRANSFER_RETRIES {
        return Err(Box::new(RosWireError::usage(format!(
            "--retries must be <= {MAX_TRANSFER_RETRIES}"
        ))));
    }

    let mut policy = default_transfer_policy();
    policy.if_exists = cli.if_exists;
    policy.timeouts.connect_seconds = cli
        .connect_timeout_seconds
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECONDS);
    policy.timeouts.wait_remote_file_seconds = cli
        .wait_timeout_seconds
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECONDS);
    policy.timeouts.transfer_seconds = cli
        .transfer_timeout_seconds
        .unwrap_or(DEFAULT_TRANSFER_TIMEOUT_SECONDS);
    policy.timeouts.cleanup_seconds = cli
        .cleanup_timeout_seconds
        .unwrap_or(DEFAULT_CLEANUP_TIMEOUT_SECONDS);
    policy.retry.max_retries = cli.retries;
    policy.retry.delay_seconds = cli.retry_delay_seconds;

    Ok(policy)
}

fn default_transfer_policy() -> TransferPolicy {
    TransferPolicy {
        if_exists: TransferIfExists::Overwrite,
        timeouts: TransferTimeouts {
            connect_seconds: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            wait_remote_file_seconds: DEFAULT_WAIT_TIMEOUT_SECONDS,
            transfer_seconds: DEFAULT_TRANSFER_TIMEOUT_SECONDS,
            cleanup_seconds: DEFAULT_CLEANUP_TIMEOUT_SECONDS,
        },
        retry: TransferRetryPolicy {
            max_retries: 0,
            delay_seconds: 0,
        },
    }
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

fn retryable_transfer_error(error: &RosWireError) -> bool {
    matches!(
        error.error_code,
        error::ErrorCode::NetworkError
            | error::ErrorCode::FileTransferFailed
            | error::ErrorCode::RosApiFailure
    )
}

fn retry_transfer_step<T, F>(policy: &TransferPolicy, mut operation: F) -> RosWireResult<T>
where
    F: FnMut() -> RosWireResult<T>,
{
    let mut retries_used = 0;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if retries_used < policy.retry.max_retries && retryable_transfer_error(&error) =>
            {
                retries_used += 1;
                let delay = policy.retry_delay();
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
            Err(error) => return Err(error),
        }
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

fn execute_upload(
    local: &str,
    remote: &str,
    config: &SshRuntimeConfig,
    policy: &TransferPolicy,
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

    let session = open_ssh_session(config, policy, context)?;
    let sftp = match session.sftp() {
        Ok(sftp) => sftp,
        Err(error) => {
            return sftp_or_scp_fallback(
                "upload",
                Err(sftp_session_unavailable_error(error, context)),
                || execute_scp_upload(&session, local, remote, metadata.len(), context),
                context,
            );
        }
    };

    execute_sftp_upload(&sftp, local, remote, context)
}

fn execute_sftp_upload(
    sftp: &ssh2::Sftp,
    local: &str,
    remote: &str,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
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

fn execute_scp_upload(
    session: &ssh2::Session,
    local: &str,
    remote: &str,
    bytes: u64,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    let mut source = File::open(local).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to open local file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let mut target = session
        .scp_send(Path::new(remote), 0o644, bytes, None)
        .map_err(|error| {
            Box::new(
                RosWireError::file_transfer_failed(format!(
                    "SCP upload fallback failed to open remote file: {error}"
                ))
                .with_context(context.clone()),
            )
        })?;
    let result = copy_with_sha256(&mut source, &mut target, context)?;
    finish_scp_send(&mut target, context)?;
    Ok(result)
}

fn execute_download(
    remote: &str,
    local: &str,
    config: &SshRuntimeConfig,
    policy: &TransferPolicy,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    let session = open_ssh_session(config, policy, context)?;
    let sftp = match session.sftp() {
        Ok(sftp) => sftp,
        Err(error) => {
            return sftp_or_scp_fallback(
                "download",
                Err(sftp_session_unavailable_error(error, context)),
                || execute_scp_download(&session, remote, local, context),
                context,
            );
        }
    };

    execute_sftp_download(&sftp, remote, local, context)
}

fn execute_sftp_download(
    sftp: &ssh2::Sftp,
    remote: &str,
    local: &str,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
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

fn execute_scp_download(
    session: &ssh2::Session,
    remote: &str,
    local: &str,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    let (mut source, _stat) = session.scp_recv(Path::new(remote)).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!(
                "SCP download fallback failed to open remote file: {error}"
            ))
            .with_context(context.clone()),
        )
    })?;
    let mut target = File::create(local).map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to create local file: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let result = copy_with_sha256(&mut source, &mut target, context)?;
    finish_scp_recv(&mut source, context)?;
    Ok(result)
}

fn sftp_session_unavailable_error(error: ssh2::Error, context: &ErrorContext) -> Box<RosWireError> {
    Box::new(
        RosWireError::file_transfer_failed(format!("SFTP subsystem is unavailable: {error}"))
            .with_context(context.clone()),
    )
}

fn sftp_or_scp_fallback<T, F>(
    operation: &str,
    sftp_result: RosWireResult<T>,
    scp_operation: F,
    context: &ErrorContext,
) -> RosWireResult<T>
where
    F: FnOnce() -> RosWireResult<T>,
{
    match sftp_result {
        Ok(value) => Ok(value),
        Err(sftp_error) => match scp_operation() {
            Ok(value) => Ok(value),
            Err(scp_error) => Err(Box::new(
                RosWireError::file_transfer_failed(format!(
                    "SFTP {operation} is unavailable and SCP fallback failed: sftp: {}; scp: {}",
                    sftp_error.message, scp_error.message
                ))
                .with_context(context.clone()),
            )),
        },
    }
}

fn finish_scp_send(channel: &mut ssh2::Channel, context: &ErrorContext) -> RosWireResult<()> {
    channel.send_eof().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("SCP upload failed to send EOF: {error}"))
                .with_context(context.clone()),
        )
    })?;
    finish_scp_channel(channel, context)
}

fn finish_scp_recv(channel: &mut ssh2::Channel, context: &ErrorContext) -> RosWireResult<()> {
    finish_scp_channel(channel, context)
}

fn finish_scp_channel(channel: &mut ssh2::Channel, context: &ErrorContext) -> RosWireResult<()> {
    channel.wait_eof().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("SCP channel failed before EOF: {error}"))
                .with_context(context.clone()),
        )
    })?;
    channel.close().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("SCP channel failed to close: {error}"))
                .with_context(context.clone()),
        )
    })?;
    channel.wait_close().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("SCP channel close failed: {error}"))
                .with_context(context.clone()),
        )
    })
}

fn open_ssh_session(
    config: &SshRuntimeConfig,
    policy: &TransferPolicy,
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
        TcpStream::connect_timeout(&socket_addr, policy.connect_timeout()).map_err(|error| {
            Box::new(
                RosWireError::network(format!("failed to connect to SSH service: {error}"))
                    .with_context(context.clone()),
            )
        })?;
    tcp.set_read_timeout(Some(policy.transfer_timeout())).ok();
    tcp.set_write_timeout(Some(policy.transfer_timeout())).ok();

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
            .userauth_pubkey_file(
                &config.user,
                None,
                Path::new(key_path),
                config.key_passphrase.as_deref(),
            )
            .map_err(|error| {
                Box::new(
                    RosWireError::auth_failed(format!(
                        "SSH key authentication failed: {error}; if the private key is encrypted, configure profile secret {SSH_KEY_PASSPHRASE_SECRET}"
                    ))
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

#[cfg(test)]
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
            action: "apply-transfer-policy".to_owned(),
            description: "Apply if-exists, timeout, and finite retry policy before transfer side effects".to_owned(),
            dry_run_side_effects: "none",
        },
        TransferStep {
            order: 3,
            action: "verify-ssh-whitelist".to_owned(),
            description: "Validate allow-from CIDR values before merging with existing RouterOS SSH service address list".to_owned(),
            dry_run_side_effects: "none",
        },
    ];

    if cli.ensure_ssh || cli.restore_ssh {
        steps.push(TransferStep {
            order: 4,
            action: "snapshot-ssh-service".to_owned(),
            description: "Read /ip service ssh disabled/address state before transfer".to_owned(),
            dry_run_side_effects: "none",
        });
    }

    if cli.ensure_ssh {
        steps.push(TransferStep {
            order: 5,
            action: "ensure-ssh-service".to_owned(),
            description: "Enable RouterOS SSH service if needed and append/merge allow-from into the existing address whitelist".to_owned(),
            dry_run_side_effects: "none",
        });
    }

    let transfer_order = match (cli.ensure_ssh, cli.restore_ssh) {
        (true, _) => 6,
        (false, true) => 5,
        (false, false) => 4,
    };
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
            description: "Best-effort restore of captured SSH service state on success or ordinary errors; process interrupts are not trapped in this release".to_owned(),
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
        merge_ssh_allow_list, parse_port, parse_transfer_command, resolve_control_runtime_config,
        resolve_ssh_key_passphrase, resolve_ssh_runtime_config, resolve_ssh_transfer_summary,
        resolve_transfer_backend, routeros_bool, selected_context, sftp_or_scp_fallback,
        sha256_fingerprint, ssh_service_snapshot_from_fields, ssh_service_snapshot_from_json,
        transfer_policy, validate_safe_cidr, ControlCommand, ControlRuntimeConfig,
        LiveWorkflowBackend, SshRuntimeConfig, SshServiceSnapshot, TransferCommand,
        WorkflowBackend, DEFAULT_CONNECT_TIMEOUT_SECONDS, MAX_TRANSFER_BYTES,
    };
    use crate::args::{Cli, TransferIfExists};
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
    fn generated_download_wait_uses_finite_retry_policy() {
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
            wait_failures_remaining: 1,
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
        .expect("transient wait failure should be retried");

        assert_eq!(payload.status, "ok");
        assert_eq!(
            backend
                .events
                .iter()
                .filter(|event| event.as_str() == "wait:roswire-backup.backup")
                .count(),
            2
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
            _context: &ErrorContext,
        ) -> crate::error::RosWireResult<(u64, String)> {
            self.events.push(format!("download:{remote}->{local}"));
            Ok((24, "download-sha".to_owned()))
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
            },
            ControlRuntimeConfig {
                host: "127.0.0.1".to_owned(),
                port: 0,
                user: "api-user".to_owned(),
                password: "api-secret".to_owned(),
                selected_protocol: selected_protocol.to_owned(),
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
