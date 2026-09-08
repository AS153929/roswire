use super::*;

pub(super) fn resolve_ssh_transfer_summary(
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

pub(super) fn resolve_ssh_runtime_config(
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
        jump: crate::jump::resolve_jump_hops(cli, env, profile)?,
    })
}

pub(super) fn key_passphrase_status(
    has_key_path: bool,
    profile: Option<&config::ProfileConfig>,
) -> String {
    if !has_key_path {
        return "not-applicable".to_owned();
    }

    if profile.is_some_and(|profile| profile.secrets.contains_key(SSH_KEY_PASSPHRASE_SECRET)) {
        "provided".to_owned()
    } else {
        "not-provided".to_owned()
    }
}

pub(super) fn resolve_ssh_key_passphrase(
    env: &BTreeMap<String, String>,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<Option<String>> {
    let Some(profile) = profile else {
        return Ok(None);
    };

    config::resolve_profile_secret_value(profile, SSH_KEY_PASSPHRASE_SECRET, env)
}

pub(super) fn resolve_ssh_password(
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

pub(super) fn resolve_control_runtime_config(
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

    let tls_cert_fingerprint = cli
        .tls_cert_fingerprint
        .clone()
        .or_else(|| profile.and_then(|profile| profile.tls_cert_fingerprint.clone()));

    Ok(ControlRuntimeConfig {
        host,
        port,
        user,
        password,
        selected_protocol,
        tls_cert_fingerprint,
        jump: crate::jump::resolve_jump_hops(cli, env, profile)?,
    })
}

pub(super) fn resolve_control_password(
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

pub(super) fn validate_control_protocol(value: &str) -> RosWireResult<()> {
    match value {
        "auto" | "api" | "api-ssl" | "rest" => Ok(()),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid protocol value: {value}",
        )))),
    }
}

pub(super) fn default_control_port(protocol: &str) -> u16 {
    match protocol {
        "api-ssl" => 8729,
        "rest" => 443,
        _ => 8728,
    }
}

fn with_control_jump<T>(
    control: &ControlRuntimeConfig,
    context: &ErrorContext,
    f: impl FnOnce(crate::jump::JumpIo) -> RosWireResult<T>,
) -> RosWireResult<T> {
    let io = crate::jump::open_channel(
        &control.jump,
        &control.host,
        control.port,
        Duration::from_secs(10),
        context,
    )?;
    let leftover = io.leftover_handle();
    crate::jump::finish_with_leftover(f(io), &leftover)
}

pub(super) fn execute_rest_control(
    command: &ControlCommand,
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<()> {
    let (path, body) = command.rest_request();
    if !control.jump.is_empty() {
        return with_control_jump(control, &context, |io| {
            let mut tls = crate::protocol::classic::transport::wrap_tls_stream(
                io,
                &control.host,
                &control.tls_trust(),
            )?;
            crate::protocol::rest::send_on_stream(
                &mut tls,
                &control.host,
                control.port,
                &control.user,
                &control.password,
                crate::mapping::RestMethod::Post,
                path,
                Some(&body),
            )
            .map(|_| ())
        })
        .map_err(|error| Box::new((*error).clone().with_context(context)));
    }
    let client = RestClient::https(
        &control.host,
        control.port,
        &control.user,
        &control.password,
        &control.tls_trust(),
    )
    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    client
        .post_json(path, body)
        .map(|_| ())
        .map_err(|error| Box::new((*error).clone().with_context(context)))
}

pub(super) fn execute_classic_control<S: ApiStream>(
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

pub(super) fn read_ssh_service(
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<SshServiceSnapshot> {
    if !control.jump.is_empty() {
        return with_control_jump(control, &context, |io| {
            match control.selected_protocol.as_str() {
                "rest" => {
                    let mut tls = crate::protocol::classic::transport::wrap_tls_stream(
                        io,
                        &control.host,
                        &control.tls_trust(),
                    )?;
                    let value = crate::protocol::rest::get_on_stream(
                        &mut tls,
                        &control.host,
                        control.port,
                        &control.user,
                        &control.password,
                        "/rest/ip/service",
                    )?;
                    ssh_service_snapshot_from_json(&value)
                }
                "api-ssl" => {
                    let stream = crate::protocol::classic::transport::wrap_tls_stream(
                        io,
                        &control.host,
                        &control.tls_trust(),
                    )?;
                    read_classic_ssh_service(stream, control, context.clone())
                }
                _ => read_classic_ssh_service(io, control, context.clone()),
            }
        });
    }
    match control.selected_protocol.as_str() {
        "rest" => read_rest_ssh_service(control, context),
        "api-ssl" => {
            let stream = TlsApiStream::connect(
                &control.host,
                control.port,
                Duration::from_secs(10),
                &control.tls_trust(),
            )
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

pub(super) fn apply_ssh_service(
    control: &ControlRuntimeConfig,
    desired: &SshServiceSnapshot,
    context: ErrorContext,
) -> RosWireResult<()> {
    if !control.jump.is_empty() {
        return with_control_jump(control, &context, |io| {
            match control.selected_protocol.as_str() {
                "rest" => {
                    let id = desired.id.as_deref().unwrap_or("ssh");
                    let mut tls = crate::protocol::classic::transport::wrap_tls_stream(
                        io,
                        &control.host,
                        &control.tls_trust(),
                    )?;
                    crate::protocol::rest::send_on_stream(
                        &mut tls,
                        &control.host,
                        control.port,
                        &control.user,
                        &control.password,
                        crate::mapping::RestMethod::Patch,
                        &format!("/rest/ip/service/{id}"),
                        Some(&serde_json::json!({
                            "disabled": routeros_bool(desired.disabled),
                            "address": desired.address.join(","),
                        })),
                    )
                    .map(|_| ())
                }
                "api-ssl" => {
                    let stream = crate::protocol::classic::transport::wrap_tls_stream(
                        io,
                        &control.host,
                        &control.tls_trust(),
                    )?;
                    apply_classic_ssh_service(stream, control, desired, context.clone())
                }
                _ => apply_classic_ssh_service(io, control, desired, context.clone()),
            }
        });
    }
    match control.selected_protocol.as_str() {
        "rest" => apply_rest_ssh_service(control, desired, context),
        "api-ssl" => {
            let stream = TlsApiStream::connect(
                &control.host,
                control.port,
                Duration::from_secs(10),
                &control.tls_trust(),
            )
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

pub(super) fn read_classic_ssh_service<S: ApiStream>(
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

pub(super) fn apply_classic_ssh_service<S: ApiStream>(
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

pub(super) fn read_rest_ssh_service(
    control: &ControlRuntimeConfig,
    context: ErrorContext,
) -> RosWireResult<SshServiceSnapshot> {
    let client = RestClient::https(
        &control.host,
        control.port,
        &control.user,
        &control.password,
        &control.tls_trust(),
    )
    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    let value = client
        .get("/rest/ip/service")
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    ssh_service_snapshot_from_json(&value).map_err(|error| Box::new((*error).with_context(context)))
}

pub(super) fn apply_rest_ssh_service(
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
        &control.tls_trust(),
    )
    .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
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

pub(super) fn execute_upload(
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

    with_ssh_session(config, policy, context, |session| {
        let sftp = match session.sftp() {
            Ok(sftp) => sftp,
            Err(error) => {
                return sftp_or_scp_fallback(
                    "upload",
                    Err(sftp_session_unavailable_error(error, context)),
                    || execute_scp_upload(session, local, remote, metadata.len(), context),
                    context,
                );
            }
        };

        execute_sftp_upload(&sftp, local, remote, context)
    })
}

pub(super) fn execute_sftp_upload(
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

pub(super) fn execute_scp_upload(
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

pub(super) fn execute_download(
    remote: &str,
    local: &str,
    config: &SshRuntimeConfig,
    policy: &TransferPolicy,
    context: &ErrorContext,
) -> RosWireResult<(u64, String)> {
    with_ssh_session(config, policy, context, |session| {
        let sftp = match session.sftp() {
            Ok(sftp) => sftp,
            Err(error) => {
                return sftp_or_scp_fallback(
                    "download",
                    Err(sftp_session_unavailable_error(error, context)),
                    || execute_scp_download(session, remote, local, context),
                    context,
                );
            }
        };

        execute_sftp_download(&sftp, remote, local, context)
    })
}

pub(super) fn execute_sftp_download(
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

pub(super) fn execute_scp_download(
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

pub(super) fn sftp_session_unavailable_error(
    error: ssh2::Error,
    context: &ErrorContext,
) -> Box<RosWireError> {
    Box::new(
        RosWireError::file_transfer_failed(format!("SFTP subsystem is unavailable: {error}"))
            .with_context(context.clone()),
    )
}

pub(super) fn sftp_or_scp_fallback<T, F>(
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

pub(super) fn finish_scp_send(
    channel: &mut ssh2::Channel,
    context: &ErrorContext,
) -> RosWireResult<()> {
    channel.send_eof().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("SCP upload failed to send EOF: {error}"))
                .with_context(context.clone()),
        )
    })?;
    finish_scp_channel(channel, context)
}

pub(super) fn finish_scp_recv(
    channel: &mut ssh2::Channel,
    context: &ErrorContext,
) -> RosWireResult<()> {
    finish_scp_channel(channel, context)
}

pub(super) fn finish_scp_channel(
    channel: &mut ssh2::Channel,
    context: &ErrorContext,
) -> RosWireResult<()> {
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

pub(super) fn with_ssh_session<T>(
    config: &SshRuntimeConfig,
    policy: &TransferPolicy,
    context: &ErrorContext,
    f: impl FnOnce(&ssh2::Session) -> RosWireResult<T>,
) -> RosWireResult<T> {
    if !config.jump.is_empty() {
        let io = crate::jump::open_channel(
            &config.jump,
            &config.host,
            config.port,
            policy.connect_timeout(),
            context,
        )?;
        let leftover = io.leftover_handle();
        let jump_session = io.into_target_ssh_session(
            &config.user,
            config.password.as_deref(),
            config.key_path.as_deref(),
            config.key_passphrase.as_deref(),
            &config.expected_host_key,
            context,
        )?;
        let result = f(&jump_session.session);
        drop(jump_session);
        return crate::jump::finish_with_leftover(result, &leftover);
    }
    let (session, leftover) = open_ssh_session(config, policy, context)?;
    let result = f(&session);
    drop(session);
    match leftover {
        Some(flag) => crate::jump::finish_with_leftover(result, &flag),
        None => result,
    }
}

pub(super) fn open_ssh_session(
    config: &SshRuntimeConfig,
    policy: &TransferPolicy,
    context: &ErrorContext,
) -> RosWireResult<(
    ssh2::Session,
    Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
)> {
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
    let session = open_ssh_session_from_stream(tcp, config, context)?;
    Ok((session, None))
}

fn open_ssh_session_from_stream(
    stream: TcpStream,
    config: &SshRuntimeConfig,
    context: &ErrorContext,
) -> RosWireResult<ssh2::Session> {
    let mut session = ssh2::Session::new().map_err(|error| {
        Box::new(
            RosWireError::file_transfer_failed(format!("failed to create SSH session: {error}"))
                .with_context(context.clone()),
        )
    })?;
    session.set_tcp_stream(stream);
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

pub(super) fn verify_host_key(
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

pub(super) fn host_key_matches(expected: &str, actual: &str) -> bool {
    expected.trim() == actual
}

pub(super) fn sha256_fingerprint(bytes: &[u8]) -> String {
    format!("SHA256:{}", BASE64_NO_PAD.encode(bytes))
}

pub(super) fn copy_with_sha256<R: Read, W: Write>(
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
