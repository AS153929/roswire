pub mod args;
pub mod config;
pub mod error;
pub mod introspect;
pub mod mapping;
pub mod protocol;
pub mod transfer;
pub mod workflow;

use args::Cli;
use clap::Parser;
use error::{ErrorContext, RosWireResult};
use mapping::ActionKind;
use protocol::classic::{transport::TcpApiStream, ClassicApiSession};
use protocol::rest::RestClient;
use std::collections::BTreeMap;
use std::time::Duration;

pub fn run() -> RosWireResult<()> {
    let cli = Cli::parse();

    if cli.simulate_error {
        return Err(Box::new(
            error::RosWireError::usage("simulated usage error for contract tests")
                .with_hint("remove --simulate-error to continue"),
        ));
    }

    if let Some(result) = config::handle(&cli.tokens, &cli) {
        let payload = result?;
        println!("{payload}");
        return Ok(());
    }

    if let Some(result) = introspect::handle(&cli.tokens, &cli) {
        let payload = result?;
        println!("{payload}");
        return Ok(());
    }

    if let Some(result) = transfer::handle(&cli.tokens, &cli) {
        let payload = result?;
        println!("{payload}");
        return Ok(());
    }

    let invocation = args::parse_invocation(&cli.tokens)?;
    let request = mapping::build_protocol_request(&invocation)?;

    if request.mapping.action_kind != ActionKind::Print {
        return Err(Box::new(
            error::RosWireError::unsupported_action(format!(
                "RouterOS write action is not implemented: {}",
                mapping::command_name(&invocation),
            ))
            .with_context(unsupported_action_context(&cli, &invocation)),
        ));
    }

    let target = resolve_execution_target(&cli)?;
    if target.requested_protocol == "rest" {
        let context = execution_context(&invocation, &target, "rest");
        let client = RestClient::https(&target.host, target.port, &target.user, &target.password);
        let value = with_context(client.execute_request(&request), context)?;
        let payload = serde_json::to_string(&value).map_err(|error| {
            Box::new(error::RosWireError::internal(format!(
                "failed to serialize RouterOS REST response: {error}",
            )))
        })?;
        println!("{payload}");
        return Ok(());
    }
    if target.requested_protocol == "api-ssl" {
        return Err(Box::new(
            error::RosWireError::network("api-ssl TLS transport is not implemented yet")
                .with_context(execution_context(&invocation, &target, "api-ssl")),
        ));
    }

    let context = execution_context(&invocation, &target, "api");
    let stream = with_context(
        TcpApiStream::connect(&target.host, target.port, Duration::from_secs(10)),
        context.clone(),
    )?;
    let mut session = ClassicApiSession::new(stream);
    with_context(
        session.login(&target.user, &target.password),
        context.clone(),
    )?;
    let rows = with_context(session.execute_request(&request), context)?;
    let payload = serde_json::to_string(&rows).map_err(|error| {
        Box::new(error::RosWireError::internal(format!(
            "failed to serialize RouterOS response: {error}",
        )))
    })?;
    println!("{payload}");

    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutionTarget {
    pub(crate) host: String,
    pub(crate) user: String,
    pub(crate) password: String,
    pub(crate) requested_protocol: String,
    pub(crate) routeros_version: String,
    pub(crate) port: u16,
}

pub(crate) fn resolve_execution_target(cli: &Cli) -> RosWireResult<ExecutionTarget> {
    let env = read_env_map();
    resolve_execution_target_with_env(cli, &env)
}

fn resolve_execution_target_with_env(
    cli: &Cli,
    env: &BTreeMap<String, String>,
) -> RosWireResult<ExecutionTarget> {
    let paths = config::ConfigPaths::from_home(config::resolve_home_path(
        env.get("ROSWIRE_HOME").map(String::as_str),
    ));
    let config_file = if paths.config.exists() {
        config::ensure_secure_directory_permissions(&paths.home)?;
        config::ensure_secure_file_permissions(&paths.config)?;
        Some(config::load_config_file(&paths.config)?)
    } else {
        None
    };

    let env_profile = env.get("ROS_PROFILE").map(String::as_str);
    let selected_profile = match &config_file {
        Some(config_file) => {
            match config::select_active_profile(cli.profile.as_deref(), env_profile, config_file) {
                Ok(profile) => Some(profile),
                Err(error) if cli.profile.is_some() || env_profile.is_some() => return Err(error),
                Err(_) => None,
            }
        }
        None if cli.profile.is_some() => {
            return Err(Box::new(error::RosWireError::profile_not_found(
                cli.profile.clone().expect("profile is checked as Some"),
            )));
        }
        None if env_profile.is_some() => {
            return Err(Box::new(error::RosWireError::profile_not_found(
                env_profile.expect("env profile is checked as Some"),
            )));
        }
        None => None,
    };

    let profile = config_file.as_ref().and_then(|config_file| {
        selected_profile
            .as_ref()
            .and_then(|name| config_file.profiles.get(name))
    });

    let host = cli
        .host
        .clone()
        .or_else(|| env.get("ROS_HOST").cloned())
        .or_else(|| profile.and_then(|profile| profile.host.clone()))
        .ok_or_else(|| {
            Box::new(error::RosWireError::config(
                "missing RouterOS host; set --host, ROS_HOST, or profile host",
            ))
        })?;
    config::validate_remote_host(&host)?;
    let user = cli
        .user
        .clone()
        .or_else(|| env.get("ROS_USER").cloned())
        .or_else(|| profile.and_then(|profile| profile.user.clone()))
        .ok_or_else(|| {
            Box::new(error::RosWireError::config(
                "missing RouterOS user; set --user, ROS_USER, or profile user",
            ))
        })?;
    let password = match cli
        .password
        .clone()
        .or_else(|| env.get("ROS_PASSWORD").cloned())
    {
        Some(password) => password,
        None => match profile {
            Some(profile) => config::resolve_profile_secret_value(profile, "password", env)?
                .ok_or_else(|| {
                    Box::new(error::RosWireError::config(
                        "missing RouterOS password; set --password, ROS_PASSWORD, or profile secret password",
                    ))
                })?,
            None => {
                return Err(Box::new(error::RosWireError::config(
                    "missing RouterOS password; set --password, ROS_PASSWORD, or profile secret password",
                )));
            }
        },
    };

    let requested_protocol = cli
        .protocol
        .map(|value| value.as_str().to_owned())
        .or_else(|| env.get("ROS_PROTOCOL").cloned())
        .or_else(|| profile.and_then(|profile| profile.protocol.clone()))
        .unwrap_or_else(|| "auto".to_owned());
    validate_protocol(&requested_protocol)?;

    let routeros_version = cli
        .routeros_version
        .map(|value| value.as_str().to_owned())
        .or_else(|| env.get("ROS_ROUTEROS_VERSION").cloned())
        .or_else(|| profile.and_then(|profile| profile.routeros_version.clone()))
        .unwrap_or_else(|| "auto".to_owned());
    validate_routeros_version(&routeros_version)?;

    let env_port = match env.get("ROS_PORT") {
        Some(value) => Some(parse_port(value)?),
        None => None,
    };
    let explicit_port = cli
        .port
        .or(env_port)
        .or_else(|| profile.and_then(|profile| profile.port));
    let port = explicit_port.unwrap_or_else(|| default_port(&requested_protocol));

    if requested_protocol == "auto" && explicit_port.is_some() {
        return Err(Box::new(error::RosWireError::config(
            "port cannot be used with --protocol auto",
        )));
    }

    Ok(ExecutionTarget {
        host,
        user,
        password,
        requested_protocol,
        routeros_version,
        port,
    })
}

fn validate_protocol(value: &str) -> RosWireResult<()> {
    match value {
        "auto" | "api" | "api-ssl" | "rest" => Ok(()),
        _ => Err(Box::new(error::RosWireError::usage(format!(
            "invalid protocol value: {value}",
        )))),
    }
}

fn validate_routeros_version(value: &str) -> RosWireResult<()> {
    match value {
        "auto" | "v6" | "v7" => Ok(()),
        _ => Err(Box::new(error::RosWireError::usage(format!(
            "invalid routeros_version value: {value}",
        )))),
    }
}

fn parse_port(value: &str) -> RosWireResult<u16> {
    value.parse::<u16>().map_err(|error| {
        Box::new(error::RosWireError::usage(format!(
            "invalid port value `{value}`: {error}",
        )))
    })
}

fn default_port(protocol: &str) -> u16 {
    match protocol {
        "api-ssl" => 8729,
        "rest" => 443,
        _ => 8728,
    }
}

fn read_env_map() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

fn with_context<T>(result: RosWireResult<T>, context: ErrorContext) -> RosWireResult<T> {
    result.map_err(|error| Box::new((*error).clone().with_context(context)))
}

fn execution_context(
    invocation: &args::ParsedInvocation,
    target: &ExecutionTarget,
    selected_protocol: &str,
) -> ErrorContext {
    ErrorContext {
        command: mapping::command_name(invocation),
        path: invocation.path.clone(),
        action: invocation.action.clone(),
        requested_protocol: target.requested_protocol.clone(),
        selected_protocol: selected_protocol.to_owned(),
        transfer_backend: None,
        routeros_version: target.routeros_version.clone(),
        host: target.host.clone(),
        resolved_args: error::redact_resolved_args(&invocation.resolved_args),
    }
}

fn unsupported_action_context(cli: &Cli, invocation: &args::ParsedInvocation) -> ErrorContext {
    ErrorContext {
        command: unsupported_command_name(invocation),
        path: invocation.path.clone(),
        action: invocation.action.clone(),
        requested_protocol: cli
            .protocol
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| "auto".to_owned()),
        selected_protocol: "unknown".to_owned(),
        transfer_backend: cli.transfer.map(|value| value.as_str().to_owned()),
        routeros_version: cli
            .routeros_version
            .map(|value| value.as_str().to_owned())
            .unwrap_or_else(|| "auto".to_owned()),
        host: cli
            .host
            .clone()
            .or_else(|| std::env::var("ROS_HOST").ok())
            .unwrap_or_default(),
        resolved_args: error::redact_resolved_args(&invocation.resolved_args),
    }
}

fn unsupported_command_name(invocation: &args::ParsedInvocation) -> String {
    invocation
        .path
        .iter()
        .chain(std::iter::once(&invocation.action))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{
        default_port, execution_context, parse_port, resolve_execution_target_with_env,
        unsupported_command_name, validate_protocol, validate_routeros_version, with_context,
        ExecutionTarget,
    };
    use crate::args::{Cli, ParsedInvocation};
    use crate::error::{ErrorCode, RosWireError};
    use clap::Parser;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn execution_target_uses_cli_values_and_defaults() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "192.0.2.1",
            "--user",
            "admin",
            "--password",
            "test-value",
            "interface",
            "print",
        ])
        .expect("cli should parse");
        let env = isolated_env();

        let target = resolve_execution_target_with_env(&cli, &env).expect("target should resolve");

        assert_eq!(target.host, "192.0.2.1");
        assert_eq!(target.user, "admin");
        assert_eq!(target.password, "test-value");
        assert_eq!(target.requested_protocol, "auto");
        assert_eq!(target.routeros_version, "auto");
        assert_eq!(target.port, 8728);
    }

    #[test]
    fn execution_target_uses_env_over_profile() {
        let (temp, env_home) = temp_home_env();
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "profile-user"
protocol = "api"
routeros_version = "v6"
port = 8728
allow_plain_secrets = true

[profiles.studio.secrets.password]
type = "plain"
value = "profile-value"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "interface", "print"]).expect("cli should parse");
        let mut env = env_home;
        env.insert("ROS_HOST".to_owned(), "203.0.113.9".to_owned());
        env.insert("ROS_USER".to_owned(), "env-user".to_owned());
        env.insert("ROS_PASSWORD".to_owned(), "env-value".to_owned());
        env.insert("ROS_PROTOCOL".to_owned(), "api-ssl".to_owned());
        env.insert("ROS_ROUTEROS_VERSION".to_owned(), "v7".to_owned());
        env.insert("ROS_PORT".to_owned(), "8729".to_owned());

        let target = resolve_execution_target_with_env(&cli, &env).expect("target should resolve");

        assert_eq!(target.host, "203.0.113.9");
        assert_eq!(target.user, "env-user");
        assert_eq!(target.password, "env-value");
        assert_eq!(target.requested_protocol, "api-ssl");
        assert_eq!(target.routeros_version, "v7");
        assert_eq!(target.port, 8729);
    }

    #[test]
    fn execution_target_rejects_mac_host_before_network() {
        let (temp, env) = temp_home_env();
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "48-8F-5A-A3-0E-A7"
user = "profile-user"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "interface", "print"]).expect("cli should parse");

        let error = resolve_execution_target_with_env(&cli, &env)
            .expect_err("MAC host should fail before any network connection");

        assert_eq!(error.error_code, ErrorCode::ConfigError);
        assert_eq!(error.context.host, "48-8F-5A-A3-0E-A7");
        assert!(error.message.contains("MAC address"));
    }

    #[test]
    fn execution_target_resolves_profile_plain_and_same_as_secret() {
        let (temp, env) = temp_home_env();
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "profile-user"
protocol = "api"
routeros_version = "v7"
allow_plain_secrets = true

[profiles.studio.secrets.actual]
type = "plain"
value = "profile-value"

[profiles.studio.secrets.password]
type = "same-as"
target = "actual"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "interface", "print"]).expect("cli should parse");

        let target = resolve_execution_target_with_env(&cli, &env).expect("target should resolve");

        assert_eq!(target.host, "198.51.100.10");
        assert_eq!(target.user, "profile-user");
        assert_eq!(target.password, "profile-value");
        assert_eq!(target.requested_protocol, "api");
        assert_eq!(target.routeros_version, "v7");
        assert_eq!(target.port, 8728);
    }

    #[test]
    fn execution_target_rejects_auto_with_explicit_port() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "192.0.2.1",
            "--user",
            "admin",
            "--password",
            "test-value",
            "--port",
            "8728",
            "interface",
            "print",
        ])
        .expect("cli should parse");
        let env = isolated_env();

        let error =
            resolve_execution_target_with_env(&cli, &env).expect_err("auto port should fail");

        assert_eq!(error.error_code, ErrorCode::ConfigError);
    }

    #[test]
    fn execution_target_reports_missing_env_secret() {
        let (temp, env) = temp_home_env();
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "profile-user"

[profiles.studio.secrets.password]
type = "env"
var = "ROSWIRE_TEST_PASSWORD"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "interface", "print"]).expect("cli should parse");

        let error = resolve_execution_target_with_env(&cli, &env)
            .expect_err("missing env secret should fail");

        assert_eq!(error.error_code, ErrorCode::SecretNotFound);
        assert!(error.message.contains("ROSWIRE_TEST_PASSWORD"));
    }

    #[test]
    fn execution_target_reports_missing_encrypted_master_key() {
        let (temp, env) = temp_home_env();
        write_config(
            temp.path(),
            r#"
version = 1
default_profile = "studio"

[profiles.studio]
host = "198.51.100.10"
user = "profile-user"

[profiles.studio.secrets.password]
type = "encrypted"
value = "v1:nonce:ciphertext"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "interface", "print"]).expect("cli should parse");

        let error = resolve_execution_target_with_env(&cli, &env)
            .expect_err("missing master key should fail");

        assert_eq!(error.error_code, ErrorCode::SecretBackendUnavailable);
        assert!(error.message.contains("ROSWIRE_MASTER_KEY"));
    }

    #[test]
    fn validation_helpers_cover_protocol_version_and_ports() {
        validate_protocol("auto").expect("auto protocol is valid");
        validate_protocol("api").expect("api protocol is valid");
        assert_eq!(
            validate_protocol("bogus")
                .expect_err("invalid protocol should fail")
                .error_code,
            ErrorCode::UsageError,
        );
        validate_routeros_version("v6").expect("v6 is valid");
        assert_eq!(
            validate_routeros_version("v8")
                .expect_err("invalid version should fail")
                .error_code,
            ErrorCode::UsageError,
        );
        assert_eq!(parse_port("8728").expect("port should parse"), 8728);
        assert_eq!(
            parse_port("bad")
                .expect_err("bad port should fail")
                .error_code,
            ErrorCode::UsageError,
        );
        assert_eq!(default_port("api"), 8728);
        assert_eq!(default_port("api-ssl"), 8729);
        assert_eq!(default_port("rest"), 443);
    }

    #[test]
    fn context_helpers_attach_execution_details() {
        let invocation = ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::from([("password".to_owned(), "test-value".to_owned())]),
        };
        let target = ExecutionTarget {
            host: "router.local".to_owned(),
            user: "admin".to_owned(),
            password: "test-value".to_owned(),
            requested_protocol: "auto".to_owned(),
            routeros_version: "v7".to_owned(),
            port: 8728,
        };

        let context = execution_context(&invocation, &target, "api");
        assert_eq!(unsupported_command_name(&invocation), "ip/address/print");
        assert_eq!(context.command, "ip/address/print");
        assert_eq!(context.selected_protocol, "api");
        assert_eq!(
            context.resolved_args.get("password").map(String::as_str),
            Some("***REDACTED***"),
        );

        let error = with_context::<()>(
            Err(Box::new(RosWireError::network("unreachable"))),
            context.clone(),
        )
        .expect_err("context should attach");
        assert_eq!(error.context.command, context.command);
    }

    fn isolated_env() -> BTreeMap<String, String> {
        let (temp, mut env) = temp_home_env();
        env.insert(
            "ROSWIRE_HOME".to_owned(),
            temp.path().join("missing-home").display().to_string(),
        );
        env
    }

    fn temp_home_env() -> (tempfile::TempDir, BTreeMap<String, String>) {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);
        (temp, env)
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
