use crate::introspect::cache::{compute_cache_key, DeviceFingerprint};
use crate::introspect::CommandDefinition;
use crate::{args::ParsedInvocation, error::ErrorCode};
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

pub fn degraded_remote_schema_snapshot(
    profile: &str,
    fingerprint: &DeviceFingerprint,
    policies: Vec<StaticCommandPolicy>,
    warning: impl Into<String>,
) -> RemoteSchemaSnapshot {
    let warning = warning.into();
    let commands = policies
        .into_iter()
        .map(|policy| {
            let overlay = RemoteOverlayCommand {
                name: policy.name.clone(),
                support: "unknown".to_owned(),
                output_fields_observed: static_output_fields(&policy.name),
                runtime_value_hints: BTreeMap::new(),
                attempted_side_effects_override: None,
                attempted_idempotency_override: None,
                warnings: vec![warning.clone()],
            };
            merge_overlay(&policy, &overlay)
        })
        .collect();

    RemoteSchemaSnapshot {
        schema_version: "roswire.remote.schema.v1".to_owned(),
        schema_source: vec!["static_catalog".to_owned(), "remote_overlay".to_owned()],
        profile: profile.to_owned(),
        device: fingerprint.clone(),
        cache: RemoteSchemaCacheStatus {
            status: "miss".to_owned(),
            ttl_seconds: 604_800,
            cache_key: compute_cache_key(profile, fingerprint),
        },
        commands,
        warnings: vec![warning],
    }
}

pub fn policy_from_command(command: &CommandDefinition) -> Option<StaticCommandPolicy> {
    let tokens = command.name.split_whitespace().collect::<Vec<_>>();
    let action = tokens.last()?;
    let path = tokens[..tokens.len().saturating_sub(1)]
        .iter()
        .map(|token| (*token).to_owned())
        .collect::<Vec<_>>();
    let invocation = ParsedInvocation {
        path,
        action: (*action).to_owned(),
        resolved_args: BTreeMap::new(),
    };
    let mapping = crate::mapping::resolve_mapping(&invocation).ok()?;

    Some(StaticCommandPolicy {
        name: command.name.clone(),
        side_effects: mapping.side_effects,
        idempotency: mapping.idempotency,
    })
}

pub fn policies_from_catalog(commands: &[CommandDefinition]) -> Vec<StaticCommandPolicy> {
    commands.iter().filter_map(policy_from_command).collect()
}

pub fn unknown_fingerprint(host: &str, selected_protocol: &str) -> DeviceFingerprint {
    DeviceFingerprint {
        host_id_hashed: crate::introspect::cache::hash_host_id(host),
        routeros_version: "unknown".to_owned(),
        build_time: "unknown".to_owned(),
        architecture: "unknown".to_owned(),
        board_name: "unknown".to_owned(),
        packages_hash: "unknown".to_owned(),
        selected_protocol: selected_protocol.to_owned(),
    }
}

pub fn warning_name(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "CAPABILITY_PROBE_FAILED".to_owned())
}

fn static_output_fields(command: &str) -> Vec<String> {
    match command {
        "system resource print" => vec![
            "version".to_owned(),
            "architecture-name".to_owned(),
            "board-name".to_owned(),
        ],
        "system package print" => vec![
            ".id".to_owned(),
            "name".to_owned(),
            "version".to_owned(),
            "build-time".to_owned(),
            "disabled".to_owned(),
        ],
        "user print" => vec![
            ".id".to_owned(),
            "name".to_owned(),
            "group".to_owned(),
            "address".to_owned(),
            "disabled".to_owned(),
            "last-logged-in".to_owned(),
        ],
        "interface print" => vec![".id".to_owned(), "name".to_owned(), "disabled".to_owned()],
        "interface wireguard print" => vec![
            ".id".to_owned(),
            "name".to_owned(),
            "listen-port".to_owned(),
            "mtu".to_owned(),
            "running".to_owned(),
            "disabled".to_owned(),
        ],
        "interface wireguard peers print" => vec![
            ".id".to_owned(),
            "interface".to_owned(),
            "public-key".to_owned(),
            "endpoint-address".to_owned(),
            "endpoint-port".to_owned(),
            "allowed-address".to_owned(),
            "disabled".to_owned(),
            "comment".to_owned(),
        ],
        "ip address print" => vec![
            ".id".to_owned(),
            "address".to_owned(),
            "network".to_owned(),
            "interface".to_owned(),
            "disabled".to_owned(),
        ],
        "ip route print" => vec![
            ".id".to_owned(),
            "dst-address".to_owned(),
            "gateway".to_owned(),
            "distance".to_owned(),
            "routing-table".to_owned(),
            "pref-src".to_owned(),
            "active".to_owned(),
            "dynamic".to_owned(),
            "disabled".to_owned(),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        degraded_remote_schema_snapshot, merge_overlay, policies_from_catalog,
        remote_schema_unavailable_snapshot, unknown_fingerprint, warning_name,
        RemoteOverlayCommand, StaticCommandPolicy,
    };
    use crate::error::ErrorCode;
    use crate::introspect::cache::{hash_host_id, DeviceFingerprint};
    use crate::introspect::CommandDefinition;
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

    #[test]
    fn degraded_snapshot_keeps_static_policy_and_uses_hashed_cache_key() {
        let fp = unknown_fingerprint("198.51.100.10", "unknown");
        let policies = vec![StaticCommandPolicy {
            name: "ip address add".to_owned(),
            side_effects: vec!["creates-routeros-record".to_owned()],
            idempotency: "not-idempotent".to_owned(),
        }];

        let snapshot = degraded_remote_schema_snapshot(
            "studio",
            &fp,
            policies,
            warning_name(ErrorCode::NetworkError),
        );

        assert_eq!(snapshot.cache.status, "miss");
        assert!(!snapshot.cache.cache_key.contains("198.51.100.10"));
        assert_eq!(snapshot.commands[0].support, "unknown");
        assert_eq!(
            snapshot.commands[0].side_effects,
            vec!["creates-routeros-record"]
        );
        assert_eq!(snapshot.commands[0].idempotency, "not-idempotent");
        assert!(snapshot.warnings.iter().any(|item| item == "NETWORK_ERROR"));
    }

    #[test]
    fn policies_from_catalog_filters_to_routeros_mapped_commands() {
        let commands = vec![
            CommandDefinition {
                name: "ip address print".to_owned(),
                summary: String::new(),
                kind: "routeros-command".to_owned(),
                syntax: String::new(),
                arguments: Vec::new(),
                examples: Vec::new(),
                errors: Vec::new(),
            },
            CommandDefinition {
                name: "config inspect".to_owned(),
                summary: String::new(),
                kind: "config".to_owned(),
                syntax: String::new(),
                arguments: Vec::new(),
                examples: Vec::new(),
                errors: Vec::new(),
            },
        ];

        let policies = policies_from_catalog(&commands);

        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].name, "ip address print");
        assert_eq!(policies[0].idempotency, "read-only");
    }

    #[test]
    fn degraded_snapshot_includes_system_package_static_fields() {
        let fp = unknown_fingerprint("198.51.100.10", "unknown");
        let policies = vec![StaticCommandPolicy {
            name: "system package print".to_owned(),
            side_effects: Vec::new(),
            idempotency: "read-only".to_owned(),
        }];

        let snapshot = degraded_remote_schema_snapshot(
            "studio",
            &fp,
            policies,
            warning_name(ErrorCode::ConfigError),
        );

        assert_eq!(snapshot.commands[0].name, "system package print");
        assert_eq!(snapshot.commands[0].idempotency, "read-only");
        assert_eq!(
            snapshot.commands[0].output_fields_observed,
            vec![".id", "name", "version", "build-time", "disabled"]
        );
    }

    #[test]
    fn degraded_snapshot_includes_user_static_fields() {
        let fp = unknown_fingerprint("198.51.100.10", "unknown");
        let policies = vec![StaticCommandPolicy {
            name: "user print".to_owned(),
            side_effects: Vec::new(),
            idempotency: "read-only".to_owned(),
        }];

        let snapshot = degraded_remote_schema_snapshot(
            "studio",
            &fp,
            policies,
            warning_name(ErrorCode::ConfigError),
        );

        assert_eq!(snapshot.commands[0].name, "user print");
        assert_eq!(snapshot.commands[0].idempotency, "read-only");
        assert_eq!(
            snapshot.commands[0].output_fields_observed,
            vec![
                ".id",
                "name",
                "group",
                "address",
                "disabled",
                "last-logged-in"
            ]
        );
    }

    #[test]
    fn degraded_snapshot_includes_ip_route_static_fields() {
        let fp = unknown_fingerprint("198.51.100.10", "unknown");
        let policies = vec![StaticCommandPolicy {
            name: "ip route print".to_owned(),
            side_effects: Vec::new(),
            idempotency: "read-only".to_owned(),
        }];

        let snapshot = degraded_remote_schema_snapshot(
            "studio",
            &fp,
            policies,
            warning_name(ErrorCode::ConfigError),
        );

        assert_eq!(snapshot.commands[0].name, "ip route print");
        assert_eq!(snapshot.commands[0].idempotency, "read-only");
        assert_eq!(
            snapshot.commands[0].output_fields_observed,
            vec![
                ".id",
                "dst-address",
                "gateway",
                "distance",
                "routing-table",
                "pref-src",
                "active",
                "dynamic",
                "disabled"
            ]
        );
    }

    #[test]
    fn degraded_snapshot_includes_wireguard_static_fields_without_private_material() {
        let fp = unknown_fingerprint("198.51.100.10", "unknown");
        let policies = vec![
            StaticCommandPolicy {
                name: "interface wireguard print".to_owned(),
                side_effects: Vec::new(),
                idempotency: "read-only".to_owned(),
            },
            StaticCommandPolicy {
                name: "interface wireguard peers print".to_owned(),
                side_effects: Vec::new(),
                idempotency: "read-only".to_owned(),
            },
        ];

        let snapshot = degraded_remote_schema_snapshot(
            "studio",
            &fp,
            policies,
            warning_name(ErrorCode::ConfigError),
        );

        assert_eq!(snapshot.commands[0].name, "interface wireguard print");
        assert!(snapshot.commands[0]
            .output_fields_observed
            .iter()
            .all(|field| !field.contains("private")));
        assert_eq!(
            snapshot.commands[1].output_fields_observed,
            vec![
                ".id",
                "interface",
                "public-key",
                "endpoint-address",
                "endpoint-port",
                "allowed-address",
                "disabled",
                "comment"
            ]
        );
        assert!(snapshot.commands[1]
            .output_fields_observed
            .iter()
            .all(|field| !field.contains("preshared")));
    }
}
