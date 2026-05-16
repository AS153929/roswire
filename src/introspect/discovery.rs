use crate::introspect::cache::{compute_cache_key, DeviceFingerprint};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteOverlayCommand {
    pub name: String,
    pub support: String,
    pub output_fields_observed: Vec<String>,
    pub runtime_value_hints: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempted_side_effects_override: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempted_idempotency_override: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StaticCommandPolicy {
    pub name: String,
    pub side_effects: Vec<String>,
    pub idempotency: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MergedCommand {
    pub name: String,
    pub support: String,
    pub schema_source: Vec<String>,
    pub side_effects: Vec<String>,
    pub idempotency: String,
    pub output_fields_observed: Vec<String>,
    pub runtime_value_hints: BTreeMap<String, Vec<String>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteSchemaCacheStatus {
    pub status: String,
    pub ttl_seconds: u64,
    pub cache_key: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteSchemaSnapshot {
    pub schema_version: String,
    pub schema_source: Vec<String>,
    pub profile: String,
    pub device: DeviceFingerprint,
    pub cache: RemoteSchemaCacheStatus,
    pub commands: Vec<MergedCommand>,
    pub warnings: Vec<String>,
}

pub fn merge_overlay(
    policy: &StaticCommandPolicy,
    overlay: &RemoteOverlayCommand,
) -> MergedCommand {
    MergedCommand {
        name: policy.name.clone(),
        support: overlay.support.clone(),
        schema_source: vec!["static_catalog".to_owned(), "remote_overlay".to_owned()],
        side_effects: policy.side_effects.clone(),
        idempotency: policy.idempotency.clone(),
        output_fields_observed: overlay.output_fields_observed.clone(),
        runtime_value_hints: overlay.runtime_value_hints.clone(),
        warnings: overlay.warnings.clone(),
    }
}

pub fn remote_schema_unavailable_snapshot(
    profile: &str,
    fingerprint: &DeviceFingerprint,
) -> RemoteSchemaSnapshot {
    RemoteSchemaSnapshot {
        schema_version: "roswire.remote.schema.v1".to_owned(),
        schema_source: vec!["static_catalog".to_owned(), "remote_overlay".to_owned()],
        profile: profile.to_owned(),
        device: fingerprint.clone(),
        cache: RemoteSchemaCacheStatus {
            status: "unavailable".to_owned(),
            ttl_seconds: 604_800,
            cache_key: compute_cache_key(profile, fingerprint),
        },
        commands: Vec::new(),
        warnings: vec!["REMOTE_SCHEMA_UNAVAILABLE".to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        merge_overlay, remote_schema_unavailable_snapshot, RemoteOverlayCommand,
        StaticCommandPolicy,
    };
    use crate::introspect::cache::{hash_host_id, DeviceFingerprint};
    use std::collections::BTreeMap;

    fn fingerprint() -> DeviceFingerprint {
        DeviceFingerprint {
            host_id_hashed: hash_host_id("192.168.88.1"),
            routeros_version: "7.15.3".to_owned(),
            build_time: "2026-01-01".to_owned(),
            architecture: "arm64".to_owned(),
            board_name: "RB5009".to_owned(),
            packages_hash: "pkg-hash".to_owned(),
            selected_protocol: "rest".to_owned(),
        }
    }

    #[test]
    fn merge_keeps_static_safety_fields() {
        let policy = StaticCommandPolicy {
            name: "ip address add".to_owned(),
            side_effects: vec!["creates-routeros-record".to_owned()],
            idempotency: "not-idempotent".to_owned(),
        };
        let overlay = RemoteOverlayCommand {
            name: "ip address add".to_owned(),
            support: "supported".to_owned(),
            output_fields_observed: vec![".id".to_owned(), "address".to_owned()],
            runtime_value_hints: BTreeMap::from([(
                "interface".to_owned(),
                vec!["bridge".to_owned(), "ether1".to_owned()],
            )]),
            attempted_side_effects_override: Some(vec!["none".to_owned()]),
            attempted_idempotency_override: Some("idempotent".to_owned()),
            warnings: Vec::new(),
        };

        let merged = merge_overlay(&policy, &overlay);

        assert_eq!(merged.side_effects, vec!["creates-routeros-record"]);
        assert_eq!(merged.idempotency, "not-idempotent");
        assert_eq!(merged.support, "supported");
        assert_eq!(
            merged.runtime_value_hints.get("interface"),
            Some(&vec!["bridge".to_owned(), "ether1".to_owned()])
        );
    }

    #[test]
    fn unavailable_snapshot_has_warning_and_hashed_cache_key() {
        let fp = fingerprint();
        let snapshot = remote_schema_unavailable_snapshot("home", &fp);

        assert_eq!(snapshot.schema_version, "roswire.remote.schema.v1");
        assert!(snapshot
            .warnings
            .iter()
            .any(|w| w == "REMOTE_SCHEMA_UNAVAILABLE"));
        assert!(snapshot.cache.cache_key.starts_with("cache:"));
        assert!(!snapshot.cache.cache_key.contains("192.168.88.1"));
    }
}
