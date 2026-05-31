use super::*;

pub(super) trait WorkflowBackend {
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
    fn remove_local_file(&mut self, local: &str, context: &ErrorContext) -> RosWireResult<()>;
}

pub(super) struct LiveWorkflowBackend {
    pub(super) ssh: SshRuntimeConfig,
    pub(super) control: ControlRuntimeConfig,
    pub(super) policy: TransferPolicy,
}

impl LiveWorkflowBackend {
    pub(super) fn new(
        ssh: SshRuntimeConfig,
        control: ControlRuntimeConfig,
        policy: TransferPolicy,
    ) -> Self {
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
                    &self.control.tls_trust(),
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

    fn remove_local_file(&mut self, local: &str, context: &ErrorContext) -> RosWireResult<()> {
        match fs::remove_file(local) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Box::new(
                RosWireError::file_transfer_failed(format!(
                    "failed to remove local temporary file: {error}"
                ))
                .with_context(context.clone()),
            )),
        }
    }
}

pub(super) fn execute_transfer_for_env(
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

pub(super) fn backend_name<B>(_backend: &B) -> &'static str {
    DEFAULT_TRANSFER_BACKEND
}

pub(super) fn execute_file_workflow<B: WorkflowBackend>(
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

pub(super) fn execute_file_workflow_inner<B: WorkflowBackend>(
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

pub(super) fn execute_with_ssh_service_guard<B, F>(
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

pub(super) fn desired_ssh_service_state(
    snapshot: &SshServiceSnapshot,
    allow_from: &[String],
) -> SshServiceSnapshot {
    SshServiceSnapshot {
        id: snapshot.id.clone(),
        disabled: false,
        address: merge_ssh_allow_list(&snapshot.address, allow_from),
    }
}

pub(super) fn ssh_restore_failed_error(
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

pub(super) fn execute_import_workflow<B: WorkflowBackend>(
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
            let _ = backend.remove_remote_file(&temporary_remote, context);
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

pub(super) struct GeneratedDownloadWorkflow<'a> {
    pub(super) operation: &'a str,
    pub(super) local: &'a str,
    pub(super) remote: String,
    pub(super) control: ControlCommand,
    pub(super) cleanup_remote: bool,
}

pub(super) fn execute_generated_download_workflow<B: WorkflowBackend>(
    spec: GeneratedDownloadWorkflow<'_>,
    policy: &TransferPolicy,
    backend: &mut B,
    context: &ErrorContext,
) -> RosWireResult<TransferResultPayload> {
    let tmp_local = raw_temporary_local_path(spec.local);
    if prepare_local_destination(spec.local, policy, context)? == DestinationDecision::Skip
        || prepare_local_destination(&tmp_local, policy, context)? == DestinationDecision::Skip
    {
        return Ok(skipped_transfer_payload(
            spec.operation,
            DEFAULT_TRANSFER_BACKEND,
            Some(redact_local_path(spec.local)),
            Some(redact_remote_path(&spec.remote)),
        ));
    }

    backend.execute_control(&spec.control, context)?;
    backend.wait_remote_file(&spec.remote, policy.wait_timeout(), context)?;

    let (bytes, checksum_sha256) = match retry_transfer_step(policy, || {
        backend.download(&spec.remote, &tmp_local, context)
    }) {
        Ok(result) => result,
        Err(error) => {
            let _ = backend.remove_local_file(&tmp_local, context);
            return Err(error);
        }
    };
    if let Err(error) = backend.finalize_local_download(&tmp_local, spec.local, context) {
        let _ = backend.remove_local_file(&tmp_local, context);
        return Err(error);
    }

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
