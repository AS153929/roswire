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
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Print => "print",
            Self::Add => "add",
            Self::Set => "set",
            Self::Remove => "remove",
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolRequest {
    pub mapping: CommandMapping,
    pub resolved_args: BTreeMap<String, String>,
}

impl ProtocolRequest {
    pub fn classic_api_words(&self) -> Vec<String> {
        let mut words = Vec::with_capacity(1 + self.resolved_args.len());
        words.push(self.mapping.routeros_path.clone());
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

    Ok(ProtocolRequest {
        mapping,
        resolved_args: invocation.resolved_args.clone(),
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

pub fn resolve_mapping(invocation: &ParsedInvocation) -> RosWireResult<CommandMapping> {
    let path = invocation
        .path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let action = invocation.action.as_str();

    match (path.as_slice(), action) {
        (["interface"], "print") => Ok(print_mapping(
            &["interface"],
            "/interface/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/interface".to_owned(),
            }),
        )),
        (["interface", "wireguard"], "print") => Ok(print_mapping(
            &["interface", "wireguard"],
            "/interface/wireguard/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/interface/wireguard".to_owned(),
            }),
        )),
        (["interface", "wireguard", "peers"], "print") => Ok(print_mapping(
            &["interface", "wireguard", "peers"],
            "/interface/wireguard/peers/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/interface/wireguard/peers".to_owned(),
            }),
        )),
        (["ip", "address"], "print") => Ok(print_mapping(
            &["ip", "address"],
            "/ip/address/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/ip/address".to_owned(),
            }),
        )),
        (["ip", "address"], "add") => Ok(write_mapping(
            &["ip", "address"],
            ActionKind::Add,
            "/ip/address/add",
            "creates-routeros-record",
            "not-idempotent",
            Some(RestMapping {
                method: RestMethod::Put,
                path: "/rest/ip/address".to_owned(),
            }),
        )),
        (["ip", "address"], "set") => Ok(write_mapping(
            &["ip", "address"],
            ActionKind::Set,
            "/ip/address/set",
            "updates-routeros-record",
            "idempotent",
            Some(RestMapping {
                method: RestMethod::Patch,
                path: "/rest/ip/address/{.id}".to_owned(),
            }),
        )),
        (["ip", "address"], "remove") => Ok(write_mapping(
            &["ip", "address"],
            ActionKind::Remove,
            "/ip/address/remove",
            "deletes-routeros-record",
            "not-idempotent",
            Some(RestMapping {
                method: RestMethod::Delete,
                path: "/rest/ip/address/{.id}".to_owned(),
            }),
        )),
        (["ip", "firewall", "address-list"], "print") => Ok(print_mapping(
            &["ip", "firewall", "address-list"],
            "/ip/firewall/address-list/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/ip/firewall/address-list".to_owned(),
            }),
        )),
        (["ip", "firewall", "filter"], "print") => Ok(print_mapping(
            &["ip", "firewall", "filter"],
            "/ip/firewall/filter/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/ip/firewall/filter".to_owned(),
            }),
        )),
        (["ip", "firewall", "nat"], "print") => Ok(print_mapping(
            &["ip", "firewall", "nat"],
            "/ip/firewall/nat/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/ip/firewall/nat".to_owned(),
            }),
        )),
        (["ip", "route"], "print") => Ok(print_mapping(
            &["ip", "route"],
            "/ip/route/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/ip/route".to_owned(),
            }),
        )),
        (["system", "resource"], "print") => Ok(print_mapping(
            &["system", "resource"],
            "/system/resource/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/system/resource".to_owned(),
            }),
        )),
        (["system", "package"], "print") => Ok(print_mapping(
            &["system", "package"],
            "/system/package/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/system/package".to_owned(),
            }),
        )),
        (["system", "script"], "add") => Ok(write_mapping(
            &["system", "script"],
            ActionKind::Add,
            "/system/script/add",
            "creates-routeros-script",
            "not-idempotent",
            Some(RestMapping {
                method: RestMethod::Put,
                path: "/rest/system/script".to_owned(),
            }),
        )),
        (["tool", "mac-server"], "print") => Ok(print_mapping(
            &["tool", "mac-server"],
            "/tool/mac-server/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/tool/mac-server".to_owned(),
            }),
        )),
        (["tool", "netwatch"], "print") => Ok(print_mapping(
            &["tool", "netwatch"],
            "/tool/netwatch/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/tool/netwatch".to_owned(),
            }),
        )),
        (["user"], "print") => Ok(print_mapping(
            &["user"],
            "/user/print",
            Some(RestMapping {
                method: RestMethod::Get,
                path: "/rest/user".to_owned(),
            }),
        )),
        _ => Err(Box::new(
            RosWireError::unsupported_action(format!(
                "unsupported RouterOS action: {}",
                command_name(invocation),
            ))
            .with_context(mapping_error_context(invocation)),
        )),
    }
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

fn print_mapping(
    cli_path: &[&str],
    routeros_path: &str,
    rest_mapping: Option<RestMapping>,
) -> CommandMapping {
    CommandMapping {
        cli_path: cli_path.iter().map(|item| (*item).to_owned()).collect(),
        action_kind: ActionKind::Print,
        routeros_path: routeros_path.to_owned(),
        side_effects: Vec::new(),
        idempotency: "read-only".to_owned(),
        rest_mapping,
    }
}

fn write_mapping(
    cli_path: &[&str],
    action_kind: ActionKind,
    routeros_path: &str,
    side_effect: &str,
    idempotency: &str,
    rest_mapping: Option<RestMapping>,
) -> CommandMapping {
    CommandMapping {
        cli_path: cli_path.iter().map(|item| (*item).to_owned()).collect(),
        action_kind,
        routeros_path: routeros_path.to_owned(),
        side_effects: vec![side_effect.to_owned()],
        idempotency: idempotency.to_owned(),
        rest_mapping,
    }
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
    use super::{build_protocol_request, resolve_mapping, ActionKind, MappingRequest, RestMethod};
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
        ParsedInvocation {
            path: path.iter().map(|item| (*item).to_owned()).collect(),
            action: action.to_owned(),
            resolved_args: args
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        }
    }
}
