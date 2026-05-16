use crate::args::Cli;
use crate::error::{ErrorCode, RosWireError, RosWireResult};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn default_config_version() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConfigFile {
    #[serde(default = "default_config_version")]
    pub version: u32,
    #[serde(default)]
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileConfig {
    pub host: Option<String>,
    pub user: Option<String>,
    pub protocol: Option<String>,
    pub routeros_version: Option<String>,
    pub transfer: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub logs: PathBuf,
}

impl ConfigPaths {
    pub fn from_home(home: PathBuf) -> Self {
        Self {
            config: home.join("config.toml"),
            logs: home.join("logs"),
            home,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValueSource {
    Cli,
    Env,
    Profile,
    Default,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResolvedField {
    pub value: String,
    pub source: ValueSource,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConfigInspectPaths {
    pub home: String,
    pub config: String,
    pub logs: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConfigInspect {
    pub schema_version: String,
    pub active_profile: String,
    pub paths: ConfigInspectPaths,
    pub resolved: BTreeMap<String, ResolvedField>,
    pub warnings: Vec<String>,
}

pub fn default_roswire_home() -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".roswire"))
        .unwrap_or_else(|| PathBuf::from(".roswire"))
}

pub fn resolve_home_path(env_home: Option<&str>) -> PathBuf {
    env_home
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(default_roswire_home)
}

pub fn parse_config_toml(contents: &str) -> RosWireResult<ConfigFile> {
    toml::from_str(contents)
        .map_err(|err| Box::new(RosWireError::config(format!("invalid config.toml: {err}"))))
}

pub fn load_config_file(path: &Path) -> RosWireResult<ConfigFile> {
    let contents = fs::read_to_string(path).map_err(|err| {
        Box::new(RosWireError::config(format!(
            "failed to read config file: {err}"
        )))
    })?;
    parse_config_toml(&contents)
}

pub fn select_active_profile(
    cli_profile: Option<&str>,
    env_profile: Option<&str>,
    config: &ConfigFile,
) -> RosWireResult<String> {
    let selected = cli_profile
        .map(str::to_owned)
        .or_else(|| env_profile.map(str::to_owned))
        .or_else(|| config.default_profile.clone())
        .or_else(|| {
            if config.profiles.len() == 1 {
                config.profiles.keys().next().cloned()
            } else {
                None
            }
        })
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "no profile selected; set --profile, ROS_PROFILE, or default_profile",
            ))
        })?;

    if config.profiles.contains_key(&selected) {
        Ok(selected)
    } else {
        Err(Box::new(RosWireError::profile_not_found(selected)))
    }
}

pub fn ensure_secure_directory_permissions(path: &Path) -> RosWireResult<()> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(path).map_err(|err| {
            Box::new(RosWireError::config(format!(
                "failed to inspect directory permissions: {err}",
            )))
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(Box::new(RosWireError::config_insecure_permissions(
                format!("directory permissions are too wide: {:o}", mode,),
            )));
        }
    }
    Ok(())
}

pub fn ensure_secure_file_permissions(path: &Path) -> RosWireResult<()> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(path).map_err(|err| {
            Box::new(RosWireError::config(format!(
                "failed to inspect file permissions: {err}",
            )))
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(Box::new(RosWireError::config_insecure_permissions(
                format!("config file permissions are too wide: {:o}", mode,),
            )));
        }
    }
    Ok(())
}

pub fn inspect_config(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    config: &ConfigFile,
    paths: &ConfigPaths,
) -> RosWireResult<ConfigInspect> {
    let active_profile = select_active_profile(
        cli.profile.as_deref(),
        env.get("ROS_PROFILE").map(String::as_str),
        config,
    )?;
    let profile = config
        .profiles
        .get(&active_profile)
        .ok_or_else(|| Box::new(RosWireError::profile_not_found(active_profile.clone())))?;

    let mut resolved = BTreeMap::new();
    insert_resolved_field(
        &mut resolved,
        "host",
        cli.host.as_deref(),
        env.get("ROS_HOST").map(String::as_str),
        profile.host.as_deref(),
        None,
    );
    insert_resolved_field(
        &mut resolved,
        "user",
        cli.user.as_deref(),
        env.get("ROS_USER").map(String::as_str),
        profile.user.as_deref(),
        None,
    );
    insert_resolved_field(
        &mut resolved,
        "protocol",
        cli.protocol.map(|value| value.as_str()),
        env.get("ROS_PROTOCOL").map(String::as_str),
        profile.protocol.as_deref(),
        Some("auto"),
    );
    insert_resolved_field(
        &mut resolved,
        "routeros_version",
        cli.routeros_version.map(|value| value.as_str()),
        env.get("ROS_ROUTEROS_VERSION").map(String::as_str),
        profile.routeros_version.as_deref(),
        Some("auto"),
    );
    insert_resolved_field(
        &mut resolved,
        "transfer",
        cli.transfer.map(|value| value.as_str()),
        env.get("ROS_TRANSFER").map(String::as_str),
        profile.transfer.as_deref(),
        Some("ssh"),
    );

    let port_cli = cli.port.map(|value| value.to_string());
    let port_profile = profile.port.map(|value| value.to_string());
    insert_resolved_field(
        &mut resolved,
        "port",
        port_cli.as_deref(),
        env.get("ROS_PORT").map(String::as_str),
        port_profile.as_deref(),
        None,
    );

    Ok(ConfigInspect {
        schema_version: "roswire.config.inspect.v1".to_owned(),
        active_profile,
        paths: ConfigInspectPaths {
            home: paths.home.display().to_string(),
            config: paths.config.display().to_string(),
            logs: paths.logs.display().to_string(),
        },
        resolved,
        warnings: Vec::new(),
    })
}

fn insert_resolved_field(
    resolved: &mut BTreeMap<String, ResolvedField>,
    name: &str,
    cli_value: Option<&str>,
    env_value: Option<&str>,
    profile_value: Option<&str>,
    default_value: Option<&str>,
) {
    let candidate = cli_value
        .map(|value| (value.to_owned(), ValueSource::Cli))
        .or_else(|| env_value.map(|value| (value.to_owned(), ValueSource::Env)))
        .or_else(|| profile_value.map(|value| (value.to_owned(), ValueSource::Profile)))
        .or_else(|| default_value.map(|value| (value.to_owned(), ValueSource::Default)));

    if let Some((value, source)) = candidate {
        resolved.insert(name.to_owned(), ResolvedField { value, source });
    }
}

pub fn has_error_code(error: &RosWireError, expected: ErrorCode) -> bool {
    error.error_code == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::ProtocolMode;
    use clap::Parser;

    #[test]
    fn roswire_home_respects_env_override() {
        let path = resolve_home_path(Some("/tmp/roswire-home"));
        assert_eq!(path, PathBuf::from("/tmp/roswire-home"));
    }

    #[test]
    fn select_profile_uses_cli_then_env_then_default() {
        let config = ConfigFile {
            version: 1,
            default_profile: Some("home".to_owned()),
            profiles: BTreeMap::from([
                ("home".to_owned(), ProfileConfig::default()),
                ("office".to_owned(), ProfileConfig::default()),
            ]),
        };

        let selected = select_active_profile(Some("office"), Some("home"), &config)
            .expect("cli profile should win");
        assert_eq!(selected, "office");

        let selected =
            select_active_profile(None, Some("home"), &config).expect("env profile should win");
        assert_eq!(selected, "home");

        let selected =
            select_active_profile(None, None, &config).expect("default profile should apply");
        assert_eq!(selected, "home");
    }

    #[test]
    fn profile_not_found_returns_structured_error() {
        let config = ConfigFile {
            version: 1,
            default_profile: Some("home".to_owned()),
            profiles: BTreeMap::from([("home".to_owned(), ProfileConfig::default())]),
        };

        let error = select_active_profile(Some("missing"), None, &config)
            .expect_err("missing profile should fail");
        assert!(has_error_code(&error, ErrorCode::ProfileNotFound));
    }

    #[test]
    fn inspect_config_resolves_precedence() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--host",
            "3.3.3.3",
            "--protocol",
            "rest",
            "ip",
            "address",
            "print",
        ])
        .expect("cli should parse");

        let config = ConfigFile {
            version: 1,
            default_profile: Some("home".to_owned()),
            profiles: BTreeMap::from([(
                "home".to_owned(),
                ProfileConfig {
                    host: Some("1.1.1.1".to_owned()),
                    user: Some("profile-user".to_owned()),
                    protocol: Some("api".to_owned()),
                    routeros_version: Some("v7".to_owned()),
                    transfer: Some("ssh".to_owned()),
                    port: Some(8728),
                },
            )]),
        };

        let env = BTreeMap::from([
            ("ROS_HOST".to_owned(), "2.2.2.2".to_owned()),
            ("ROS_USER".to_owned(), "env-user".to_owned()),
        ]);
        let paths = ConfigPaths::from_home(PathBuf::from("/tmp/roswire"));

        let inspect = inspect_config(&cli, &env, &config, &paths).expect("inspect should work");

        assert_eq!(inspect.active_profile, "home");
        assert_eq!(inspect.schema_version, "roswire.config.inspect.v1");
        assert_eq!(
            inspect.resolved.get("host"),
            Some(&ResolvedField {
                value: "3.3.3.3".to_owned(),
                source: ValueSource::Cli,
            })
        );
        assert_eq!(
            inspect.resolved.get("user"),
            Some(&ResolvedField {
                value: "env-user".to_owned(),
                source: ValueSource::Env,
            })
        );
        assert_eq!(
            inspect.resolved.get("protocol"),
            Some(&ResolvedField {
                value: ProtocolMode::Rest.as_str().to_owned(),
                source: ValueSource::Cli,
            })
        );
        assert_eq!(
            inspect.resolved.get("routeros_version"),
            Some(&ResolvedField {
                value: "v7".to_owned(),
                source: ValueSource::Profile,
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn insecure_file_permissions_are_rejected() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let file = temp.path().join("config.toml");
        fs::write(&file, "version = 1").expect("config file should be written");

        let mut permissions = fs::metadata(&file)
            .expect("metadata should exist")
            .permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&file, permissions).expect("permissions should be applied");

        let error =
            ensure_secure_file_permissions(&file).expect_err("insecure permissions should fail");
        assert!(has_error_code(&error, ErrorCode::ConfigInsecurePermissions));
    }
}
