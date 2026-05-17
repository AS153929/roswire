use crate::args::Cli;
use crate::config::{self, ConfigPaths, LoggingConfig};
use crate::error::{self, RosWireError};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use time::{Date, Duration as TimeDuration, Month, OffsetDateTime};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const LOG_SCHEMA_VERSION: &str = "roswire.log.v1";
const DEBUG_SCHEMA_VERSION: &str = "roswire.debug.v1";
const MAX_RETENTION_DAYS: u16 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLogger {
    enabled: bool,
    debug: bool,
    level: String,
    command: String,
    log_path: Option<PathBuf>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LogEvent {
    schema_version: &'static str,
    level: String,
    event: &'static str,
    command: String,
    status: &'static str,
    debug: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DebugEvent<'a> {
    schema_version: &'static str,
    event: &'static str,
    command: &'a str,
    logging: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_path: Option<String>,
    warnings: &'a [String],
}

impl RuntimeLogger {
    pub fn initialize(cli: &Cli, env: &BTreeMap<String, String>) -> Self {
        initialize_for_date(cli, env, OffsetDateTime::now_utc().date())
    }

    pub fn debug_payload(&self) -> Option<String> {
        if !self.debug {
            return None;
        }

        let payload = DebugEvent {
            schema_version: DEBUG_SCHEMA_VERSION,
            event: "debug.enabled",
            command: &self.command,
            logging: if self.enabled { "enabled" } else { "disabled" },
            log_path: self
                .log_path
                .as_ref()
                .map(|path| redact_local_path(&path.display().to_string())),
            warnings: &self.warnings,
        };
        serde_json::to_string(&payload).ok()
    }

    pub fn log_start(&mut self) {
        self.write_event(LogEvent {
            schema_version: LOG_SCHEMA_VERSION,
            level: if self.debug {
                "debug".to_owned()
            } else {
                self.level.clone()
            },
            event: "command.start",
            command: self.command.clone(),
            status: "running",
            debug: self.debug,
            error_code: None,
            message: None,
            context: None,
            warnings: self.warnings.clone(),
        });
    }

    pub fn log_success(&mut self) {
        self.write_event(LogEvent {
            schema_version: LOG_SCHEMA_VERSION,
            level: self.level.clone(),
            event: "command.end",
            command: self.command.clone(),
            status: "ok",
            debug: self.debug,
            error_code: None,
            message: None,
            context: None,
            warnings: Vec::new(),
        });
    }

    pub fn log_error(&mut self, error: &RosWireError) {
        self.write_event(LogEvent {
            schema_version: LOG_SCHEMA_VERSION,
            level: "error".to_owned(),
            event: "command.error",
            command: self.command.clone(),
            status: "error",
            debug: self.debug,
            error_code: Some(format!("{:?}", error.error_code)),
            message: Some(sanitize_text(&error.message)),
            context: Some(sanitize_json_value(
                serde_json::to_value(&error.context).unwrap_or(Value::Null),
            )),
            warnings: Vec::new(),
        });
    }

    fn write_event(&mut self, event: LogEvent) {
        if !self.enabled {
            return;
        }

        let Some(path) = &self.log_path else {
            return;
        };
        let line = match serde_json::to_string(&event) {
            Ok(line) => line,
            Err(error) => {
                self.warnings
                    .push(format!("failed to serialize log event: {error}"));
                return;
            }
        };

        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| {
                file.write_all(line.as_bytes())?;
                file.write_all(b"\n")
            });
        if let Err(error) = result {
            self.warnings
                .push(format!("failed to write log event: {error}"));
            self.enabled = false;
            return;
        }

        #[cfg(unix)]
        {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
    }
}

fn initialize_for_date(cli: &Cli, env: &BTreeMap<String, String>, today: Date) -> RuntimeLogger {
    let paths = ConfigPaths::from_home(config::resolve_home_path(
        env.get("ROSWIRE_HOME").map(String::as_str),
    ));
    let mut warnings = Vec::new();
    let command = command_name(&cli.tokens);
    let debug = cli.debug || env_debug_enabled(env);
    if !paths.home.exists() {
        warnings.push("roswire home is missing; logging is disabled for this run".to_owned());
        return RuntimeLogger {
            enabled: false,
            debug,
            level: "info".to_owned(),
            command,
            log_path: None,
            warnings,
        };
    }

    let logging = load_logging_config(&paths, &mut warnings);
    let enabled = logging.enabled;
    let retention_days = normalize_retention_days(logging.retention_days);
    let level = normalize_level(&logging.level);

    if !enabled {
        return RuntimeLogger {
            enabled: false,
            debug,
            level,
            command,
            log_path: None,
            warnings,
        };
    }

    let mut active = true;
    if let Err(error) = fs::create_dir_all(&paths.logs) {
        warnings.push(format!("failed to create logs directory: {error}"));
        active = false;
    }
    #[cfg(unix)]
    if active {
        if let Err(error) = fs::set_permissions(&paths.logs, fs::Permissions::from_mode(0o700)) {
            warnings.push(format!("failed to set logs permissions: {error}"));
        }
    }

    if active {
        warnings.extend(cleanup_old_logs(&paths.logs, retention_days, today));
    }

    RuntimeLogger {
        enabled: active,
        debug,
        level,
        command,
        log_path: active.then(|| log_path_for_date(&paths.logs, today)),
        warnings,
    }
}

fn load_logging_config(paths: &ConfigPaths, warnings: &mut Vec<String>) -> LoggingConfig {
    if !paths.config.exists() {
        return LoggingConfig::default();
    }

    match config::load_config_file(&paths.config) {
        Ok(file) => file.logging,
        Err(error) => {
            warnings.push(format!(
                "failed to read logging config: {}",
                sanitize_text(&error.message)
            ));
            LoggingConfig::default()
        }
    }
}

fn normalize_retention_days(value: u16) -> u16 {
    value.clamp(1, MAX_RETENTION_DAYS)
}

fn normalize_level(value: &str) -> String {
    match value {
        "debug" | "info" | "warn" | "error" => value.to_owned(),
        _ => "info".to_owned(),
    }
}

fn env_debug_enabled(env: &BTreeMap<String, String>) -> bool {
    env.get("ROSWIRE_DEBUG")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

fn cleanup_old_logs(logs_dir: &Path, retention_days: u16, today: Date) -> Vec<String> {
    let mut warnings = Vec::new();
    let threshold = today - TimeDuration::days(i64::from(retention_days));
    let entries = match fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!("failed to read logs directory: {error}"));
            return warnings;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(date) = log_date_from_path(&path) else {
            continue;
        };
        if date < threshold {
            if let Err(error) = fs::remove_file(&path) {
                warnings.push(format!("failed to remove old log file: {error}"));
            }
        }
    }

    warnings
}

fn log_path_for_date(logs_dir: &Path, date: Date) -> PathBuf {
    logs_dir.join(format!("roswire-{}.log", date))
}

fn log_date_from_path(path: &Path) -> Option<Date> {
    let name = path.file_name()?.to_str()?;
    let date = name.strip_prefix("roswire-")?.strip_suffix(".log")?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::from_calendar_date(year, month, day).ok()
}

fn command_name(tokens: &[String]) -> String {
    match tokens {
        [] => "unknown".to_owned(),
        [file, action, ..] if file == "file" => format!("file/{action}"),
        [command, ..] if command == "import" => "import".to_owned(),
        [command, action, ..] if command == "backup" || command == "export" => {
            format!("{command}/{action}")
        }
        [command, subcommand, ..] if command == "config" || command == "secret" => {
            format!("{command}/{subcommand}")
        }
        _ => tokens
            .iter()
            .take_while(|token| !token.contains('='))
            .map(|token| sanitize_text(token))
            .collect::<Vec<_>>()
            .join("/"),
    }
}

fn sanitize_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if error::is_sensitive_key(&key) {
                        (key, Value::String("***REDACTED***".to_owned()))
                    } else {
                        (key, sanitize_json_value(value))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize_json_value).collect()),
        Value::String(value) => Value::String(sanitize_text(&value)),
        other => other,
    }
}

fn sanitize_text(value: &str) -> String {
    if looks_like_absolute_path(value) {
        return redact_local_path(value);
    }

    value
        .split_whitespace()
        .map(|part| {
            if looks_like_absolute_path(part) {
                redact_local_path(part)
            } else if let Some((key, value)) = part.split_once('=') {
                if error::is_sensitive_key(key) {
                    format!("{key}=***REDACTED***")
                } else if looks_like_absolute_path(value) {
                    format!("{key}={}", redact_local_path(value))
                } else {
                    part.to_owned()
                }
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_absolute_path(value: &str) -> bool {
    value.starts_with('/') || value.starts_with("/Users/") || value.starts_with("/Volumes/")
}

fn redact_local_path(path: &str) -> String {
    let path = Path::new(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("path");
    format!("***REDACTED***/{file_name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Cli;
    use crate::error::{ErrorContext, RosWireError};
    use clap::Parser;
    use std::fs;
    use time::Month;

    #[test]
    fn disabled_logging_does_not_create_files() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1

[logging]
enabled = false
retention_days = 30
level = "info"
"#,
        );
        let cli = Cli::try_parse_from(["roswire", "config", "profiles", "--debug"])
            .expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);

        let mut logger = initialize_for_date(&cli, &env, test_date(17));
        logger.log_start();
        logger.log_success();

        assert!(!temp.path().join("logs").exists());
        assert!(logger
            .debug_payload()
            .expect("debug payload should exist")
            .contains("disabled"));
    }

    #[test]
    fn jsonl_logging_writes_stable_redacted_events() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        write_config(
            temp.path(),
            r#"
version = 1

[logging]
enabled = true
retention_days = 30
level = "debug"
"#,
        );
        let cli = Cli::try_parse_from([
            "roswire",
            "--debug",
            "file",
            "upload",
            "/Users/example/private/setup.rsc",
            "flash/setup.rsc",
        ])
        .expect("cli should parse");
        let env = BTreeMap::from([("ROSWIRE_HOME".to_owned(), temp.path().display().to_string())]);
        let mut logger = initialize_for_date(&cli, &env, test_date(17));

        logger.log_start();
        logger.log_error(
            &RosWireError::config(
                "failed with password=super-secret path=/Users/example/private/id_ed25519",
            )
            .with_context(ErrorContext {
                command: "file/upload".to_owned(),
                host: "198.51.100.10".to_owned(),
                resolved_args: BTreeMap::from([
                    ("password".to_owned(), "super-secret".to_owned()),
                    (
                        "ssh_key".to_owned(),
                        "/Users/example/.ssh/id_ed25519".to_owned(),
                    ),
                ]),
                ..ErrorContext::default()
            }),
        );

        let log = fs::read_to_string(temp.path().join("logs/roswire-2026-05-17.log"))
            .expect("log file should exist");
        let lines = log.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"schema_version\":\"roswire.log.v1\""));
        assert!(lines[0].contains("\"command\":\"file/upload\""));
        assert!(lines[1].contains("\"error_code\":\"ConfigError\""));
        assert!(!log.contains("super-secret"));
        assert!(!log.contains("/Users/example/private"));
        assert!(log.contains("***REDACTED***/id_ed25519"));
    }

    #[test]
    fn retention_cleanup_deletes_old_logs_only() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let logs = temp.path().join("logs");
        fs::create_dir_all(&logs).expect("logs dir should be created");
        fs::write(logs.join("roswire-2026-04-01.log"), "old\n").expect("old log");
        fs::write(logs.join("roswire-2026-05-10.log"), "new\n").expect("new log");
        fs::write(logs.join("notes.txt"), "keep\n").expect("other file");

        let warnings = cleanup_old_logs(&logs, 30, test_date(17));

        assert!(warnings.is_empty());
        assert!(!logs.join("roswire-2026-04-01.log").exists());
        assert!(logs.join("roswire-2026-05-10.log").exists());
        assert!(logs.join("notes.txt").exists());
    }

    #[test]
    fn retention_and_level_are_normalized() {
        assert_eq!(normalize_retention_days(0), 1);
        assert_eq!(normalize_retention_days(90), 30);
        assert_eq!(normalize_level("debug"), "debug");
        assert_eq!(normalize_level("trace"), "info");
        assert!(env_debug_enabled(&BTreeMap::from([(
            "ROSWIRE_DEBUG".to_owned(),
            "true".to_owned(),
        )])));
    }

    fn test_date(day: u8) -> Date {
        Date::from_calendar_date(2026, Month::May, day).expect("test date should be valid")
    }

    fn write_config(home: &Path, contents: &str) {
        fs::write(home.join("config.toml"), contents).expect("config should be written");
        #[cfg(unix)]
        {
            fs::set_permissions(home, fs::Permissions::from_mode(0o700))
                .expect("home permissions should be set");
            fs::set_permissions(home.join("config.toml"), fs::Permissions::from_mode(0o600))
                .expect("config permissions should be set");
        }
    }
}
