use super::*;

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

pub(super) fn direct_transfer_payload(
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

pub(super) fn skipped_transfer_payload(
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

pub(super) fn build_plan_for_env(
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

pub(super) fn resolve_transfer_backend(
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

pub(super) fn plan_from_command(
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

pub(super) fn plan_steps(command: &TransferCommand, cli: &Cli) -> Vec<TransferStep> {
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
