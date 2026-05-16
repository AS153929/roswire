use crate::protocol::classic::ResourceInfo;
use crate::protocol::RouterOsMajor;
use std::collections::BTreeMap;

pub trait Dialect {
    fn name(&self) -> &'static str;
    fn routeros_major(&self) -> RouterOsMajor;
    fn normalize_fields(
        &self,
        command: &str,
        row: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String>;
    fn command_supported(&self, command: &str) -> bool;
    fn error_hint(&self, error_message: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicDialect {
    Unknown,
    V6,
    V7,
}

impl ClassicDialect {
    pub fn from_resource_info(resource: &ResourceInfo) -> Self {
        Self::from_version(&resource.version)
    }

    pub fn from_version(version: &str) -> Self {
        let trimmed = version.trim_start();
        if trimmed.starts_with('6') {
            Self::V6
        } else if trimmed.starts_with('7') {
            Self::V7
        } else {
            Self::Unknown
        }
    }
}

impl Dialect for ClassicDialect {
    fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::V6 => "v6",
            Self::V7 => "v7",
        }
    }

    fn routeros_major(&self) -> RouterOsMajor {
        match self {
            Self::Unknown => RouterOsMajor::Unknown,
            Self::V6 => RouterOsMajor::V6,
            Self::V7 => RouterOsMajor::V7,
        }
    }

    fn normalize_fields(
        &self,
        command: &str,
        row: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut normalized = row.clone();
        normalize_resource_fields(&mut normalized);
        if command == "interface print" {
            normalize_interface_fields(&mut normalized);
        }
        if command == "ip address print" {
            normalize_ip_address_fields(&mut normalized);
        }
        normalized
    }

    fn command_supported(&self, command: &str) -> bool {
        match (self, command) {
            (Self::V6, "rest") => false,
            (Self::V6, "system resource print") => true,
            (Self::V6, "interface print") => true,
            (Self::V6, "ip address print") => true,
            (Self::V6, "ip address add") => true,
            (Self::V6, "ip address set") => true,
            (Self::V6, "ip address remove") => true,
            (Self::V6, _) => false,
            (Self::V7, _) => true,
            (Self::Unknown, _) => true,
        }
    }

    fn error_hint(&self, error_message: &str) -> Option<String> {
        let message = error_message.to_ascii_lowercase();
        if message.contains("no such item") {
            return Some("refresh item IDs with a print command before retrying".to_owned());
        }
        match self {
            Self::V6 if message.contains("unknown parameter") => Some(
                "RouterOS v6 may use a different field name; inspect schema/print output first"
                    .to_owned(),
            ),
            Self::V7 if message.contains("not found") => {
                Some("RouterOS v7 REST/native paths can differ; verify command support".to_owned())
            }
            _ => None,
        }
    }
}

pub fn normalize_rows(
    dialect: &impl Dialect,
    command: &str,
    rows: &[BTreeMap<String, String>],
) -> Vec<BTreeMap<String, String>> {
    rows.iter()
        .map(|row| dialect.normalize_fields(command, row))
        .collect()
}

fn normalize_resource_fields(row: &mut BTreeMap<String, String>) {
    copy_alias(row, "architecture", "architecture-name");
    copy_alias(row, "architecture-name", "architecture");
    copy_alias(row, "board", "board-name");
}

fn normalize_interface_fields(row: &mut BTreeMap<String, String>) {
    copy_alias(row, "mac-address", "mac_address");
    copy_alias(row, "actual-mtu", "actual_mtu");
    copy_alias(row, "l2mtu", "l2_mtu");
}

fn normalize_ip_address_fields(row: &mut BTreeMap<String, String>) {
    copy_alias(row, "network", "network-address");
    copy_alias(row, "actual-interface", "interface");
}

fn copy_alias(row: &mut BTreeMap<String, String>, from: &str, to: &str) {
    if row.contains_key(to) {
        return;
    }
    if let Some(value) = row.get(from).cloned() {
        row.insert(to.to_owned(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_rows, ClassicDialect, Dialect};
    use crate::protocol::classic::ResourceInfo;
    use crate::protocol::RouterOsMajor;
    use std::collections::BTreeMap;

    #[test]
    fn dialect_is_selected_from_resource_version() {
        let v6 = ResourceInfo {
            version: "6.49.10".to_owned(),
            architecture: "mipsbe".to_owned(),
            board_name: "RB2011".to_owned(),
        };
        let v7 = ResourceInfo {
            version: "7.15.3 (stable)".to_owned(),
            architecture: "arm64".to_owned(),
            board_name: "RB5009".to_owned(),
        };

        assert_eq!(ClassicDialect::from_resource_info(&v6), ClassicDialect::V6);
        assert_eq!(ClassicDialect::from_resource_info(&v7), ClassicDialect::V7);
        assert_eq!(
            ClassicDialect::from_version("unknown"),
            ClassicDialect::Unknown
        );
        assert_eq!(ClassicDialect::V6.name(), "v6");
        assert_eq!(ClassicDialect::V7.routeros_major(), RouterOsMajor::V7);
    }

    #[test]
    fn v6_and_v7_resource_fixtures_normalize_to_common_fields() {
        let v6_row = BTreeMap::from([
            ("version".to_owned(), "6.49.10".to_owned()),
            ("architecture".to_owned(), "mipsbe".to_owned()),
            ("board".to_owned(), "RB2011".to_owned()),
        ]);
        let v7_row = BTreeMap::from([
            ("version".to_owned(), "7.15.3".to_owned()),
            ("architecture-name".to_owned(), "arm64".to_owned()),
            ("board-name".to_owned(), "RB5009".to_owned()),
        ]);

        let normalized_v6 = ClassicDialect::V6.normalize_fields("system resource print", &v6_row);
        let normalized_v7 = ClassicDialect::V7.normalize_fields("system resource print", &v7_row);

        assert_eq!(
            normalized_v6.get("architecture-name"),
            Some(&"mipsbe".to_owned())
        );
        assert_eq!(normalized_v6.get("board-name"), Some(&"RB2011".to_owned()));
        assert_eq!(normalized_v7.get("architecture"), Some(&"arm64".to_owned()));
        assert_eq!(normalized_v7.get("board-name"), Some(&"RB5009".to_owned()));
    }

    #[test]
    fn unknown_fields_are_preserved_without_panic() {
        let row = BTreeMap::from([
            ("name".to_owned(), "ether1".to_owned()),
            ("custom-vendor-field".to_owned(), "kept".to_owned()),
            ("mac-address".to_owned(), "AA:BB:CC:DD:EE:FF".to_owned()),
        ]);

        let normalized = ClassicDialect::Unknown.normalize_fields("interface print", &row);

        assert_eq!(
            normalized.get("custom-vendor-field"),
            Some(&"kept".to_owned())
        );
        assert_eq!(
            normalized.get("mac_address"),
            Some(&"AA:BB:CC:DD:EE:FF".to_owned())
        );
    }

    #[test]
    fn command_support_tracks_v6_v7_differences() {
        assert!(!ClassicDialect::V6.command_supported("rest"));
        assert!(ClassicDialect::V6.command_supported("interface print"));
        assert!(ClassicDialect::V7.command_supported("rest"));
        assert!(ClassicDialect::Unknown.command_supported("future command"));
    }

    #[test]
    fn dialect_specific_error_hints_are_available() {
        assert_eq!(
            ClassicDialect::V6
                .error_hint("unknown parameter foo")
                .as_deref(),
            Some("RouterOS v6 may use a different field name; inspect schema/print output first"),
        );
        assert_eq!(
            ClassicDialect::V7.error_hint("item not found").as_deref(),
            Some("RouterOS v7 REST/native paths can differ; verify command support"),
        );
        assert_eq!(
            ClassicDialect::Unknown
                .error_hint("no such item")
                .as_deref(),
            Some("refresh item IDs with a print command before retrying"),
        );
    }

    #[test]
    fn normalizes_rows_for_snapshot_like_comparisons() {
        let rows = vec![BTreeMap::from([
            ("address".to_owned(), "192.0.2.1/24".to_owned()),
            ("actual-interface".to_owned(), "bridge".to_owned()),
        ])];

        let normalized = normalize_rows(&ClassicDialect::V7, "ip address print", &rows);

        assert_eq!(normalized[0].get("interface"), Some(&"bridge".to_owned()));
        assert_eq!(
            normalized[0].get("address"),
            Some(&"192.0.2.1/24".to_owned())
        );
    }
}
