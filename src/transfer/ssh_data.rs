use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SshRuntimeConfig {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) user: String,
    pub(super) password: Option<String>,
    pub(super) key_path: Option<String>,
    pub(super) key_passphrase: Option<String>,
    pub(super) expected_host_key: String,
    pub(super) jump: Vec<crate::jump::ResolvedJumpHop>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControlRuntimeConfig {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) user: String,
    pub(super) password: String,
    pub(super) selected_protocol: String,
    pub(super) tls_cert_fingerprint: Option<String>,
    pub(super) jump: Vec<crate::jump::ResolvedJumpHop>,
}

impl ControlRuntimeConfig {
    pub(super) fn tls_trust(&self) -> TlsTrust {
        TlsTrust::from_fingerprint(self.tls_cert_fingerprint.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SshServiceSnapshot {
    pub(super) id: Option<String>,
    pub(super) disabled: bool,
    pub(super) address: Vec<String>,
}

impl Default for SshServiceSnapshot {
    fn default() -> Self {
        Self {
            id: Some("ssh".to_owned()),
            disabled: false,
            address: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ControlCommand {
    Import { file_name: String },
    BackupSave { name: String },
    Export { file: String, compact: bool },
}

impl ControlCommand {
    pub(super) fn classic_words(&self) -> Vec<String> {
        match self {
            Self::Import { file_name } => {
                vec!["/import".to_owned(), format!("=file-name={file_name}")]
            }
            Self::BackupSave { name } => {
                vec!["/system/backup/save".to_owned(), format!("=name={name}")]
            }
            Self::Export { file, compact } => {
                let mut words = vec!["/export".to_owned(), format!("=file={file}")];
                if *compact {
                    words.push("=compact=yes".to_owned());
                }
                words
            }
        }
    }

    pub(super) fn rest_request(&self) -> (&'static str, Value) {
        match self {
            Self::Import { file_name } => ("/rest/import", json!({ "file-name": file_name })),
            Self::BackupSave { name } => ("/rest/system/backup/save", json!({ "name": name })),
            Self::Export { file, compact } => {
                let mut body =
                    serde_json::Map::from_iter([("file".to_owned(), Value::String(file.clone()))]);
                if *compact {
                    body.insert("compact".to_owned(), Value::String("yes".to_owned()));
                }
                ("/rest/export", Value::Object(body))
            }
        }
    }
}

pub(super) fn ssh_service_snapshot_from_fields(
    fields: &BTreeMap<String, String>,
) -> SshServiceSnapshot {
    SshServiceSnapshot {
        id: fields
            .get(".id")
            .cloned()
            .or_else(|| fields.get("id").cloned())
            .or_else(|| Some("ssh".to_owned())),
        disabled: fields
            .get("disabled")
            .is_some_and(|value| routeros_bool_is_true(value)),
        address: fields
            .get("address")
            .map(|value| parse_address_list(value))
            .unwrap_or_default(),
    }
}

pub(super) fn ssh_service_snapshot_from_json(value: &Value) -> RosWireResult<SshServiceSnapshot> {
    let entry = match value {
        Value::Array(items) => items
            .iter()
            .find(|item| {
                value_string(item.get("name").unwrap_or(&Value::Null)).as_deref() == Some("ssh")
            })
            .ok_or_else(|| {
                Box::new(RosWireError::ros_api_failure(
                    "RouterOS SSH service was not found",
                ))
            })?,
        Value::Object(_) => value,
        _ => {
            return Err(Box::new(RosWireError::ros_api_failure(
                "RouterOS SSH service response has unexpected shape",
            )))
        }
    };
    let object = entry.as_object().ok_or_else(|| {
        Box::new(RosWireError::ros_api_failure(
            "RouterOS SSH service response item has unexpected shape",
        ))
    })?;

    Ok(SshServiceSnapshot {
        id: object
            .get(".id")
            .and_then(value_string)
            .or_else(|| object.get("id").and_then(value_string))
            .or_else(|| Some("ssh".to_owned())),
        disabled: object
            .get("disabled")
            .and_then(value_string)
            .is_some_and(|value| routeros_bool_is_true(&value)),
        address: object
            .get("address")
            .and_then(value_string)
            .map(|value| parse_address_list(&value))
            .unwrap_or_default(),
    })
}

pub(super) fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(true) => Some("yes".to_owned()),
        Value::Bool(false) => Some("no".to_owned()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn parse_address_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn routeros_bool(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

pub(super) fn routeros_bool_is_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "1"
    )
}

pub(super) fn merge_ssh_allow_list(existing: &[String], allow_from: &[String]) -> Vec<String> {
    let mut merged = if existing.is_empty() {
        Vec::new()
    } else {
        existing.to_vec()
    };
    for cidr in allow_from {
        if !merged.iter().any(|item| item == cidr) {
            merged.push(cidr.clone());
        }
    }
    merged
}
