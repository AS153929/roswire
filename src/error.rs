use serde::Serialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UsageError,
    ConfigError,
    ProfileNotFound,
    ConfigInsecurePermissions,
    AuthFailed,
    NetworkError,
    RosApiFailure,
    InternalError,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorContext {
    pub command: String,
    pub requested_protocol: String,
    pub selected_protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_backend: Option<String>,
    pub routeros_version: String,
    pub host: String,
    pub resolved_args: BTreeMap<String, String>,
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            command: String::new(),
            requested_protocol: "unknown".to_owned(),
            selected_protocol: "unknown".to_owned(),
            transfer_backend: None,
            routeros_version: "unknown".to_owned(),
            host: String::new(),
            resolved_args: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Error, Serialize)]
#[error("{message}")]
pub struct RosWireError {
    pub error_code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub context: ErrorContext,
    #[serde(skip)]
    pub exit_code: u8,
}

pub type RosWireResult<T> = Result<T, Box<RosWireError>>;

impl RosWireError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::UsageError,
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 2,
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::ConfigError,
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 2,
        }
    }

    pub fn auth_failed(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::AuthFailed,
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 3,
        }
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::NetworkError,
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 4,
        }
    }

    pub fn ros_api_failure(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::RosApiFailure,
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 1,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::InternalError,
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 5,
        }
    }

    pub fn profile_not_found(profile: impl Into<String>) -> Self {
        let profile = profile.into();
        Self {
            error_code: ErrorCode::ProfileNotFound,
            message: format!("profile not found: {profile}"),
            hint: Some("set --profile, ROS_PROFILE, or default_profile".to_owned()),
            context: ErrorContext::default(),
            exit_code: 2,
        }
    }

    pub fn config_insecure_permissions(message: impl Into<String>) -> Self {
        Self {
            error_code: ErrorCode::ConfigInsecurePermissions,
            message: message.into(),
            hint: Some("fix permissions to 0700 for directories and 0600 for files".to_owned()),
            context: ErrorContext::default(),
            exit_code: 2,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = context;
        self
    }

    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }

    pub fn to_json_payload(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            "{\"error_code\":\"SERIALIZATION_ERROR\",\"message\":\"failed to serialize error\"}"
                .to_owned()
        })
    }

    pub fn print_to_stderr(&self) {
        let payload = self.to_json_payload();
        eprintln!("{payload}");
    }
}

pub fn redact_value(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        "***REDACTED***".to_owned()
    }
}

pub fn is_sensitive_key(key: &str) -> bool {
    let lowercase = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "private",
        "ssh-key",
        "ssh_key",
        "ssh_password",
    ]
    .iter()
    .any(|needle| lowercase.contains(needle))
}

pub fn redact_resolved_args(args: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut sanitized = BTreeMap::new();
    for (key, value) in args {
        if is_sensitive_key(key) {
            sanitized.insert(key.clone(), redact_value(value));
        } else {
            sanitized.insert(key.clone(), value.clone());
        }
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::{
        is_sensitive_key, redact_resolved_args, redact_value, ErrorCode, ErrorContext, RosWireError,
    };
    use std::collections::BTreeMap;

    #[test]
    fn usage_error_has_expected_code_and_exit_code() {
        let error = RosWireError::usage("missing arguments");
        assert_eq!(error.error_code, ErrorCode::UsageError);
        assert_eq!(error.message, "missing arguments");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn internal_error_serializes_to_stable_json_shape() {
        let error = RosWireError::internal("unexpected");
        let payload = error.to_json_payload();
        let json: serde_json::Value =
            serde_json::from_str(&payload).expect("error payload should be valid JSON");

        assert_eq!(json["error_code"], "INTERNAL_ERROR");
        assert_eq!(json["message"], "unexpected");
        assert!(json.get("hint").is_none());
        assert!(json.get("context").is_some());
        assert!(payload.find("\"error_code\"") < payload.find("\"message\""));
        assert!(!payload.contains("timestamp"));
        assert!(!payload.contains("trace_id"));
    }

    #[test]
    fn print_to_stderr_does_not_panic() {
        RosWireError::usage("oops").print_to_stderr();
    }

    #[test]
    fn redaction_masks_sensitive_arguments() {
        let mut args = BTreeMap::new();
        args.insert("address".to_owned(), "192.168.88.2/24".to_owned());
        args.insert("password".to_owned(), "super-secret".to_owned());
        args.insert("api_token".to_owned(), "abc123".to_owned());

        let sanitized = redact_resolved_args(&args);

        assert_eq!(
            sanitized.get("address").map(String::as_str),
            Some("192.168.88.2/24")
        );
        assert_eq!(
            sanitized.get("password").map(String::as_str),
            Some("***REDACTED***")
        );
        assert_eq!(
            sanitized.get("api_token").map(String::as_str),
            Some("***REDACTED***")
        );
    }

    #[test]
    fn sensitive_key_detection_is_case_insensitive() {
        assert!(is_sensitive_key("Password"));
        assert!(is_sensitive_key("SSH_KEY_PATH"));
        assert!(is_sensitive_key("privateKey"));
        assert!(!is_sensitive_key("interface"));
    }

    #[test]
    fn constructor_exit_codes_match_contract() {
        let config = RosWireError::config("bad config");
        assert_eq!(config.error_code, ErrorCode::ConfigError);
        assert_eq!(config.exit_code(), 2);

        let profile_missing = RosWireError::profile_not_found("home");
        assert_eq!(profile_missing.error_code, ErrorCode::ProfileNotFound);
        assert_eq!(profile_missing.exit_code(), 2);

        let insecure = RosWireError::config_insecure_permissions("too wide");
        assert_eq!(insecure.error_code, ErrorCode::ConfigInsecurePermissions);
        assert_eq!(insecure.exit_code(), 2);

        let auth = RosWireError::auth_failed("invalid credentials");
        assert_eq!(auth.error_code, ErrorCode::AuthFailed);
        assert_eq!(auth.exit_code(), 3);

        let network = RosWireError::network("unreachable");
        assert_eq!(network.error_code, ErrorCode::NetworkError);
        assert_eq!(network.exit_code(), 4);

        let api = RosWireError::ros_api_failure("trap");
        assert_eq!(api.error_code, ErrorCode::RosApiFailure);
        assert_eq!(api.exit_code(), 1);
    }

    #[test]
    fn hint_and_context_are_attached() {
        let mut args = BTreeMap::new();
        args.insert("interface".to_owned(), "bridge".to_owned());

        let context = ErrorContext {
            command: "ip/address/add".to_owned(),
            requested_protocol: "auto".to_owned(),
            selected_protocol: "rest".to_owned(),
            transfer_backend: Some("ssh".to_owned()),
            routeros_version: "v7".to_owned(),
            host: "router.local".to_owned(),
            resolved_args: args,
        };

        let payload = RosWireError::usage("invalid interface")
            .with_hint("run interface print first")
            .with_context(context)
            .to_json_payload();

        let json: serde_json::Value =
            serde_json::from_str(&payload).expect("error payload should be valid JSON");

        assert_eq!(json["hint"], "run interface print first");
        assert_eq!(json["context"]["command"], "ip/address/add");
        assert_eq!(json["context"]["selected_protocol"], "rest");
        assert_eq!(json["context"]["transfer_backend"], "ssh");
    }

    #[test]
    fn redact_value_handles_empty_and_non_empty() {
        assert_eq!(redact_value(""), "");
        assert_eq!(redact_value("secret"), "***REDACTED***");
    }
}
