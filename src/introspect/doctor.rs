use crate::args::Cli;
use crate::config::{self, ConfigFile, ConfigInspectPaths, ConfigPaths, SecretInspectField};
use crate::error::{ErrorCode, RosWireError, RosWireResult};
use crate::protocol::classic::{transport::TcpApiStream, ClassicApiSession};
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct DoctorPayload {
    pub schema_version: &'static str,
    pub local: LocalDoctor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteDoctor>,
    pub selected_protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routeros_version: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LocalDoctor {
    pub paths: ConfigInspectPaths,
    pub home_exists: bool,
    pub config_exists: bool,
    pub logs_exists: bool,
    pub permissions_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
    pub profiles: Vec<String>,
    pub secret_status: BTreeMap<String, SecretInspectField>,
    pub dependencies: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RemoteDoctor {
    pub status: String,
    pub selected_protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routeros_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub warnings: Vec<String>,
}

pub fn doctor_payload(cli: &Cli) -> RosWireResult<String> {
    let env = read_env_map();
    let paths = ConfigPaths::from_home(config::resolve_home_path(
        env.get("ROSWIRE_HOME").map(String::as_str),
    ));
    let (local, config_file) = local_doctor(cli, &env, &paths)?;
    let remote = if cli.include_remote {
        Some(remote_doctor(cli))
    } else {
        None
    };

    let selected_protocol = remote
        .as_ref()
        .map(|remote| remote.selected_protocol.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let routeros_version = remote
        .as_ref()
        .and_then(|remote| remote.routeros_version.clone())
        .or_else(|| local_routeros_version_hint(cli, &env, config_file.as_ref()));
    let mut warnings = local.warnings.clone();
    if let Some(remote) = &remote {
        warnings.extend(remote.warnings.clone());
    }

    render_json(&DoctorPayload {
        schema_version: "roswire.doctor.v1",
        local,
        remote,
        selected_protocol,
        routeros_version,
        warnings,
    })
}

fn local_doctor(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    paths: &ConfigPaths,
) -> RosWireResult<(LocalDoctor, Option<ConfigFile>)> {
    let home_exists = paths.home.exists();
    let config_exists = paths.config.exists();
    let logs_exists = paths.logs.exists();
    let mut permissions_ok = true;
    let mut warnings = Vec::new();

    if !home_exists {
        warnings.push("HOME_MISSING".to_owned());
        permissions_ok = false;
    } else if let Err(error) = config::ensure_secure_directory_permissions(&paths.home) {
        warnings.push(error_code_name(error.error_code));
        permissions_ok = false;
    }

    let config_file = if config_exists {
        if let Err(error) = config::ensure_secure_file_permissions(&paths.config) {
            warnings.push(error_code_name(error.error_code));
            permissions_ok = false;
        }
        Some(config::load_config_file(&paths.config)?)
    } else {
        warnings.push("CONFIG_MISSING".to_owned());
        None
    };

    let mut profiles = Vec::new();
    let mut active_profile = None;
    let mut secret_status = BTreeMap::new();

    if let Some(config_file) = &config_file {
        profiles = config_file.profiles.keys().cloned().collect();
        match config::select_active_profile(
            cli.profile.as_deref(),
            env.get("ROS_PROFILE").map(String::as_str),
            config_file,
        ) {
            Ok(profile_name) => {
                if let Some(profile) = config_file.profiles.get(&profile_name) {
                    match config::resolve_profile_secrets(profile) {
                        Ok(secrets) => secret_status = secrets,
                        Err(error) => warnings.push(error_code_name(error.error_code)),
                    }
                }
                active_profile = Some(profile_name);
            }
            Err(error) => warnings.push(error_code_name(error.error_code)),
        }
    }

    Ok((
        LocalDoctor {
            paths: ConfigInspectPaths {
                home: paths.home.display().to_string(),
                config: paths.config.display().to_string(),
                logs: paths.logs.display().to_string(),
            },
            home_exists,
            config_exists,
            logs_exists,
            permissions_ok,
            active_profile,
            profiles,
            secret_status,
            dependencies: local_dependencies(),
            warnings,
        },
        config_file,
    ))
}

fn remote_doctor(cli: &Cli) -> RemoteDoctor {
    let target = match crate::resolve_execution_target(cli) {
        Ok(target) => target,
        Err(error) => return remote_error("unknown", &error),
    };

    match target.requested_protocol.as_str() {
        "rest" => {
            return remote_error(
                "unknown",
                &RosWireError::unsupported_action("REST remote doctor is not implemented yet"),
            );
        }
        "api-ssl" => {
            return remote_error(
                "api-ssl",
                &RosWireError::network("api-ssl TLS transport is not implemented yet"),
            );
        }
        _ => {}
    }

    let stream = match TcpApiStream::connect(&target.host, target.port, Duration::from_secs(10)) {
        Ok(stream) => stream,
        Err(error) => return remote_error("api", &error),
    };
    let mut session = ClassicApiSession::new(stream);
    if let Err(error) = session.login(&target.user, &target.password) {
        return remote_error("api", &error);
    }
    match session.probe_resource() {
        Ok(resource) => RemoteDoctor {
            status: "ok".to_owned(),
            selected_protocol: "api".to_owned(),
            routeros_version: Some(resource.version),
            architecture: Some(resource.architecture),
            board_name: Some(resource.board_name),
            error_code: None,
            message: None,
            warnings: Vec::new(),
        },
        Err(error) => remote_error("api", &error),
    }
}

fn remote_error(selected_protocol: &str, error: &RosWireError) -> RemoteDoctor {
    RemoteDoctor {
        status: "error".to_owned(),
        selected_protocol: selected_protocol.to_owned(),
        routeros_version: None,
        architecture: None,
        board_name: None,
        error_code: Some(error_code_name(error.error_code)),
        message: Some(error.message.clone()),
        warnings: vec![error_code_name(error.error_code)],
    }
}

fn local_routeros_version_hint(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    config_file: Option<&ConfigFile>,
) -> Option<String> {
    cli.routeros_version
        .map(|value| value.as_str().to_owned())
        .or_else(|| env.get("ROS_ROUTEROS_VERSION").cloned())
        .or_else(|| {
            let config_file = config_file?;
            let profile_name = config::select_active_profile(
                cli.profile.as_deref(),
                env.get("ROS_PROFILE").map(String::as_str),
                config_file,
            )
            .ok()?;
            config_file
                .profiles
                .get(&profile_name)
                .and_then(|profile| profile.routeros_version.clone())
        })
}

fn local_dependencies() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("classic_api_tcp".to_owned(), "available".to_owned()),
        (
            "classic_api_sentence_codec".to_owned(),
            "available".to_owned(),
        ),
        ("classic_api_login".to_owned(), "available".to_owned()),
        ("api_ssl_tls".to_owned(), "not_implemented".to_owned()),
        ("rest_client".to_owned(), "available".to_owned()),
        (
            "rest_remote_doctor".to_owned(),
            "not_implemented".to_owned(),
        ),
        ("plain_secret_backend".to_owned(), "available".to_owned()),
        ("env_secret_backend".to_owned(), "available".to_owned()),
        (
            "encrypted_secret_backend".to_owned(),
            "available".to_owned(),
        ),
        ("keychain_backend".to_owned(), "available".to_owned()),
        ("ssh_transfer_dry_run".to_owned(), "available".to_owned()),
        (
            "ssh_transfer_runtime".to_owned(),
            "not_implemented".to_owned(),
        ),
        (
            "remote_schema_overlay".to_owned(),
            "not_implemented".to_owned(),
        ),
    ])
}

fn error_code_name(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "INTERNAL_ERROR".to_owned())
}

fn read_env_map() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

fn render_json<T: Serialize>(value: &T) -> RosWireResult<String> {
    serde_json::to_string(value).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize doctor payload: {error}",
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::{error_code_name, local_dependencies, local_routeros_version_hint, remote_error};
    use crate::args::{Cli, RouterOsVersionMode};
    use crate::error::{ErrorCode, RosWireError};
    use clap::Parser;
    use std::collections::BTreeMap;

    #[test]
    fn local_dependencies_are_stable() {
        let dependencies = local_dependencies();
        assert_eq!(
            dependencies.get("classic_api_tcp").map(String::as_str),
            Some("available"),
        );
        assert_eq!(
            dependencies.get("rest_client").map(String::as_str),
            Some("available"),
        );
        assert_eq!(
            dependencies.get("keychain_backend").map(String::as_str),
            Some("available"),
        );
        assert_eq!(
            dependencies
                .get("encrypted_secret_backend")
                .map(String::as_str),
            Some("available"),
        );
        assert_eq!(
            dependencies.get("api_ssl_tls").map(String::as_str),
            Some("not_implemented"),
        );
        assert_eq!(
            dependencies.get("ssh_transfer_runtime").map(String::as_str),
            Some("not_implemented"),
        );
    }

    #[test]
    fn local_routeros_version_prefers_cli_hint() {
        let cli = Cli::try_parse_from(["roswire", "--routeros-version", "v7", "doctor", "--json"])
            .expect("cli should parse");
        assert_eq!(cli.routeros_version, Some(RouterOsVersionMode::V7));

        let hint = local_routeros_version_hint(&cli, &BTreeMap::new(), None);

        assert_eq!(hint.as_deref(), Some("v7"));
    }

    #[test]
    fn remote_error_keeps_error_classification_in_payload() {
        let remote = remote_error("api", &RosWireError::network("unreachable"));

        assert_eq!(remote.status, "error");
        assert_eq!(remote.selected_protocol, "api");
        assert_eq!(remote.error_code.as_deref(), Some("NETWORK_ERROR"));
        assert!(remote.warnings.iter().any(|item| item == "NETWORK_ERROR"));
    }

    #[test]
    fn error_code_names_are_stable() {
        assert_eq!(error_code_name(ErrorCode::ConfigError), "CONFIG_ERROR");
    }
}
