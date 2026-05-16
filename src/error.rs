use serde::Serialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ErrorContext {
    pub command: String,
    pub requested_protocol: String,
    pub selected_protocol: String,
    pub routeros_version: String,
    pub host: String,
    pub resolved_args: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Error, Serialize)]
#[error("{message}")]
pub struct RosWireError {
    pub error_code: String,
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
            error_code: "USAGE_ERROR".to_owned(),
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 2,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error_code: "INTERNAL_ERROR".to_owned(),
            message: message.into(),
            hint: None,
            context: ErrorContext::default(),
            exit_code: 5,
        }
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

#[cfg(test)]
mod tests {
    use super::RosWireError;

    #[test]
    fn usage_error_has_expected_code_and_exit_code() {
        let error = RosWireError::usage("missing arguments");
        assert_eq!(error.error_code, "USAGE_ERROR");
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
    }

    #[test]
    fn print_to_stderr_does_not_panic() {
        RosWireError::usage("oops").print_to_stderr();
    }
}
