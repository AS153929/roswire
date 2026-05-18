use crate::args::Cli;
use crate::config::{self, ConfigFile, ConfigInspectPaths, ConfigPaths, SecretInspectField};
use crate::error::{ErrorCode, RosWireError, RosWireResult};
use crate::protocol::classic::{
    transport::{TcpApiStream, TlsApiStream},
    ClassicApiSession,
};
use serde::Serialize;
use serde_json::Value;
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
    _env: &BTreeMap<String, String>,
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
        match config::select_active_profile(cli.profile.as_deref(), config_file) {
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
        "auto" => return remote_doctor_auto(&target),
        "rest" => {
            return remote_doctor_rest(&target, target.port);
        }
        "api-ssl" => {
            let stream =
                match TlsApiStream::connect(&target.host, target.port, Duration::from_secs(10)) {
                    Ok(stream) => stream,
                    Err(error) => return remote_error("api-ssl", &error),
                };
            return probe_classic_remote(stream, &target.user, &target.password, "api-ssl");
        }
        _ => {}
    }

    let stream = match TcpApiStream::connect(&target.host, target.port, Duration::from_secs(10)) {
        Ok(stream) => stream,
        Err(error) => return remote_error("api", &error),
    };
    probe_classic_remote(stream, &target.user, &target.password, "api")
}

fn remote_doctor_auto(target: &crate::ExecutionTarget) -> RemoteDoctor {
    match probe_rest_resource(target, crate::default_port("rest")) {
        Ok(remote) => return remote,
        Err(error) if error.error_code == ErrorCode::AuthFailed => {
            return remote_error("rest", &error)
        }
        Err(_) => {}
    }

    match TlsApiStream::connect(
        &target.host,
        crate::default_port("api-ssl"),
        Duration::from_secs(10),
    ) {
        Ok(stream) => {
            return probe_classic_remote(stream, &target.user, &target.password, "api-ssl")
        }
        Err(error) if error.error_code == ErrorCode::AuthFailed => {
            return remote_error("api-ssl", &error);
        }
        Err(_) => {}
    }

    let stream = match TcpApiStream::connect(
        &target.host,
        crate::default_port("api"),
        Duration::from_secs(10),
    ) {
        Ok(stream) => stream,
        Err(error) => return remote_error("api", &error),
    };
    probe_classic_remote(stream, &target.user, &target.password, "api")
}

fn remote_doctor_rest(target: &crate::ExecutionTarget, port: u16) -> RemoteDoctor {
    match probe_rest_resource(target, port) {
        Ok(remote) => remote,
        Err(error) => remote_error("rest", &error),
    }
}

fn probe_rest_resource(
    target: &crate::ExecutionTarget,
    port: u16,
) -> Result<RemoteDoctor, Box<RosWireError>> {
    let client = crate::protocol::rest::RestClient::https(
        &target.host,
        port,
        &target.user,
        &target.password,
    );
    client
        .system_resource()
        .map(|value| remote_doctor_from_rest_resource("rest", &value))
}

fn remote_doctor_from_rest_resource(selected_protocol: &str, value: &Value) -> RemoteDoctor {
    RemoteDoctor {
        status: "ok".to_owned(),
        selected_protocol: selected_protocol.to_owned(),
        routeros_version: string_field(value, "version"),
        architecture: string_field(value, "architecture-name")
            .or_else(|| string_field(value, "architecture")),
        board_name: string_field(value, "board-name"),
        error_code: None,
        message: None,
        warnings: Vec::new(),
    }
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(str::to_owned)
}

fn probe_classic_remote<S: crate::protocol::classic::transport::ApiStream>(
    stream: S,
    user: &str,
    password: &str,
    selected_protocol: &str,
) -> RemoteDoctor {
    let mut session = ClassicApiSession::new(stream);
    if let Err(error) = session.login(user, password) {
        return remote_error(selected_protocol, &error);
    }
    match session.probe_resource() {
        Ok(resource) => RemoteDoctor {
            status: "ok".to_owned(),
            selected_protocol: selected_protocol.to_owned(),
            routeros_version: Some(resource.version),
            architecture: Some(resource.architecture),
            board_name: Some(resource.board_name),
            error_code: None,
            message: None,
            warnings: Vec::new(),
        },
        Err(error) => remote_error(selected_protocol, &error),
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
    _env: &BTreeMap<String, String>,
    config_file: Option<&ConfigFile>,
) -> Option<String> {
    cli.routeros_version
        .map(|value| value.as_str().to_owned())
        .or_else(|| {
            let config_file = config_file?;
            let profile_name =
                config::select_active_profile(cli.profile.as_deref(), config_file).ok()?;
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
        ("api_ssl_tls".to_owned(), "available".to_owned()),
        ("rest_client".to_owned(), "available".to_owned()),
        ("rest_remote_doctor".to_owned(), "available".to_owned()),
        ("protocol_auto_probe".to_owned(), "available".to_owned()),
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
        ("remote_schema_overlay".to_owned(), "available".to_owned()),
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
    serde_json::to_string_pretty(value).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize doctor payload: {error}",
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        error_code_name, local_dependencies, local_doctor, local_routeros_version_hint,
        remote_error, string_field,
    };
    use crate::args::{Cli, RouterOsVersionMode};
    use crate::config::ConfigPaths;
    use crate::error::{ErrorCode, RosWireError};
    use clap::Parser;
    use std::collections::BTreeMap;
    use std::fs;

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
            dependencies.get("protocol_auto_probe").map(String::as_str),
            Some("available"),
        );
        assert_eq!(
            dependencies.get("rest_remote_doctor").map(String::as_str),
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
            Some("available"),
        );
        assert_eq!(
            dependencies.get("ssh_transfer_runtime").map(String::as_str),
            Some("not_implemented"),
        );
        assert_eq!(
            dependencies
                .get("remote_schema_overlay")
                .map(String::as_str),
            Some("available"),
        );
    }

    #[test]
    fn rest_remote_doctor_parses_resource_payload() {
        let payload = serde_json::json!({
            "version": "7.15.3",
            "architecture-name": "arm64",
            "board-name": "RB5009",
        });

        let remote = super::remote_doctor_from_rest_resource("rest", &payload);

        assert_eq!(remote.status, "ok");
        assert_eq!(remote.selected_protocol, "rest");
        assert_eq!(remote.routeros_version.as_deref(), Some("7.15.3"));
        assert_eq!(remote.architecture.as_deref(), Some("arm64"));
        assert_eq!(remote.board_name.as_deref(), Some("RB5009"));
    }

    #[test]
    fn string_field_reads_strings_only() {
        let payload = serde_json::json!({
            "version": "7.15.3",
            "uptime": 123,
        });

        assert_eq!(string_field(&payload, "version").as_deref(), Some("7.15.3"));
        assert_eq!(string_field(&payload, "uptime"), None);
        assert_eq!(string_field(&payload, "missing"), None);
    }

    #[test]
    fn local_doctor_reads_existing_config_profile_and_secret_status() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "admin"
allow_plain_secrets = true

[profiles.studio.secrets.password]
type = "plain"
value = "test-value"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "doctor", "--json"]).expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);
        let paths = ConfigPaths::from_home(temp.path().to_path_buf());

        let (local, config_file) = local_doctor(&cli, &env, &paths).expect("doctor should work");

        assert!(config_file.is_some());
        assert!(local.home_exists);
        assert!(local.config_exists);
        assert!(local.permissions_ok);
        assert_eq!(local.active_profile.as_deref(), Some("studio"));
        assert_eq!(local.profiles, vec!["studio".to_owned()]);
        assert_eq!(
            local
                .secret_status
                .get("password")
                .map(|secret| secret.secret_type.as_str()),
            Some("plain"),
        );
        assert!(local.warnings.is_empty());
    }

    #[test]
    fn local_doctor_records_profile_and_secret_warnings_without_failing() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "admin"

[profiles.studio.secrets.password]
type = "plain"
value = "test-value"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "--profile", "missing", "doctor", "--json"])
            .expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);
        let paths = ConfigPaths::from_home(temp.path().to_path_buf());

        let (local, config_file) = local_doctor(&cli, &env, &paths).expect("doctor should degrade");

        assert!(config_file.is_some());
        assert_eq!(local.active_profile, None);
        assert!(local
            .warnings
            .iter()
            .any(|warning| warning == "PROFILE_NOT_FOUND"));

        let cli = Cli::try_parse_from(["roswire", "doctor", "--json"]).expect("cli should parse");
        let (local, _) = local_doctor(&cli, &env, &paths).expect("doctor should collect warnings");
        assert!(local
            .warnings
            .iter()
            .any(|warning| warning == "CONFIG_ERROR"));
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
}
