use crate::args::ParsedInvocation;
use crate::error::{redact_resolved_args, ErrorContext, RosWireError, RosWireResult};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingRequest {
    pub tokens: Vec<String>,
}

impl MappingRequest {
    pub fn new(tokens: Vec<String>) -> Self {
        Self { tokens }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Print,
    Add,
    Set,
    Remove,
    Raw,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Print => "print",
            Self::Add => "add",
            Self::Set => "set",
            Self::Remove => "remove",
            Self::Raw => "raw",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl RestMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestMapping {
    pub method: RestMethod,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMapping {
    pub cli_path: Vec<String>,
    pub action_kind: ActionKind,
    pub routeros_path: String,
    pub side_effects: Vec<String>,
    pub idempotency: String,
    pub rest_mapping: Option<RestMapping>,
}

impl CommandMapping {
    pub fn has_rest_mapping(&self) -> bool {
        self.rest_mapping.is_some()
    }

    pub fn is_raw(&self) -> bool {
        self.cli_path.as_slice() == ["raw"]
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProtocolRequest {
    pub mapping: CommandMapping,
    pub resolved_args: BTreeMap<String, String>,
    pub flags: Vec<String>,
}

impl std::fmt::Debug for ProtocolRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolRequest")
            .field("mapping", &self.mapping)
            .field(
                "resolved_args",
                &crate::error::redact_resolved_args(&self.resolved_args),
            )
            .field("flags", &self.flags)
            .finish()
    }
}

impl ProtocolRequest {
    pub fn classic_api_words(&self) -> Vec<String> {
        let mut words = Vec::with_capacity(1 + self.flags.len() + self.resolved_args.len());
        words.push(self.mapping.routeros_path.clone());
        words.extend(self.flags.iter().map(|flag| format!("={flag}=")));
        words.extend(
            self.resolved_args
                .iter()
                .map(|(key, value)| format!("={key}={value}")),
        );
        words
    }
}

pub fn build_protocol_request(invocation: &ParsedInvocation) -> RosWireResult<ProtocolRequest> {
    let mapping = resolve_mapping(invocation)?;
    validate_required_args(invocation, &mapping)?;
    validate_print_options(invocation, &mapping)?;

    Ok(ProtocolRequest {
        mapping,
        resolved_args: invocation.resolved_args.clone(),
        flags: invocation.flags.clone(),
    })
}

fn validate_required_args(
    invocation: &ParsedInvocation,
    mapping: &CommandMapping,
) -> RosWireResult<()> {
    let required = match (mapping.cli_path.as_slice(), mapping.action_kind) {
        ([ip, address], ActionKind::Add) if ip == "ip" && address == "address" => {
            &["address", "interface"][..]
        }
        ([ip, address], ActionKind::Set | ActionKind::Remove)
            if ip == "ip" && address == "address" =>
        {
            &[".id"][..]
        }
        ([system, script], ActionKind::Add) if system == "system" && script == "script" => {
            &["name", "source"][..]
        }
        _ => &[][..],
    };

    for name in required {
        if !invocation.resolved_args.contains_key(*name) {
            return Err(Box::new(
                RosWireError::usage(format!(
                    "missing required argument for {}: {name}=<value>",
                    command_name(invocation),
                ))
                .with_context(mapping_error_context(invocation)),
            ));
        }
    }

    Ok(())
}

fn validate_print_options(
    invocation: &ParsedInvocation,
    mapping: &CommandMapping,
) -> RosWireResult<()> {
    if mapping.action_kind != ActionKind::Print && !invocation.flags.is_empty() {
        return Err(Box::new(
            RosWireError::usage(
                "bare RouterOS options are supported only for read-only print commands",
            )
            .with_context(mapping_error_context(invocation)),
        ));
    }

    for flag in &invocation.flags {
        if !is_readonly_print_flag(flag) {
            return Err(Box::new(
                RosWireError::usage(format!("unsupported read-only print option: {flag}"))
                    .with_hint("supported bare print options are: detail, stats, count-only")
                    .with_context(mapping_error_context(invocation)),
            ));
        }
    }

    if mapping.action_kind == ActionKind::Print {
        for key in invocation.resolved_args.keys() {
            if is_unsafe_print_option(key) {
                return Err(Box::new(
                    RosWireError::usage(format!(
                        "print option `{key}` is not treated as read-only by roswire"
                    ))
                    .with_hint(
                        "use detail, stats, count-only, or explicit key=value filters/proplist only",
                    )
                    .with_context(mapping_error_context(invocation)),
                ));
            }
        }
    }

    Ok(())
}

/// Canonical, single-source-of-truth specification for a statically supported
/// RouterOS command. `resolve_mapping` is driven by [`STATIC_COMMANDS`] so the
/// executable mapping and the self-describing catalog can be cross-checked
/// against one ledger instead of drifting apart. The `raw` passthrough is the
/// only command not represented here because its action is derived dynamically
/// from the request path.
struct StaticCommand {
    cli_path: &'static [&'static str],
    action: &'static str,
    action_kind: ActionKind,
    routeros_path: &'static str,
    side_effects: &'static [&'static str],
    idempotency: &'static str,
    rest: Option<(RestMethod, &'static str)>,
}

impl StaticCommand {
    fn to_mapping(&self) -> CommandMapping {
        CommandMapping {
            cli_path: self
                .cli_path
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            action_kind: self.action_kind,
            routeros_path: self.routeros_path.to_owned(),
            side_effects: self
                .side_effects
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            idempotency: self.idempotency.to_owned(),
            rest_mapping: self.rest.map(|(method, path)| RestMapping {
                method,
                path: path.to_owned(),
            }),
        }
    }
}

const STATIC_COMMANDS: &[StaticCommand] = &[
    StaticCommand {
        cli_path: &["ip", "dhcp-client"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/dhcp-client/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/dhcp-client")),
    },
    StaticCommand {
        cli_path: &["interface"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/interface/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/interface")),
    },
    StaticCommand {
        cli_path: &["interface", "wireguard"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/interface/wireguard/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/interface/wireguard")),
    },
    StaticCommand {
        cli_path: &["interface", "wireguard", "peers"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/interface/wireguard/peers/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/interface/wireguard/peers")),
    },
    StaticCommand {
        cli_path: &["ip", "address"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/address/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/address")),
    },
    StaticCommand {
        cli_path: &["ip", "address"],
        action: "add",
        action_kind: ActionKind::Add,
        routeros_path: "/ip/address/add",
        side_effects: &["creates-routeros-record"],
        idempotency: "not-idempotent",
        rest: Some((RestMethod::Put, "/rest/ip/address")),
    },
    StaticCommand {
        cli_path: &["ip", "address"],
        action: "set",
        action_kind: ActionKind::Set,
        routeros_path: "/ip/address/set",
        side_effects: &["updates-routeros-record"],
        idempotency: "idempotent",
        rest: Some((RestMethod::Patch, "/rest/ip/address/{.id}")),
    },
    StaticCommand {
        cli_path: &["ip", "address"],
        action: "remove",
        action_kind: ActionKind::Remove,
        routeros_path: "/ip/address/remove",
        side_effects: &["deletes-routeros-record"],
        idempotency: "not-idempotent",
        rest: Some((RestMethod::Delete, "/rest/ip/address/{.id}")),
    },
    StaticCommand {
        cli_path: &["ip", "firewall", "address-list"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/firewall/address-list/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/firewall/address-list")),
    },
    StaticCommand {
        cli_path: &["ip", "firewall", "filter"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/firewall/filter/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/firewall/filter")),
    },
    StaticCommand {
        cli_path: &["ip", "firewall", "nat"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/firewall/nat/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/firewall/nat")),
    },
    StaticCommand {
        cli_path: &["ip", "firewall", "connection"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/firewall/connection/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/firewall/connection")),
    },
    StaticCommand {
        cli_path: &["ip", "route"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/ip/route/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/ip/route")),
    },
    StaticCommand {
        cli_path: &["system", "resource"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/system/resource/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/system/resource")),
    },
    StaticCommand {
        cli_path: &["system", "package"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/system/package/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/system/package")),
    },
    StaticCommand {
        cli_path: &["system", "script"],
        action: "add",
        action_kind: ActionKind::Add,
        routeros_path: "/system/script/add",
        side_effects: &["creates-routeros-script"],
        idempotency: "not-idempotent",
        rest: Some((RestMethod::Put, "/rest/system/script")),
    },
    StaticCommand {
        cli_path: &["tool", "mac-server"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/tool/mac-server/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/tool/mac-server")),
    },
    StaticCommand {
        cli_path: &["tool", "netwatch"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/tool/netwatch/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/tool/netwatch")),
    },
    StaticCommand {
        cli_path: &["user"],
        action: "print",
        action_kind: ActionKind::Print,
        routeros_path: "/user/print",
        side_effects: &[],
        idempotency: "read-only",
        rest: Some((RestMethod::Get, "/rest/user")),
    },
];

/// Owned view over every statically supported RouterOS command. The catalog
/// consistency tests cross-check this against `introspect::catalog()` so that a
/// command added to one ledger without the other fails the build. The `raw`
/// passthrough is intentionally excluded (its action is path-derived).
pub fn supported_commands() -> Vec<CommandMapping> {
    STATIC_COMMANDS
        .iter()
        .map(StaticCommand::to_mapping)
        .collect()
}

pub fn resolve_mapping(invocation: &ParsedInvocation) -> RosWireResult<CommandMapping> {
    let path = invocation
        .path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let action = invocation.action.as_str();

    if path.as_slice() == ["raw"] {
        return raw_mapping(action);
    }

    STATIC_COMMANDS
        .iter()
        .find(|command| command.cli_path == path.as_slice() && command.action == action)
        .map(StaticCommand::to_mapping)
        .ok_or_else(|| {
            Box::new(
                RosWireError::unsupported_action(format!(
                    "unsupported RouterOS action: {}",
                    command_name(invocation),
                ))
                .with_context(mapping_error_context(invocation)),
            )
        })
}

pub fn command_name(invocation: &ParsedInvocation) -> String {
    invocation
        .path
        .iter()
        .chain(std::iter::once(&invocation.action))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

fn raw_mapping(raw_path: &str) -> RosWireResult<CommandMapping> {
    let routeros_path = normalize_raw_routeros_path(raw_path)?;
    let action_kind = raw_action_kind(&routeros_path);
    let is_print = action_kind == ActionKind::Print;

    Ok(CommandMapping {
        cli_path: vec!["raw".to_owned()],
        action_kind,
        routeros_path,
        side_effects: if is_print {
            Vec::new()
        } else {
            vec!["raw-routeros-command".to_owned()]
        },
        idempotency: if is_print {
            "read-only".to_owned()
        } else {
            "unknown".to_owned()
        },
        rest_mapping: None,
    })
}

fn normalize_raw_routeros_path(raw_path: &str) -> RosWireResult<String> {
    let raw_path = raw_path.trim();
    if !raw_path.starts_with('/') {
        return Err(Box::new(RosWireError::usage(
            "raw command requires a RouterOS API path starting with `/`, e.g. roswire raw /system/resource/print --json",
        )));
    }

    if raw_path == "/" || raw_path.contains(char::is_whitespace) {
        return Err(Box::new(RosWireError::usage(
            "raw RouterOS path must be a single non-empty token without whitespace",
        )));
    }

    if raw_path.split('/').skip(1).any(str::is_empty) {
        return Err(Box::new(RosWireError::usage(
            "raw RouterOS path must not contain empty path segments",
        )));
    }

    Ok(raw_path.to_owned())
}

fn raw_action_kind(routeros_path: &str) -> ActionKind {
    match routeros_path.rsplit('/').next() {
        Some("print") => ActionKind::Print,
        Some("add") => ActionKind::Add,
        Some("set") => ActionKind::Set,
        Some("remove") => ActionKind::Remove,
        _ => ActionKind::Raw,
    }
}

pub fn is_readonly_print_flag(flag: &str) -> bool {
    matches!(flag, "detail" | "stats" | "count-only")
}

fn is_unsafe_print_option(option: &str) -> bool {
    matches!(option, "file" | "interval" | "follow" | "follow-only")
}

fn mapping_error_context(invocation: &ParsedInvocation) -> ErrorContext {
    ErrorContext {
        command: command_name(invocation),
        path: invocation.path.clone(),
        action: invocation.action.clone(),
        resolved_args: redact_resolved_args(&invocation.resolved_args),
        ..ErrorContext::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_protocol_request, resolve_mapping, ActionKind, MappingRequest, ProtocolRequest,
        RestMethod,
    };
    use crate::args::ParsedInvocation;
    use crate::error::ErrorCode;
    use std::collections::BTreeMap;

    #[test]
    fn new_keeps_all_tokens() {
        let request = MappingRequest::new(vec!["ip".into(), "address".into(), "print".into()]);
        assert_eq!(request.tokens, vec!["ip", "address", "print"]);
    }

    #[test]
    fn maps_ip_address_print_to_classic_api_path() {
        let invocation = invocation(&["ip", "address"], "print", &[]);

        let mapping = resolve_mapping(&invocation).expect("mapping should resolve");

        assert_eq!(mapping.action_kind, ActionKind::Print);
        assert_eq!(mapping.action_kind.as_str(), "print");
        assert_eq!(mapping.routeros_path, "/ip/address/print");
        assert!(mapping.side_effects.is_empty());
        assert_eq!(mapping.idempotency, "read-only");
        assert!(mapping.has_rest_mapping());
        assert_eq!(
            mapping
                .rest_mapping
                .as_ref()
                .map(|rest| rest.method.as_str()),
            Some("GET"),
        );
    }

    #[test]
    fn builds_stable_protocol_request_for_ip_address_add() {
        let invocation = invocation(
            &["ip", "address"],
            "add",
            &[("interface", "ether1"), ("address", "192.168.88.2/24")],
        );

        let request = build_protocol_request(&invocation).expect("request should build");

        assert_eq!(request.mapping.action_kind, ActionKind::Add);
        assert_eq!(request.mapping.routeros_path, "/ip/address/add");
        assert_eq!(
            request.mapping.side_effects,
            vec!["creates-routeros-record".to_owned()],
        );
        assert_eq!(request.mapping.idempotency, "not-idempotent");
        assert_eq!(
            request
                .mapping
                .rest_mapping
                .as_ref()
                .map(|rest| rest.method),
            Some(RestMethod::Put),
        );
        assert_eq!(
            request.classic_api_words(),
            vec![
                "/ip/address/add".to_owned(),
                "=address=192.168.88.2/24".to_owned(),
                "=interface=ether1".to_owned(),
            ],
        );
    }

    #[test]
    fn write_requests_validate_required_arguments_before_network() {
        let missing_interface = build_protocol_request(&invocation(
            &["ip", "address"],
            "add",
            &[("address", "192.168.88.2/24")],
        ))
        .expect_err("add should require interface");
        assert_eq!(missing_interface.error_code, ErrorCode::UsageError);
        assert_eq!(missing_interface.context.command, "ip/address/add");

        let missing_id = build_protocol_request(&invocation(&["ip", "address"], "remove", &[]))
            .expect_err("remove should require .id");
        assert_eq!(missing_id.error_code, ErrorCode::UsageError);
        assert_eq!(missing_id.context.command, "ip/address/remove");
    }

    #[test]
    fn maps_ip_address_set_and_remove_side_effects() {
        let set = resolve_mapping(&invocation(&["ip", "address"], "set", &[]))
            .expect("set mapping should resolve");
        assert_eq!(set.action_kind, ActionKind::Set);
        assert_eq!(set.routeros_path, "/ip/address/set");
        assert_eq!(set.side_effects, vec!["updates-routeros-record".to_owned()]);
        assert_eq!(set.idempotency, "idempotent");

        let remove = resolve_mapping(&invocation(&["ip", "address"], "remove", &[]))
            .expect("remove mapping should resolve");
        assert_eq!(remove.action_kind, ActionKind::Remove);
        assert_eq!(remove.routeros_path, "/ip/address/remove");
        assert_eq!(
            remove.side_effects,
            vec!["deletes-routeros-record".to_owned()],
        );
        assert_eq!(remove.idempotency, "not-idempotent");
    }

    #[test]
    fn maps_ip_route_print_as_read_only_with_rest_support() {
        let route = resolve_mapping(&invocation(&["ip", "route"], "print", &[]))
            .expect("ip route print should resolve");

        assert_eq!(route.action_kind, ActionKind::Print);
        assert_eq!(route.routeros_path, "/ip/route/print");
        assert!(route.side_effects.is_empty());
        assert_eq!(route.idempotency, "read-only");
        assert_eq!(
            route
                .rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/ip/route")),
        );
    }

    #[test]
    fn maps_firewall_prints_as_read_only_with_rest_support() {
        for (path, classic_path, rest_path) in [
            (
                &["ip", "firewall", "address-list"][..],
                "/ip/firewall/address-list/print",
                "/rest/ip/firewall/address-list",
            ),
            (
                &["ip", "firewall", "filter"][..],
                "/ip/firewall/filter/print",
                "/rest/ip/firewall/filter",
            ),
            (
                &["ip", "firewall", "nat"][..],
                "/ip/firewall/nat/print",
                "/rest/ip/firewall/nat",
            ),
        ] {
            let mapping = resolve_mapping(&invocation(path, "print", &[]))
                .expect("firewall print should resolve");

            assert_eq!(mapping.action_kind, ActionKind::Print);
            assert_eq!(mapping.routeros_path, classic_path);
            assert_eq!(mapping.idempotency, "read-only");
            assert!(mapping.side_effects.is_empty());
            assert_eq!(
                mapping
                    .rest_mapping
                    .as_ref()
                    .map(|rest| (&rest.method, rest.path.as_str())),
                Some((&RestMethod::Get, rest_path)),
            );
        }
    }

    #[test]
    fn maps_wireguard_prints_as_read_only_with_rest_support() {
        let wg = resolve_mapping(&invocation(&["interface", "wireguard"], "print", &[]))
            .expect("wireguard print should resolve");
        assert_eq!(wg.action_kind, ActionKind::Print);
        assert_eq!(wg.routeros_path, "/interface/wireguard/print");
        assert_eq!(wg.idempotency, "read-only");
        assert!(wg.side_effects.is_empty());
        assert_eq!(
            wg.rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/interface/wireguard")),
        );

        let peers = resolve_mapping(&invocation(
            &["interface", "wireguard", "peers"],
            "print",
            &[],
        ))
        .expect("wireguard peers print should resolve");
        assert_eq!(peers.routeros_path, "/interface/wireguard/peers/print");
        assert_eq!(peers.idempotency, "read-only");
        assert_eq!(
            peers
                .rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/interface/wireguard/peers")),
        );
    }

    #[test]
    fn maps_interface_and_system_resource_print() {
        let interface = resolve_mapping(&invocation(&["interface"], "print", &[]))
            .expect("interface print should resolve");
        assert_eq!(interface.routeros_path, "/interface/print");

        let resource = resolve_mapping(&invocation(&["system", "resource"], "print", &[]))
            .expect("system resource print should resolve");
        assert_eq!(resource.routeros_path, "/system/resource/print");
    }

    #[test]
    fn maps_system_package_print_as_read_only_with_rest_support() {
        let package = resolve_mapping(&invocation(&["system", "package"], "print", &[]))
            .expect("system package print should resolve");

        assert_eq!(package.action_kind, ActionKind::Print);
        assert_eq!(package.routeros_path, "/system/package/print");
        assert!(package.side_effects.is_empty());
        assert_eq!(package.idempotency, "read-only");
        assert_eq!(
            package
                .rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/system/package")),
        );
    }

    #[test]
    fn maps_system_script_add_as_write_with_required_source() {
        let request = build_protocol_request(&invocation(
            &["system", "script"],
            "add",
            &[("name", "bootstrap"), ("source", ":put hello")],
        ))
        .expect("system script add should map");

        assert_eq!(request.mapping.action_kind, ActionKind::Add);
        assert_eq!(request.mapping.routeros_path, "/system/script/add");
        assert_eq!(
            request.mapping.side_effects,
            vec!["creates-routeros-script"]
        );
        assert_eq!(request.mapping.idempotency, "not-idempotent");
        assert_eq!(
            request
                .mapping
                .rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Put, "/rest/system/script")),
        );
        assert_eq!(
            request.classic_api_words(),
            vec![
                "/system/script/add".to_owned(),
                "=name=bootstrap".to_owned(),
                "=source=:put hello".to_owned(),
            ],
        );

        let error = build_protocol_request(&invocation(
            &["system", "script"],
            "add",
            &[("source", ":put secret")],
        ))
        .expect_err("script add should require name and redact source");
        assert_eq!(error.error_code, ErrorCode::UsageError);
        assert_eq!(error.context.command, "system/script/add");
        assert_eq!(
            error
                .context
                .resolved_args
                .get("source")
                .map(String::as_str),
            Some("***REDACTED***"),
        );
    }

    #[test]
    fn maps_raw_print_passthrough_to_classic_words_only() {
        let request = build_protocol_request(&invocation(
            &["raw"],
            "/system/resource/print",
            &[("detail", "yes")],
        ))
        .expect("raw print should map");

        assert!(request.mapping.is_raw());
        assert_eq!(request.mapping.action_kind, ActionKind::Print);
        assert_eq!(request.mapping.routeros_path, "/system/resource/print");
        assert!(request.mapping.side_effects.is_empty());
        assert_eq!(request.mapping.idempotency, "read-only");
        assert!(!request.mapping.has_rest_mapping());
        assert_eq!(
            request.classic_api_words(),
            vec![
                "/system/resource/print".to_owned(),
                "=detail=yes".to_owned(),
            ],
        );
    }

    #[test]
    fn maps_print_bare_options_to_classic_attribute_words() {
        let request = build_protocol_request(&invocation_with_flags(
            &["ip", "firewall", "filter"],
            "print",
            &[],
            &["stats"],
        ))
        .expect("print with stats should map");

        assert_eq!(request.mapping.action_kind, ActionKind::Print);
        assert_eq!(request.flags, vec!["stats"]);
        assert_eq!(
            request.classic_api_words(),
            vec!["/ip/firewall/filter/print".to_owned(), "=stats=".to_owned(),],
        );
    }

    #[test]
    fn maps_raw_print_bare_options_as_read_only() {
        let request = build_protocol_request(&invocation_with_flags(
            &["raw"],
            "/ip/firewall/connection/print",
            &[],
            &["count-only"],
        ))
        .expect("raw count-only print should map");

        assert!(request.mapping.is_raw());
        assert_eq!(request.mapping.action_kind, ActionKind::Print);
        assert_eq!(request.mapping.idempotency, "read-only");
        assert_eq!(
            request.classic_api_words(),
            vec![
                "/ip/firewall/connection/print".to_owned(),
                "=count-only=".to_owned(),
            ],
        );
    }

    #[test]
    fn rejects_unsafe_or_unknown_print_options() {
        let unsafe_option = build_protocol_request(&invocation(
            &["ip", "firewall", "filter"],
            "print",
            &[("file", "firewall-export")],
        ))
        .expect_err("file print option should not be treated as read-only");
        assert_eq!(unsafe_option.error_code, ErrorCode::UsageError);

        let unknown_flag = build_protocol_request(&invocation_with_flags(
            &["ip", "firewall", "filter"],
            "print",
            &[],
            &["brief"],
        ))
        .expect_err("unknown bare flag should fail");
        assert_eq!(unknown_flag.error_code, ErrorCode::UsageError);
    }

    #[test]
    fn maps_dhcp_client_and_firewall_connection_prints() {
        let dhcp = resolve_mapping(&invocation(&["ip", "dhcp-client"], "print", &[]))
            .expect("dhcp client print should resolve");
        assert_eq!(dhcp.routeros_path, "/ip/dhcp-client/print");
        assert_eq!(
            dhcp.rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/ip/dhcp-client")),
        );

        let connection =
            resolve_mapping(&invocation(&["ip", "firewall", "connection"], "print", &[]))
                .expect("firewall connection print should resolve");
        assert_eq!(connection.routeros_path, "/ip/firewall/connection/print");
        assert_eq!(
            connection
                .rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/ip/firewall/connection")),
        );
    }

    #[test]
    fn maps_raw_write_passthrough_with_unknown_idempotency() {
        let request = build_protocol_request(&invocation(
            &["raw"],
            "/tool/fetch",
            &[("url", "https://example.invalid/a.rsc")],
        ))
        .expect("raw write should map");

        assert!(request.mapping.is_raw());
        assert_eq!(request.mapping.action_kind, ActionKind::Raw);
        assert_eq!(request.mapping.routeros_path, "/tool/fetch");
        assert_eq!(request.mapping.side_effects, vec!["raw-routeros-command"]);
        assert_eq!(request.mapping.idempotency, "unknown");
        assert!(!request.mapping.has_rest_mapping());
    }

    #[test]
    fn raw_passthrough_requires_absolute_routeros_path() {
        let error = build_protocol_request(&invocation(&["raw"], "system/resource/print", &[]))
            .expect_err("raw path should require slash");

        assert_eq!(error.error_code, ErrorCode::UsageError);
        assert!(error.message.contains("starting with `/`"));
    }

    #[test]
    fn maps_tool_prints_as_read_only_with_rest_support() {
        for (path, classic_path, rest_path) in [
            (
                &["tool", "mac-server"][..],
                "/tool/mac-server/print",
                "/rest/tool/mac-server",
            ),
            (
                &["tool", "netwatch"][..],
                "/tool/netwatch/print",
                "/rest/tool/netwatch",
            ),
        ] {
            let mapping = resolve_mapping(&invocation(path, "print", &[]))
                .expect("tool print should resolve");

            assert_eq!(mapping.action_kind, ActionKind::Print);
            assert_eq!(mapping.routeros_path, classic_path);
            assert_eq!(mapping.idempotency, "read-only");
            assert!(mapping.side_effects.is_empty());
            assert_eq!(
                mapping
                    .rest_mapping
                    .as_ref()
                    .map(|rest| (&rest.method, rest.path.as_str())),
                Some((&RestMethod::Get, rest_path)),
            );
        }
    }

    #[test]
    fn maps_user_print_as_read_only_with_rest_support() {
        let user = resolve_mapping(&invocation(&["user"], "print", &[]))
            .expect("user print should resolve");

        assert_eq!(user.action_kind, ActionKind::Print);
        assert_eq!(user.routeros_path, "/user/print");
        assert!(user.side_effects.is_empty());
        assert_eq!(user.idempotency, "read-only");
        assert_eq!(
            user.rest_mapping
                .as_ref()
                .map(|rest| (&rest.method, rest.path.as_str())),
            Some((&RestMethod::Get, "/rest/user")),
        );
    }

    #[test]
    fn unknown_mapping_returns_unsupported_action_with_redacted_args() {
        let invocation = invocation(
            &["ip", "address"],
            "enable",
            &[("password", "super-secret")],
        );

        let error = resolve_mapping(&invocation).expect_err("unknown action should fail");

        assert_eq!(error.error_code, ErrorCode::UnsupportedAction);
        assert_eq!(error.context.command, "ip/address/enable");
        assert_eq!(error.context.path, vec!["ip", "address"]);
        assert_eq!(error.context.action, "enable");
        assert_eq!(
            error
                .context
                .resolved_args
                .get("password")
                .map(String::as_str),
            Some("***REDACTED***"),
        );
    }

    fn invocation(path: &[&str], action: &str, args: &[(&str, &str)]) -> ParsedInvocation {
        invocation_with_flags(path, action, args, &[])
    }

    #[test]
    fn debug_output_redacts_sensitive_resolved_args() {
        let mapping =
            resolve_mapping(&invocation(&["ip", "address"], "print", &[])).expect("mapping");
        let secret = "SuperSecret123";
        let request = ProtocolRequest {
            mapping,
            resolved_args: BTreeMap::from([
                ("password".to_owned(), secret.to_owned()),
                ("source".to_owned(), secret.to_owned()),
                ("address".to_owned(), "10.0.0.1".to_owned()),
            ]),
            flags: Vec::new(),
        };

        let debug = format!("{request:?}");

        assert!(
            !debug.contains(secret),
            "debug must not leak secret values: {debug}",
        );
        assert!(debug.contains("***REDACTED***"));
        assert!(debug.contains("10.0.0.1"));
    }

    fn invocation_with_flags(
        path: &[&str],
        action: &str,
        args: &[(&str, &str)],
        flags: &[&str],
    ) -> ParsedInvocation {
        ParsedInvocation {
            path: path.iter().map(|item| (*item).to_owned()).collect(),
            action: action.to_owned(),
            resolved_args: args
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            flags: flags.iter().map(|flag| (*flag).to_owned()).collect(),
        }
    }
}
