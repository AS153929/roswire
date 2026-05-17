use crate::args::Cli;
use crate::error::{ErrorCode, ErrorContext, RosWireError, RosWireResult};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use directories::BaseDirs;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn default_config_version() -> u32 {
    1
}

const DEFAULT_MASTER_KEY_ENV: &str = "ROSWIRE_MASTER_KEY";

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ConfigFile {
    #[serde(default = "default_config_version")]
    pub version: u32,
    #[serde(default)]
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProfileConfig {
    pub host: Option<String>,
    pub user: Option<String>,
    pub protocol: Option<String>,
    pub routeros_version: Option<String>,
    pub transfer: Option<String>,
    pub port: Option<u16>,
    pub ssh_port: Option<u16>,
    pub ssh_user: Option<String>,
    pub ssh_key: Option<String>,
    #[serde(default)]
    pub allow_plain_secrets: bool,
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SecretSpec {
    Plain {
        value: String,
    },
    Encrypted {
        key_id: Option<String>,
        value: String,
    },
    Keychain {
        service: String,
        account: String,
    },
    Env {
        var: String,
    },
    SameAs {
        target: String,
    },
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
    pub secrets: BTreeMap<String, SecretInspectField>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SecretInspectField {
    pub status: String,
    #[serde(rename = "type")]
    pub secret_type: String,
    pub source: ValueSource,
    pub redacted: bool,
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

pub fn validate_remote_host(host: &str) -> RosWireResult<()> {
    if is_mac_address(host) {
        return Err(Box::new(
            RosWireError::config(format!(
                "RouterOS host must be an IP address or DNS name, not a MAC address: {host}"
            ))
            .with_hint(
                "set host/ROS_HOST/--host to a routable IP address or DNS name; MAC-based Layer 2 discovery is not supported by this CLI",
            )
            .with_context(ErrorContext {
                host: host.to_owned(),
                ..ErrorContext::default()
            }),
        ));
    }

    Ok(())
}

pub fn is_mac_address(value: &str) -> bool {
    let value = value.trim();
    is_separated_mac_address(value, ':') || is_separated_mac_address(value, '-')
}

fn is_separated_mac_address(value: &str, separator: char) -> bool {
    let parts = value.split(separator).collect::<Vec<_>>();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|ch| ch.is_ascii_hexdigit()))
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

pub fn resolve_profile_secrets(
    profile: &ProfileConfig,
) -> RosWireResult<BTreeMap<String, SecretInspectField>> {
    let mut resolved = BTreeMap::new();
    let mut visiting = Vec::new();

    for name in profile.secrets.keys() {
        let field = resolve_secret_recursive(name, profile, &mut resolved, &mut visiting)?;
        resolved.insert(name.clone(), field);
    }

    Ok(resolved)
}

fn resolve_secret_recursive(
    name: &str,
    profile: &ProfileConfig,
    resolved: &mut BTreeMap<String, SecretInspectField>,
    visiting: &mut Vec<String>,
) -> RosWireResult<SecretInspectField> {
    if let Some(field) = resolved.get(name) {
        return Ok(field.clone());
    }

    if visiting.iter().any(|item| item == name) {
        return Err(Box::new(RosWireError::config(
            "secret same-as cycle detected",
        )));
    }

    let spec = profile.secrets.get(name).ok_or_else(|| {
        Box::new(RosWireError::config(format!(
            "secret target missing: {name}",
        )))
    })?;

    visiting.push(name.to_owned());
    let field = match spec {
        SecretSpec::Plain { .. } => {
            if !profile.allow_plain_secrets {
                return Err(Box::new(RosWireError::config(
                    "plain secrets require allow_plain_secrets = true",
                )));
            }

            SecretInspectField {
                status: "available".to_owned(),
                secret_type: "plain".to_owned(),
                source: ValueSource::Profile,
                redacted: true,
            }
        }
        SecretSpec::Encrypted { .. } => SecretInspectField {
            status: "available".to_owned(),
            secret_type: "encrypted".to_owned(),
            source: ValueSource::Profile,
            redacted: true,
        },
        SecretSpec::Keychain { .. } => SecretInspectField {
            status: "available".to_owned(),
            secret_type: "keychain".to_owned(),
            source: ValueSource::Profile,
            redacted: true,
        },
        SecretSpec::Env { .. } => SecretInspectField {
            status: "available".to_owned(),
            secret_type: "env".to_owned(),
            source: ValueSource::Env,
            redacted: true,
        },
        SecretSpec::SameAs { target } => {
            resolve_secret_recursive(target, profile, resolved, visiting)?
        }
    };
    visiting.pop();

    resolved.insert(name.to_owned(), field.clone());
    Ok(field)
}

pub fn resolve_profile_secret_value(
    profile: &ProfileConfig,
    name: &str,
    env: &BTreeMap<String, String>,
) -> RosWireResult<Option<String>> {
    resolve_profile_secret_value_recursive(profile, name, env, &mut Vec::new())
}

fn resolve_profile_secret_value_recursive(
    profile: &ProfileConfig,
    name: &str,
    env: &BTreeMap<String, String>,
    visiting: &mut Vec<String>,
) -> RosWireResult<Option<String>> {
    let Some(spec) = profile.secrets.get(name) else {
        return Ok(None);
    };

    if visiting.iter().any(|item| item == name) {
        return Err(Box::new(RosWireError::config(
            "secret same-as cycle detected",
        )));
    }

    visiting.push(name.to_owned());
    let value = match spec {
        SecretSpec::Plain { value } => {
            if !profile.allow_plain_secrets {
                return Err(Box::new(RosWireError::config(
                    "plain secrets require allow_plain_secrets = true",
                )));
            }
            Some(value.clone())
        }
        SecretSpec::Encrypted { key_id, value } => {
            let master_key = encrypted_master_key(env, key_id.as_deref())?;
            Some(decrypt_secret_value(value, &master_key)?)
        }
        SecretSpec::Keychain { service, account } => Some(read_keychain_secret(service, account)?),
        SecretSpec::Env { var } => Some(read_env_secret(env, var)?),
        SecretSpec::SameAs { target } => {
            resolve_profile_secret_value_recursive(profile, target, env, visiting)?
        }
    };
    visiting.pop();

    Ok(value)
}

fn read_env_secret(env: &BTreeMap<String, String>, var: &str) -> RosWireResult<String> {
    env.get(var)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| {
            Box::new(RosWireError::secret_not_found(format!(
                "environment secret is not set: {var}",
            )))
        })
}

fn read_keychain_secret(service: &str, account: &str) -> RosWireResult<String> {
    let entry = keyring::Entry::new(service, account).map_err(map_keychain_backend_error)?;
    entry.get_password().map_err(map_keychain_read_error)
}

fn write_keychain_secret(service: &str, account: &str, value: &str) -> RosWireResult<()> {
    let entry = keyring::Entry::new(service, account).map_err(map_keychain_backend_error)?;
    entry
        .set_password(value)
        .map_err(map_keychain_backend_error)
}

fn map_keychain_read_error(error: keyring::Error) -> Box<RosWireError> {
    match error {
        keyring::Error::NoEntry => Box::new(RosWireError::secret_not_found(
            "keychain secret was not found",
        )),
        other => map_keychain_backend_error(other),
    }
}

fn map_keychain_backend_error(error: keyring::Error) -> Box<RosWireError> {
    Box::new(RosWireError::secret_backend_unavailable(format!(
        "keychain backend unavailable: {error}",
    )))
}

fn encrypted_master_key(
    env: &BTreeMap<String, String>,
    key_id: Option<&str>,
) -> RosWireResult<String> {
    let key_env = key_id.unwrap_or(DEFAULT_MASTER_KEY_ENV);
    env.get(key_env)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| {
            Box::new(RosWireError::secret_backend_unavailable(format!(
                "encrypted secret master key env var is unavailable: {key_env}",
            )))
        })
}

fn encrypt_secret_value(value: &str, master_key: &str) -> RosWireResult<String> {
    let key = derive_encryption_key(master_key);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to initialize secret cipher: {error}",
        )))
    })?;
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|_| {
            Box::new(RosWireError::secret_decrypt_failed(
                "failed to encrypt secret",
            ))
        })?;

    Ok(format!(
        "v1:{}:{}",
        BASE64_STANDARD.encode(nonce),
        BASE64_STANDARD.encode(ciphertext)
    ))
}

fn decrypt_secret_value(value: &str, master_key: &str) -> RosWireResult<String> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "v1" {
        return Err(Box::new(RosWireError::secret_decrypt_failed(
            "encrypted secret payload has an unsupported format",
        )));
    }

    let nonce = BASE64_STANDARD.decode(parts[1]).map_err(|_| {
        Box::new(RosWireError::secret_decrypt_failed(
            "encrypted secret nonce is invalid",
        ))
    })?;
    let ciphertext = BASE64_STANDARD.decode(parts[2]).map_err(|_| {
        Box::new(RosWireError::secret_decrypt_failed(
            "encrypted secret ciphertext is invalid",
        ))
    })?;
    if nonce.len() != 12 {
        return Err(Box::new(RosWireError::secret_decrypt_failed(
            "encrypted secret nonce has invalid length",
        )));
    }

    let key = derive_encryption_key(master_key);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to initialize secret cipher: {error}",
        )))
    })?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| RosWireError::secret_decrypt_failed("failed to decrypt secret"))?;

    String::from_utf8(plaintext).map_err(|_| {
        Box::new(RosWireError::secret_decrypt_failed(
            "decrypted secret is not valid UTF-8",
        ))
    })
}

fn derive_encryption_key(master_key: &str) -> [u8; 32] {
    Sha256::digest(master_key.as_bytes()).into()
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
    let secrets = resolve_profile_secrets(profile)?;

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

    let ssh_port_cli = cli.ssh_port.map(|value| value.to_string());
    let ssh_port_profile = profile.ssh_port.map(|value| value.to_string());
    insert_resolved_field(
        &mut resolved,
        "ssh_port",
        ssh_port_cli.as_deref(),
        env.get("ROS_SSH_PORT").map(String::as_str),
        ssh_port_profile.as_deref(),
        Some("22"),
    );
    insert_resolved_field(
        &mut resolved,
        "ssh_user",
        cli.ssh_user.as_deref(),
        env.get("ROS_SSH_USER").map(String::as_str),
        profile.ssh_user.as_deref(),
        None,
    );
    let ssh_key_cli = cli.ssh_key.as_deref().map(redact_local_path_for_inspect);
    let ssh_key_env = env
        .get("ROS_SSH_KEY")
        .map(String::as_str)
        .map(redact_local_path_for_inspect);
    let ssh_key_profile = profile
        .ssh_key
        .as_deref()
        .map(redact_local_path_for_inspect);
    insert_resolved_field(
        &mut resolved,
        "ssh_key",
        ssh_key_cli.as_deref(),
        ssh_key_env.as_deref(),
        ssh_key_profile.as_deref(),
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
        secrets,
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

#[derive(Debug, Serialize)]
struct ConfigInitPayload {
    schema_version: &'static str,
    operation: &'static str,
    created_home: bool,
    created_config: bool,
    paths: ConfigInspectPaths,
}

#[derive(Debug, Serialize)]
struct ConfigProfilesPayload {
    schema_version: &'static str,
    default_profile: Option<String>,
    profiles: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ConfigDevicePayload {
    schema_version: &'static str,
    operation: &'static str,
    profile: String,
    updated_fields: Vec<String>,
    default_profile: Option<String>,
}

#[derive(Debug, Serialize)]
struct ConfigSecretPayload {
    schema_version: &'static str,
    operation: &'static str,
    profile: String,
    secret_name: String,
    #[serde(rename = "type")]
    secret_type: String,
    redacted: bool,
    allow_plain_secrets: bool,
}

pub fn handle(tokens: &[String], cli: &Cli) -> Option<RosWireResult<String>> {
    if tokens.is_empty() {
        return None;
    }

    match tokens[0].as_str() {
        "config" => Some(handle_config_tokens(tokens, cli)),
        "secret" => Some(handle_secret_alias(tokens, cli)),
        _ => None,
    }
}

fn handle_config_tokens(tokens: &[String], cli: &Cli) -> RosWireResult<String> {
    if tokens.len() < 2 {
        return Err(Box::new(RosWireError::usage(
            "config command requires a subcommand",
        )));
    }

    match tokens[1].as_str() {
        "init" => handle_config_init(),
        "inspect" => handle_config_inspect(cli),
        "profiles" => handle_config_profiles(),
        "device" => handle_config_device(tokens),
        "secret" => handle_config_secret(tokens, cli),
        _ => Err(Box::new(RosWireError::usage(format!(
            "unsupported config subcommand: {}",
            tokens[1],
        )))),
    }
}

fn handle_secret_alias(tokens: &[String], cli: &Cli) -> RosWireResult<String> {
    if tokens.get(1).map(String::as_str) != Some("set") {
        return Err(Box::new(RosWireError::usage(
            "secret command supports: secret set <profile> <name> type=<...>",
        )));
    }

    if tokens.len() < 4 {
        return Err(Box::new(RosWireError::usage(
            "secret set requires: secret set <profile> <name> type=<...>",
        )));
    }

    handle_secret_set_impl(&tokens[2], &tokens[3], &tokens[4..], cli.stdin)
}

fn handle_config_init() -> RosWireResult<String> {
    let env = read_env_map();
    let paths = runtime_paths_from_env(&env);
    let (created_home, created_config) = ensure_home_layout(&paths)?;

    render_json(&ConfigInitPayload {
        schema_version: "roswire.config.init.v1",
        operation: "config.init",
        created_home,
        created_config,
        paths: ConfigInspectPaths {
            home: paths.home.display().to_string(),
            config: paths.config.display().to_string(),
            logs: paths.logs.display().to_string(),
        },
    })
}

fn handle_config_inspect(cli: &Cli) -> RosWireResult<String> {
    let env = read_env_map();
    let paths = runtime_paths_from_env(&env);

    if !paths.config.exists() {
        return Err(Box::new(RosWireError::config(
            "config.toml not found; run `roswire config init --json` first",
        )));
    }

    ensure_secure_directory_permissions(&paths.home)?;
    ensure_secure_file_permissions(&paths.config)?;

    let config = load_config_file(&paths.config)?;
    let inspect = inspect_config(cli, &env, &config, &paths)?;
    render_json(&inspect)
}

fn handle_config_profiles() -> RosWireResult<String> {
    let env = read_env_map();
    let paths = runtime_paths_from_env(&env);

    if !paths.config.exists() {
        return Err(Box::new(RosWireError::config(
            "config.toml not found; run `roswire config init --json` first",
        )));
    }

    let config = load_config_file(&paths.config)?;
    let profiles = config.profiles.keys().cloned().collect::<Vec<_>>();

    render_json(&ConfigProfilesPayload {
        schema_version: "roswire.config.profiles.v1",
        default_profile: config.default_profile,
        profiles,
    })
}

fn handle_config_device(tokens: &[String]) -> RosWireResult<String> {
    if tokens.len() < 4 {
        return Err(Box::new(RosWireError::usage(
            "config device requires: config device <add|set> <profile> [key=value ...]",
        )));
    }

    let operation = tokens[2].as_str();
    let profile_name = tokens[3].clone();
    let key_values = parse_key_value_tokens(&tokens[4..])?;

    if operation != "add" && operation != "set" {
        return Err(Box::new(RosWireError::usage(
            "config device supports add|set",
        )));
    }

    let env = read_env_map();
    let paths = runtime_paths_from_env(&env);
    let _ = ensure_home_layout(&paths)?;

    let mut config = load_or_default_config(&paths.config)?;
    let profile_exists = config.profiles.contains_key(&profile_name);

    if operation == "add" && profile_exists {
        return Err(Box::new(RosWireError::config(format!(
            "profile already exists: {profile_name}",
        ))));
    }

    let profile = config
        .profiles
        .entry(profile_name.clone())
        .or_insert_with(ProfileConfig::default);

    let mut updated_fields = Vec::new();
    for (key, value) in key_values {
        match key.as_str() {
            "host" => {
                validate_remote_host(&value)?;
                profile.host = Some(value);
                updated_fields.push("host".to_owned());
            }
            "user" => {
                profile.user = Some(value);
                updated_fields.push("user".to_owned());
            }
            "protocol" => {
                profile.protocol = Some(normalize_protocol(&value)?);
                updated_fields.push("protocol".to_owned());
            }
            "routeros_version" | "routeros-version" => {
                profile.routeros_version = Some(normalize_routeros_version(&value)?);
                updated_fields.push("routeros_version".to_owned());
            }
            "transfer" => {
                profile.transfer = Some(normalize_transfer(&value)?);
                updated_fields.push("transfer".to_owned());
            }
            "port" => {
                profile.port = Some(parse_port(&value)?);
                updated_fields.push("port".to_owned());
            }
            "ssh_port" | "ssh-port" => {
                profile.ssh_port = Some(parse_port(&value)?);
                updated_fields.push("ssh_port".to_owned());
            }
            "ssh_user" | "ssh-user" => {
                profile.ssh_user = Some(value);
                updated_fields.push("ssh_user".to_owned());
            }
            "ssh_key" | "ssh-key" => {
                profile.ssh_key = Some(value);
                updated_fields.push("ssh_key".to_owned());
            }
            _ => {
                return Err(Box::new(RosWireError::usage(format!(
                    "unsupported device field: {key}",
                ))));
            }
        }
    }

    if operation == "add" && (profile.host.is_none() || profile.user.is_none()) {
        return Err(Box::new(RosWireError::usage(
            "config device add requires host=<...> and user=<...>",
        )));
    }

    if config.default_profile.is_none() {
        config.default_profile = Some(profile_name.clone());
    }

    save_config_file(&paths.config, &config)?;

    render_json(&ConfigDevicePayload {
        schema_version: "roswire.config.device.v1",
        operation: if operation == "add" {
            "config.device.add"
        } else {
            "config.device.set"
        },
        profile: profile_name,
        updated_fields,
        default_profile: config.default_profile,
    })
}

fn handle_config_secret(tokens: &[String], cli: &Cli) -> RosWireResult<String> {
    if tokens.get(2).map(String::as_str) != Some("set") {
        return Err(Box::new(RosWireError::usage(
            "config secret supports: config secret set <profile> <name> type=<...>",
        )));
    }

    if tokens.len() < 5 {
        return Err(Box::new(RosWireError::usage(
            "config secret set requires: config secret set <profile> <name> type=<...>",
        )));
    }

    handle_secret_set_impl(&tokens[3], &tokens[4], &tokens[5..], cli.stdin)
}

fn handle_secret_set_impl(
    profile_name: &str,
    secret_name: &str,
    key_value_tokens: &[String],
    read_stdin: bool,
) -> RosWireResult<String> {
    let mut key_values = parse_key_value_tokens(key_value_tokens)?;
    let secret_type = key_values
        .remove("type")
        .ok_or_else(|| Box::new(RosWireError::usage("secret set requires type=<...>")))?;

    let input_value = match secret_type.as_str() {
        "plain" | "encrypted" | "keychain" => take_secret_input(&mut key_values, read_stdin, true)?,
        _ => take_secret_input(&mut key_values, false, false)?,
    };

    let env = read_env_map();
    let paths = runtime_paths_from_env(&env);
    let _ = ensure_home_layout(&paths)?;

    let mut config = load_or_default_config(&paths.config)?;

    if !config.profiles.contains_key(profile_name) {
        return Err(Box::new(RosWireError::profile_not_found(profile_name)));
    }

    let (secret_spec, normalized_type, toggled_plain) = match secret_type.as_str() {
        "plain" => {
            let value = input_value.ok_or_else(|| {
                Box::new(RosWireError::usage(
                    "plain secret requires value=<...>, env=<VAR>, or --stdin",
                ))
            })?;
            (SecretSpec::Plain { value }, "plain".to_owned(), true)
        }
        "encrypted" => {
            let value = input_value.ok_or_else(|| {
                Box::new(RosWireError::usage(
                    "encrypted secret requires value=<...>, env=<VAR>, or --stdin",
                ))
            })?;
            let key_id = key_values.remove("key_id");
            let master_key = encrypted_master_key(&env, key_id.as_deref())?;
            let value = encrypt_secret_value(&value, &master_key)?;
            (
                SecretSpec::Encrypted { key_id, value },
                "encrypted".to_owned(),
                false,
            )
        }
        "keychain" => {
            let service = key_values.remove("service").ok_or_else(|| {
                Box::new(RosWireError::usage(
                    "keychain secret requires service=<...>",
                ))
            })?;
            let account = key_values.remove("account").ok_or_else(|| {
                Box::new(RosWireError::usage(
                    "keychain secret requires account=<...>",
                ))
            })?;
            if let Some(value) = input_value.as_deref() {
                write_keychain_secret(&service, &account, value)?;
            }
            (
                SecretSpec::Keychain { service, account },
                "keychain".to_owned(),
                false,
            )
        }
        "env" => {
            let var = key_values
                .remove("env")
                .ok_or_else(|| Box::new(RosWireError::usage("env secret requires env=<VAR>")))?;
            if input_value.is_some() || read_stdin {
                return Err(Box::new(RosWireError::usage(
                    "env secret stores an environment variable name; do not pass value=<...> or --stdin",
                )));
            }
            (SecretSpec::Env { var }, "env".to_owned(), false)
        }
        "same-as" => {
            let target = key_values.remove("target").ok_or_else(|| {
                Box::new(RosWireError::usage("same-as secret requires target=<...>"))
            })?;
            if input_value.is_some() || read_stdin {
                return Err(Box::new(RosWireError::usage(
                    "same-as secret does not accept value=<...>, env=<VAR>, or --stdin",
                )));
            }
            (SecretSpec::SameAs { target }, "same-as".to_owned(), false)
        }
        _ => {
            return Err(Box::new(RosWireError::usage(format!(
                "unsupported secret type: {secret_type}",
            ))));
        }
    };

    if let Some(extra) = key_values.keys().next() {
        return Err(Box::new(RosWireError::usage(format!(
            "unexpected secret option: {extra}",
        ))));
    }

    let allow_plain_secrets = {
        let profile = config
            .profiles
            .get_mut(profile_name)
            .ok_or_else(|| Box::new(RosWireError::profile_not_found(profile_name)))?;

        if toggled_plain {
            profile.allow_plain_secrets = true;
        }

        profile.secrets.insert(secret_name.to_owned(), secret_spec);
        profile.allow_plain_secrets
    };

    save_config_file(&paths.config, &config)?;

    render_json(&ConfigSecretPayload {
        schema_version: "roswire.config.secret.v1",
        operation: "config.secret.set",
        profile: profile_name.to_owned(),
        secret_name: secret_name.to_owned(),
        secret_type: normalized_type,
        redacted: true,
        allow_plain_secrets,
    })
}

fn runtime_paths_from_env(env: &BTreeMap<String, String>) -> ConfigPaths {
    ConfigPaths::from_home(resolve_home_path(
        env.get("ROSWIRE_HOME").map(String::as_str),
    ))
}

fn read_env_map() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

fn parse_key_value_tokens(tokens: &[String]) -> RosWireResult<BTreeMap<String, String>> {
    let mut key_values = BTreeMap::new();
    for token in tokens {
        let (key, value) = token.split_once('=').ok_or_else(|| {
            Box::new(RosWireError::usage(format!(
                "expected key=value token, got: {token}",
            )))
        })?;
        if key.is_empty() {
            return Err(Box::new(RosWireError::usage(
                "key=value token cannot have empty key",
            )));
        }

        key_values.insert(key.to_owned(), value.to_owned());
    }

    Ok(key_values)
}

fn take_secret_input(
    key_values: &mut BTreeMap<String, String>,
    read_stdin: bool,
    allow_env_source: bool,
) -> RosWireResult<Option<String>> {
    let value = key_values.remove("value");
    let env_var = if allow_env_source {
        key_values.remove("env")
    } else {
        None
    };

    let source_count =
        usize::from(value.is_some()) + usize::from(env_var.is_some()) + read_stdin as usize;
    if source_count > 1 {
        return Err(Box::new(RosWireError::usage(
            "secret value source must be exactly one of value=<...>, env=<VAR>, or --stdin",
        )));
    }

    if let Some(value) = value {
        return Ok(Some(value));
    }

    if let Some(var) = env_var {
        return std::env::var(&var).map(Some).map_err(|_| {
            Box::new(RosWireError::secret_not_found(format!(
                "secret source environment variable is not set: {var}",
            )))
        });
    }

    if read_stdin {
        return read_secret_from_stdin().map(Some);
    }

    Ok(None)
}

fn read_secret_from_stdin() -> RosWireResult<String> {
    let mut value = String::new();
    io::stdin().read_to_string(&mut value).map_err(|error| {
        Box::new(RosWireError::config(format!(
            "failed to read secret from stdin: {error}",
        )))
    })?;
    Ok(trim_secret_stdin(value))
}

fn trim_secret_stdin(mut value: String) -> String {
    while value.ends_with('\n') || value.ends_with('\r') {
        value.pop();
    }
    value
}

fn normalize_protocol(value: &str) -> RosWireResult<String> {
    match value {
        "auto" | "api" | "api-ssl" | "rest" => Ok(value.to_owned()),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid protocol value: {value}",
        )))),
    }
}

fn normalize_routeros_version(value: &str) -> RosWireResult<String> {
    match value {
        "auto" | "v6" | "v7" => Ok(value.to_owned()),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid routeros_version value: {value}",
        )))),
    }
}

fn normalize_transfer(value: &str) -> RosWireResult<String> {
    match value {
        "ssh" => Ok(value.to_owned()),
        _ => Err(Box::new(RosWireError::usage(format!(
            "invalid transfer value: {value}",
        )))),
    }
}

fn parse_port(value: &str) -> RosWireResult<u16> {
    value.parse::<u16>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid port value `{value}`: {error}",
        )))
    })
}

fn redact_local_path_for_inspect(path: &str) -> String {
    let path_ref = Path::new(path);
    if path_ref.is_absolute() {
        let file_name = path_ref
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("ssh-key");
        format!("***REDACTED***/{file_name}")
    } else {
        path.to_owned()
    }
}

fn ensure_home_layout(paths: &ConfigPaths) -> RosWireResult<(bool, bool)> {
    let created_home = !paths.home.exists();
    let created_config = !paths.config.exists();

    fs::create_dir_all(&paths.home).map_err(|error| {
        Box::new(RosWireError::config(format!(
            "failed to create roswire home: {error}",
        )))
    })?;
    fs::create_dir_all(&paths.logs).map_err(|error| {
        Box::new(RosWireError::config(format!(
            "failed to create roswire logs directory: {error}",
        )))
    })?;

    if created_config {
        save_config_file(&paths.config, &ConfigFile::default())?;
    }

    #[cfg(unix)]
    {
        fs::set_permissions(&paths.home, fs::Permissions::from_mode(0o700)).map_err(|error| {
            Box::new(RosWireError::config(format!(
                "failed to set home permissions: {error}",
            )))
        })?;

        if paths.config.exists() {
            fs::set_permissions(&paths.config, fs::Permissions::from_mode(0o600)).map_err(
                |error| {
                    Box::new(RosWireError::config(format!(
                        "failed to set config permissions: {error}",
                    )))
                },
            )?;
        }
    }

    Ok((created_home, created_config))
}

fn load_or_default_config(path: &Path) -> RosWireResult<ConfigFile> {
    if path.exists() {
        load_config_file(path)
    } else {
        Ok(ConfigFile::default())
    }
}

fn save_config_file(path: &Path, config: &ConfigFile) -> RosWireResult<()> {
    let serialized = toml::to_string_pretty(config).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize config.toml: {error}",
        )))
    })?;

    fs::write(path, serialized).map_err(|error| {
        Box::new(RosWireError::config(format!(
            "failed to write config.toml: {error}",
        )))
    })?;

    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            Box::new(RosWireError::config(format!(
                "failed to set config permissions: {error}",
            )))
        })?;
    }

    Ok(())
}

fn render_json<T: Serialize>(value: &T) -> RosWireResult<String> {
    serde_json::to_string(value).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize config payload: {error}",
        )))
    })
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
    fn mac_host_detection_rejects_common_layer2_formats_only() {
        assert!(is_mac_address("48:8F:5A:A3:0E:A7"));
        assert!(is_mac_address("48-8f-5a-a3-0e-a7"));
        assert!(!is_mac_address("192.0.2.1"));
        assert!(!is_mac_address("router.example.test"));
        assert!(!is_mac_address("2001:db8::1"));

        validate_remote_host("router.example.test").expect("DNS host should be accepted");
        let error =
            validate_remote_host("48:8F:5A:A3:0E:A7").expect_err("MAC host should be rejected");

        assert!(has_error_code(&error, ErrorCode::ConfigError));
        assert_eq!(error.context.host, "48:8F:5A:A3:0E:A7");
        assert!(error.message.contains("MAC address"));
        assert!(error
            .hint
            .as_deref()
            .is_some_and(|hint| hint.contains("Layer 2 discovery is not supported")));
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
                    ..ProfileConfig::default()
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

    #[test]
    fn plain_secret_requires_explicit_allow_flag() {
        let profile = ProfileConfig {
            allow_plain_secrets: false,
            secrets: BTreeMap::from([(
                "password".to_owned(),
                SecretSpec::Plain {
                    value: "super-secret".to_owned(),
                },
            )]),
            ..ProfileConfig::default()
        };

        let error = resolve_profile_secrets(&profile).expect_err("plain secret should be blocked");
        assert!(has_error_code(&error, ErrorCode::ConfigError));
    }

    #[test]
    fn same_as_secret_detects_cycle() {
        let profile = ProfileConfig {
            allow_plain_secrets: true,
            secrets: BTreeMap::from([
                (
                    "password".to_owned(),
                    SecretSpec::SameAs {
                        target: "ssh_password".to_owned(),
                    },
                ),
                (
                    "ssh_password".to_owned(),
                    SecretSpec::SameAs {
                        target: "password".to_owned(),
                    },
                ),
            ]),
            ..ProfileConfig::default()
        };

        let error = resolve_profile_secrets(&profile).expect_err("cycle should fail");
        assert!(has_error_code(&error, ErrorCode::ConfigError));
    }

    #[test]
    fn same_as_secret_resolves_without_exposing_plain_value() {
        let profile = ProfileConfig {
            allow_plain_secrets: true,
            secrets: BTreeMap::from([
                (
                    "password".to_owned(),
                    SecretSpec::Plain {
                        value: "super-secret".to_owned(),
                    },
                ),
                (
                    "ssh_password".to_owned(),
                    SecretSpec::SameAs {
                        target: "password".to_owned(),
                    },
                ),
            ]),
            ..ProfileConfig::default()
        };

        let resolved = resolve_profile_secrets(&profile).expect("secrets should resolve");
        assert_eq!(
            resolved
                .get("password")
                .map(|field| field.secret_type.as_str()),
            Some("plain")
        );
        assert_eq!(
            resolved
                .get("ssh_password")
                .map(|field| field.secret_type.as_str()),
            Some("plain")
        );
        assert_eq!(
            resolved.get("password").map(|field| field.redacted),
            Some(true)
        );
    }

    #[test]
    fn env_secret_resolves_without_storing_value() {
        let secret = generated_secret();
        let profile = ProfileConfig {
            secrets: BTreeMap::from([
                (
                    "password".to_owned(),
                    SecretSpec::Env {
                        var: "ROSWIRE_TEST_PASSWORD".to_owned(),
                    },
                ),
                (
                    "ssh_password".to_owned(),
                    SecretSpec::SameAs {
                        target: "password".to_owned(),
                    },
                ),
            ]),
            ..ProfileConfig::default()
        };
        let env = BTreeMap::from([("ROSWIRE_TEST_PASSWORD".to_owned(), secret.clone())]);

        let password = resolve_profile_secret_value(&profile, "password", &env)
            .expect("env secret should resolve");
        let ssh_password = resolve_profile_secret_value(&profile, "ssh_password", &env)
            .expect("same-as env secret should resolve");

        assert_eq!(password.as_deref(), Some(secret.as_str()));
        assert_eq!(ssh_password.as_deref(), Some(secret.as_str()));
    }

    #[test]
    fn missing_env_secret_uses_structured_error() {
        let profile = ProfileConfig {
            secrets: BTreeMap::from([(
                "password".to_owned(),
                SecretSpec::Env {
                    var: "ROSWIRE_TEST_PASSWORD".to_owned(),
                },
            )]),
            ..ProfileConfig::default()
        };

        let error = resolve_profile_secret_value(&profile, "password", &BTreeMap::new())
            .expect_err("missing env secret should fail");

        assert!(has_error_code(&error, ErrorCode::SecretNotFound));
    }

    #[test]
    fn encrypted_secret_round_trips_with_env_master_key() {
        let secret = generated_secret();
        let master_key = generated_master_key();
        let encrypted = encrypt_secret_value(&secret, &master_key).expect("secret should encrypt");
        let profile = ProfileConfig {
            secrets: BTreeMap::from([(
                "password".to_owned(),
                SecretSpec::Encrypted {
                    key_id: Some("ROSWIRE_TEST_MASTER_KEY".to_owned()),
                    value: encrypted.clone(),
                },
            )]),
            ..ProfileConfig::default()
        };
        let env = BTreeMap::from([("ROSWIRE_TEST_MASTER_KEY".to_owned(), master_key)]);

        let resolved = resolve_profile_secret_value(&profile, "password", &env)
            .expect("encrypted secret should resolve");

        assert_ne!(encrypted, secret);
        assert_eq!(resolved.as_deref(), Some(secret.as_str()));
    }

    #[test]
    fn encrypted_secret_rejects_wrong_master_key() {
        let secret = generated_secret();
        let encrypted =
            encrypt_secret_value(&secret, &generated_master_key()).expect("secret should encrypt");
        let profile = ProfileConfig {
            secrets: BTreeMap::from([(
                "password".to_owned(),
                SecretSpec::Encrypted {
                    key_id: Some("ROSWIRE_TEST_MASTER_KEY".to_owned()),
                    value: encrypted,
                },
            )]),
            ..ProfileConfig::default()
        };
        let env = BTreeMap::from([(
            "ROSWIRE_TEST_MASTER_KEY".to_owned(),
            generated_other_master_key(),
        )]);

        let error = resolve_profile_secret_value(&profile, "password", &env)
            .expect_err("wrong master key should fail");

        assert!(has_error_code(&error, ErrorCode::SecretDecryptFailed));
    }

    #[test]
    fn stdin_secret_trimming_removes_line_endings_only() {
        assert_eq!(trim_secret_stdin("abc\n".to_owned()), "abc");
        assert_eq!(trim_secret_stdin("abc\r\n".to_owned()), "abc");
        assert_eq!(trim_secret_stdin(" abc ".to_owned()), " abc ");
    }

    #[test]
    fn inspect_output_never_contains_secret_values() {
        let cli = Cli::try_parse_from(["roswire", "interface", "print"]).expect("cli should parse");
        let config = ConfigFile {
            version: 1,
            default_profile: Some("home".to_owned()),
            profiles: BTreeMap::from([(
                "home".to_owned(),
                ProfileConfig {
                    allow_plain_secrets: true,
                    secrets: BTreeMap::from([(
                        "password".to_owned(),
                        SecretSpec::Plain {
                            value: "super-secret".to_owned(),
                        },
                    )]),
                    ..ProfileConfig::default()
                },
            )]),
        };

        let secret = generated_secret();
        let mut config = config;
        if let Some(profile) = config.profiles.get_mut("home") {
            profile.secrets.insert(
                "env_password".to_owned(),
                SecretSpec::Env {
                    var: "ROSWIRE_TEST_PASSWORD".to_owned(),
                },
            );
        }

        let inspect = inspect_config(
            &cli,
            &BTreeMap::from([("ROSWIRE_TEST_PASSWORD".to_owned(), secret.clone())]),
            &config,
            &ConfigPaths::from_home(PathBuf::from("/tmp/roswire")),
        )
        .expect("inspect should succeed");

        let payload = serde_json::to_string(&inspect).expect("inspect payload should serialize");
        assert!(!payload.contains("super-secret"));
        assert!(!payload.contains(&secret));
        assert_eq!(
            inspect.secrets.get("password").map(|field| field.redacted),
            Some(true)
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

    fn generated_secret() -> String {
        [
            'r', 'o', 's', 'w', 'i', 'r', 'e', '-', 's', 'e', 'c', 'r', 'e', 't',
        ]
        .into_iter()
        .collect()
    }

    fn generated_master_key() -> String {
        [
            'r', 'o', 's', 'w', 'i', 'r', 'e', '-', 'm', 'a', 's', 't', 'e', 'r', '-', 'k', 'e',
            'y',
        ]
        .into_iter()
        .collect()
    }

    fn generated_other_master_key() -> String {
        [
            'o', 't', 'h', 'e', 'r', '-', 'm', 'a', 's', 't', 'e', 'r', '-', 'k', 'e', 'y',
        ]
        .into_iter()
        .collect()
    }
}
