use crate::args::Cli;
use crate::error::{RosWireError, RosWireResult};
use serde::Serialize;

pub mod cache;
pub mod discovery;
pub mod doctor;

#[derive(Debug, Clone, Serialize)]
pub struct ArgumentSpec {
    pub name: String,
    pub style: String,
    pub required: bool,
    #[serde(rename = "type")]
    pub arg_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandDefinition {
    pub name: String,
    pub summary: String,
    pub kind: String,
    pub syntax: String,
    pub arguments: Vec<ArgumentSpec>,
    pub examples: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CommandsPayload {
    schema_version: &'static str,
    commands: Vec<CommandSummary>,
}

#[derive(Debug, Serialize)]
struct CommandSummary {
    name: String,
    summary: String,
    kind: String,
}

#[derive(Debug, Serialize)]
struct HelpPayload {
    schema_version: &'static str,
    command: CommandDefinition,
}

#[derive(Debug, Serialize)]
struct HelpIndexPayload {
    schema_version: &'static str,
    global_options: Vec<String>,
    commands: Vec<CommandSummary>,
}

#[derive(Debug, Serialize)]
struct SchemaPayload {
    schema_version: &'static str,
    command: String,
    arguments: Vec<ArgumentSpec>,
}

#[derive(Debug, Serialize)]
struct ExplainErrorPayload {
    schema_version: &'static str,
    error_code: String,
    summary: String,
    common_causes: Vec<String>,
    suggested_next_steps: Vec<String>,
}

pub fn handle(tokens: &[String], cli: &Cli) -> Option<RosWireResult<String>> {
    if tokens.is_empty() {
        return None;
    }

    let command = tokens[0].as_str();

    if cli.remote && command == "commands" {
        return Some(Err(Box::new(RosWireError::remote_schema_unavailable())));
    }

    match command {
        "commands" => Some(render_json(&commands_payload())),
        "doctor" => Some(doctor::doctor_payload(cli)),
        "help" => Some(help_payload(tokens)),
        "schema" if cli.remote => Some(remote_schema_payload(tokens, cli)),
        "schema" => Some(schema_payload(tokens)),
        "explain-error" => Some(explain_error_payload(tokens)),
        _ => None,
    }
}

fn commands_payload() -> CommandsPayload {
    let commands = catalog()
        .into_iter()
        .map(|entry| CommandSummary {
            name: entry.name,
            summary: entry.summary,
            kind: entry.kind,
        })
        .collect();

    CommandsPayload {
        schema_version: "roswire.commands.v1",
        commands,
    }
}

fn help_payload(tokens: &[String]) -> RosWireResult<String> {
    if tokens.len() == 1 {
        let payload = HelpIndexPayload {
            schema_version: "roswire.help.index.v1",
            global_options: vec![
                "--profile".to_owned(),
                "--host".to_owned(),
                "--user".to_owned(),
                "--password".to_owned(),
                "--protocol".to_owned(),
                "--routeros-version".to_owned(),
                "--transfer".to_owned(),
                "--port".to_owned(),
                "--json".to_owned(),
                "--debug".to_owned(),
                "--include-remote".to_owned(),
                "--refresh".to_owned(),
                "--source".to_owned(),
                "--allow-write".to_owned(),
            ],
            commands: commands_payload().commands,
        };
        return render_json(&payload);
    }

    let topic = normalize_topic(&tokens[1..]);
    let command = lookup_command(&topic)
        .ok_or_else(|| Box::new(RosWireError::help_topic_not_found(topic.clone())))?;

    render_json(&HelpPayload {
        schema_version: "roswire.command.help.v1",
        command,
    })
}

fn schema_payload(tokens: &[String]) -> RosWireResult<String> {
    if tokens.get(1).map(String::as_str) == Some("discover") {
        return Err(Box::new(RosWireError::usage(
            "schema discover requires --remote: roswire schema discover --remote --json",
        )));
    }

    if tokens.len() < 3 || tokens[1].as_str() != "command" {
        return Err(Box::new(RosWireError::usage(
            "schema command requires: roswire schema command <command...>",
        )));
    }

    let topic = normalize_topic(&tokens[2..]);
    let command = lookup_command(&topic)
        .ok_or_else(|| Box::new(RosWireError::schema_unavailable(topic.clone())))?;

    render_json(&SchemaPayload {
        schema_version: "roswire.command.schema.v1",
        command: command.name,
        arguments: command.arguments,
    })
}

fn remote_schema_payload(tokens: &[String], cli: &Cli) -> RosWireResult<String> {
    let commands = catalog();
    let policies = match tokens.get(1).map(String::as_str) {
        Some("discover") => discovery::policies_from_catalog(&commands),
        Some("command") if tokens.len() >= 3 => {
            let topic = normalize_topic(&tokens[2..]);
            let command = lookup_command(&topic)
                .ok_or_else(|| Box::new(RosWireError::schema_unavailable(topic.clone())))?;
            discovery::policy_from_command(&command)
                .map(|policy| vec![policy])
                .ok_or_else(|| Box::new(RosWireError::schema_unavailable(topic)))?
        }
        _ => {
            return Err(Box::new(RosWireError::usage(
                "remote schema requires: roswire schema discover --remote --json or roswire schema command <command...> --remote --json",
            )));
        }
    };

    let (profile, fingerprint, warning) = match crate::resolve_execution_target(cli) {
        Ok(target) => (
            cli.profile.clone().unwrap_or_else(|| "default".to_owned()),
            discovery::unknown_fingerprint(&target.host, &target.requested_protocol),
            "CAPABILITY_PROBE_FAILED".to_owned(),
        ),
        Err(error) => (
            cli.profile.clone().unwrap_or_else(|| "default".to_owned()),
            discovery::unknown_fingerprint("unknown", "unknown"),
            discovery::warning_name(error.error_code),
        ),
    };

    let cache_status = if cli.refresh {
        cache::CacheLookupStatus::Refresh
    } else {
        cache::CacheLookupStatus::Miss
    };
    let snapshot = discovery::degraded_remote_schema_snapshot_with_cache_status(
        &profile,
        &fingerprint,
        policies,
        warning,
        cache_status,
    );
    render_json(&snapshot)
}

fn explain_error_payload(tokens: &[String]) -> RosWireResult<String> {
    if tokens.len() < 2 {
        return Err(Box::new(RosWireError::usage(
            "missing error code: roswire explain-error <CODE>",
        )));
    }

    let code = tokens[1].to_ascii_uppercase();
    let payload = match code.as_str() {
        "ROS_API_FAILURE" => ExplainErrorPayload {
            schema_version: "roswire.error.explain.v1",
            error_code: code,
            summary: "RouterOS returned a trap or command failure.".to_owned(),
            common_causes: vec![
                "target interface/item does not exist".to_owned(),
                "argument value does not match menu expectations".to_owned(),
            ],
            suggested_next_steps: vec![
                "run `roswire interface print --json` to discover valid interfaces".to_owned(),
                "verify key=value arguments and .id references".to_owned(),
            ],
        },
        "USAGE_ERROR" => ExplainErrorPayload {
            schema_version: "roswire.error.explain.v1",
            error_code: code,
            summary: "CLI arguments are missing or invalid.".to_owned(),
            common_causes: vec![
                "missing action token".to_owned(),
                "malformed key=value argument".to_owned(),
            ],
            suggested_next_steps: vec![
                "run `roswire help --json` to inspect command contracts".to_owned(),
                "run `roswire commands --json` for command discovery".to_owned(),
            ],
        },
        "SSH_RESTORE_FAILED" => ExplainErrorPayload {
            schema_version: "roswire.error.explain.v1",
            error_code: code,
            summary:
                "RosWire could not restore the RouterOS SSH service state captured before transfer."
                    .to_owned(),
            common_causes: vec![
                "RouterOS rejected /ip service ssh restore fields".to_owned(),
                "control protocol connection failed during cleanup".to_owned(),
            ],
            suggested_next_steps: vec![
                "inspect `/ip service print where name=ssh` on the router".to_owned(),
                "restore the intended disabled/address values manually before retrying".to_owned(),
            ],
        },
        _ => {
            return Err(Box::new(RosWireError::help_topic_not_found(format!(
                "error code {code}",
            ))));
        }
    };

    render_json(&payload)
}

fn render_json<T: Serialize>(value: &T) -> RosWireResult<String> {
    serde_json::to_string(value).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize introspection payload: {error}",
        )))
    })
}

fn normalize_topic(tokens: &[String]) -> String {
    tokens
        .iter()
        .flat_map(|token| token.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn lookup_command(topic: &str) -> Option<CommandDefinition> {
    catalog()
        .into_iter()
        .find(|command| command.name.eq_ignore_ascii_case(topic))
}

fn catalog() -> Vec<CommandDefinition> {
    vec![
        CommandDefinition {
            name: "ip address print".to_owned(),
            summary: "Print IP address records.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip address print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire ip address print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip address add".to_owned(),
            summary: "Add an IP address entry.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip address add address=<cidr> interface=<name> --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: "address".to_owned(),
                    style: "key-value".to_owned(),
                    required: true,
                    arg_type: "cidr".to_owned(),
                    description: "Address with prefix length.".to_owned(),
                    example: Some("192.168.88.2/24".to_owned()),
                },
                ArgumentSpec {
                    name: "interface".to_owned(),
                    style: "key-value".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "RouterOS interface name.".to_owned(),
                    example: Some("bridge".to_owned()),
                },
            ],
            examples: vec![
                "roswire ip address add address=192.168.88.2/24 interface=bridge --json".to_owned(),
            ],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip address set".to_owned(),
            summary: "Update an existing IP address entry.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip address set .id=<id> disabled=<bool> --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: ".id".to_owned(),
                    style: "key-value".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "RouterOS internal item ID.".to_owned(),
                    example: Some("*1".to_owned()),
                },
                ArgumentSpec {
                    name: "disabled".to_owned(),
                    style: "key-value".to_owned(),
                    required: false,
                    arg_type: "bool".to_owned(),
                    description: "Disable or enable the address entry.".to_owned(),
                    example: Some("true".to_owned()),
                },
            ],
            examples: vec!["roswire ip address set .id=*1 disabled=true --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip address remove".to_owned(),
            summary: "Remove an IP address entry.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip address remove .id=<id> --json".to_owned(),
            arguments: vec![ArgumentSpec {
                name: ".id".to_owned(),
                style: "key-value".to_owned(),
                required: true,
                arg_type: "string".to_owned(),
                description: "RouterOS internal item ID.".to_owned(),
                example: Some("*1".to_owned()),
            }],
            examples: vec!["roswire ip address remove .id=*1 --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip firewall address-list print".to_owned(),
            summary: "Print firewall address-list entries.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip firewall address-list print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire ip firewall address-list print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip firewall filter print".to_owned(),
            summary: "Print firewall filter rules without changing packet handling.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip firewall filter print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire ip firewall filter print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip firewall nat print".to_owned(),
            summary: "Print firewall NAT rules without changing packet handling.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip firewall nat print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire ip firewall nat print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "ip route print".to_owned(),
            summary: "Print RouterOS IP routes, including v6/v7 route table fields when present."
                .to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire ip route print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire ip route print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "interface print".to_owned(),
            summary: "Print interface list.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire interface print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire interface print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "interface wireguard print".to_owned(),
            summary: "Print RouterOS v7 WireGuard interfaces without exposing private keys."
                .to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire interface wireguard print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire interface wireguard print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "interface wireguard peers print".to_owned(),
            summary: "Print RouterOS v7 WireGuard peers without exposing preshared keys."
                .to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire interface wireguard peers print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire interface wireguard peers print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "system package print".to_owned(),
            summary: "Print installed RouterOS packages.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire system package print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire system package print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "script put".to_owned(),
            summary: "Read local .rsc text and store it as a RouterOS system script without creating a RouterOS file.".to_owned(),
            kind: "workflow".to_owned(),
            syntax: "roswire script put <name> --source @<local.rsc> --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: "name".to_owned(),
                    style: "positional".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "RouterOS system script name to create.".to_owned(),
                    example: Some("bootstrap".to_owned()),
                },
                ArgumentSpec {
                    name: "--source".to_owned(),
                    style: "option".to_owned(),
                    required: true,
                    arg_type: "@path".to_owned(),
                    description: "Local UTF-8 .rsc file to read as script source. The content is never printed in errors or dry-run output.".to_owned(),
                    example: Some("@setup.rsc".to_owned()),
                },
            ],
            examples: vec![
                "roswire script put bootstrap --source @setup.rsc --dry-run --json".to_owned(),
                "roswire --profile lab script put bootstrap --source @setup.rsc --json".to_owned(),
            ],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "FILE_TOO_LARGE".to_owned(),
                "CONFIG_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "raw".to_owned(),
            summary: "Explicitly pass a RouterOS classic API path and key=value words for advanced unsupported commands.".to_owned(),
            kind: "raw-routeros-command".to_owned(),
            syntax: "roswire raw /system/resource/print [key=value ...] --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: "routeros-path".to_owned(),
                    style: "positional".to_owned(),
                    required: true,
                    arg_type: "classic-api-path".to_owned(),
                    description: "RouterOS classic API path beginning with `/`, e.g. /system/resource/print.".to_owned(),
                    example: Some("/system/resource/print".to_owned()),
                },
                ArgumentSpec {
                    name: "key=value".to_owned(),
                    style: "key-value".to_owned(),
                    required: false,
                    arg_type: "string".to_owned(),
                    description: "Additional RouterOS API word arguments. Sensitive keys and local paths are redacted in errors/logs.".to_owned(),
                    example: Some("detail=yes".to_owned()),
                },
                ArgumentSpec {
                    name: "--allow-write".to_owned(),
                    style: "flag".to_owned(),
                    required: false,
                    arg_type: "bool".to_owned(),
                    description: "Required for raw commands that are not `/.../print`; REST generic raw passthrough is intentionally unavailable.".to_owned(),
                    example: Some("--allow-write".to_owned()),
                },
            ],
            examples: vec![
                "roswire raw /system/resource/print --json".to_owned(),
                "roswire raw /tool/fetch url=https://example.invalid/a.rsc --allow-write --json".to_owned(),
            ],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "CONFIG_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
                "UNSUPPORTED_ACTION".to_owned(),
            ],
        },
        CommandDefinition {
            name: "tool mac-server print".to_owned(),
            summary: "Print RouterOS MAC server settings without changing service state."
                .to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire tool mac-server print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire tool mac-server print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "tool netwatch print".to_owned(),
            summary: "Print RouterOS Netwatch entries; does not run ad-hoc probes.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire tool netwatch print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire tool netwatch print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "user print".to_owned(),
            summary: "Print RouterOS users without exposing password material.".to_owned(),
            kind: "routeros-command".to_owned(),
            syntax: "roswire user print --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire user print --json".to_owned()],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "config inspect".to_owned(),
            summary: "Inspect resolved local configuration and source precedence.".to_owned(),
            kind: "config".to_owned(),
            syntax: "roswire config inspect --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire config inspect --json".to_owned()],
            errors: vec![
                "CONFIG_ERROR".to_owned(),
                "PROFILE_NOT_FOUND".to_owned(),
                "CONFIG_INSECURE_PERMISSIONS".to_owned(),
            ],
        },
        CommandDefinition {
            name: "config init".to_owned(),
            summary: "Initialize ~/.roswire home, logs, and config.toml".to_owned(),
            kind: "config".to_owned(),
            syntax: "roswire config init --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire config init --json".to_owned()],
            errors: vec!["CONFIG_ERROR".to_owned(), "INTERNAL_ERROR".to_owned()],
        },
        CommandDefinition {
            name: "config profiles".to_owned(),
            summary: "List configured profiles and default profile.".to_owned(),
            kind: "config".to_owned(),
            syntax: "roswire config profiles --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire config profiles --json".to_owned()],
            errors: vec!["CONFIG_ERROR".to_owned()],
        },
        CommandDefinition {
            name: "config device add".to_owned(),
            summary: "Create a profile/device entry in config.toml.".to_owned(),
            kind: "config".to_owned(),
            syntax: "roswire config device add <profile> host=<host> user=<user> [protocol=<mode>] [transfer=<mode>] [port=<n>] --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: "profile".to_owned(),
                    style: "positional".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "Profile name to create.".to_owned(),
                    example: Some("studio-router".to_owned()),
                },
                ArgumentSpec {
                    name: "host".to_owned(),
                    style: "key-value".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "Router host/IP".to_owned(),
                    example: Some("10.189.189.1".to_owned()),
                },
                ArgumentSpec {
                    name: "user".to_owned(),
                    style: "key-value".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "Router username".to_owned(),
                    example: Some("master".to_owned()),
                },
            ],
            examples: vec![
                "roswire config device add studio-router host=10.189.189.1 user=master --json".to_owned(),
            ],
            errors: vec!["USAGE_ERROR".to_owned(), "CONFIG_ERROR".to_owned()],
        },
        CommandDefinition {
            name: "config device set".to_owned(),
            summary: "Update an existing profile/device entry in config.toml.".to_owned(),
            kind: "config".to_owned(),
            syntax: "roswire config device set <profile> [host=<host>] [user=<user>] [protocol=<mode>] [transfer=<mode>] [port=<n>] --json".to_owned(),
            arguments: vec![ArgumentSpec {
                name: "profile".to_owned(),
                style: "positional".to_owned(),
                required: true,
                arg_type: "string".to_owned(),
                description: "Profile name to update.".to_owned(),
                example: Some("studio-router".to_owned()),
            }],
            examples: vec![
                "roswire config device set studio-router protocol=rest --json".to_owned(),
            ],
            errors: vec!["USAGE_ERROR".to_owned(), "CONFIG_ERROR".to_owned()],
        },
        CommandDefinition {
            name: "config secret set".to_owned(),
            summary: "Set profile secret using plain/encrypted/keychain/same-as types.".to_owned(),
            kind: "config".to_owned(),
            syntax: "roswire config secret set <profile> <name> type=<plain|encrypted|keychain|same-as> ... --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: "profile".to_owned(),
                    style: "positional".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "Profile name.".to_owned(),
                    example: Some("studio-router".to_owned()),
                },
                ArgumentSpec {
                    name: "name".to_owned(),
                    style: "positional".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "Secret key name, e.g. password.".to_owned(),
                    example: Some("password".to_owned()),
                },
                ArgumentSpec {
                    name: "type".to_owned(),
                    style: "key-value".to_owned(),
                    required: true,
                    arg_type: "string".to_owned(),
                    description: "Secret backend type.".to_owned(),
                    example: Some("keychain".to_owned()),
                },
            ],
            examples: vec![
                "roswire config secret set studio-router password type=plain value=All.007! --json".to_owned(),
                "roswire config secret set studio-router password type=keychain service=roswire account=profiles/studio-router/password --json".to_owned(),
                "roswire config secret set studio-router ssh_password type=same-as target=password --json".to_owned(),
            ],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "PROFILE_NOT_FOUND".to_owned(),
                "CONFIG_ERROR".to_owned(),
            ],
        },
        CommandDefinition {
            name: "doctor".to_owned(),
            summary: "Run local diagnostics and optionally include read-only remote checks."
                .to_owned(),
            kind: "introspection".to_owned(),
            syntax: "roswire doctor [--include-remote] --json".to_owned(),
            arguments: vec![ArgumentSpec {
                name: "--include-remote".to_owned(),
                style: "flag".to_owned(),
                required: false,
                arg_type: "bool".to_owned(),
                description: "Include read-only RouterOS port/login/resource diagnostics."
                    .to_owned(),
                example: Some("--include-remote".to_owned()),
            }],
            examples: vec![
                "roswire doctor --json".to_owned(),
                "roswire --profile studio-router doctor --include-remote --json".to_owned(),
            ],
            errors: vec![
                "CONFIG_ERROR".to_owned(),
                "AUTH_FAILED".to_owned(),
                "NETWORK_ERROR".to_owned(),
                "ROS_API_FAILURE".to_owned(),
            ],
        },
        CommandDefinition {
            name: "commands".to_owned(),
            summary: "List command index for agent discovery.".to_owned(),
            kind: "introspection".to_owned(),
            syntax: "roswire commands --json".to_owned(),
            arguments: vec![],
            examples: vec!["roswire commands --json".to_owned()],
            errors: vec!["USAGE_ERROR".to_owned()],
        },
        CommandDefinition {
            name: "schema command".to_owned(),
            summary: "Show argument schema for a command topic.".to_owned(),
            kind: "introspection".to_owned(),
            syntax: "roswire schema command ip address add --json".to_owned(),
            arguments: vec![ArgumentSpec {
                name: "command".to_owned(),
                style: "positional".to_owned(),
                required: true,
                arg_type: "string".to_owned(),
                description: "Command topic, e.g. `ip address add`.".to_owned(),
                example: Some("ip address add".to_owned()),
            }],
            examples: vec!["roswire schema command ip address add --json".to_owned()],
            errors: vec!["SCHEMA_UNAVAILABLE".to_owned(), "USAGE_ERROR".to_owned()],
        },
        CommandDefinition {
            name: "schema discover".to_owned(),
            summary: "Discover remote schema overlay with cache TTL metadata.".to_owned(),
            kind: "introspection".to_owned(),
            syntax: "roswire schema discover --remote [--refresh] --json".to_owned(),
            arguments: vec![
                ArgumentSpec {
                    name: "--remote".to_owned(),
                    style: "flag".to_owned(),
                    required: true,
                    arg_type: "bool".to_owned(),
                    description: "Enable remote schema discovery.".to_owned(),
                    example: Some("--remote".to_owned()),
                },
                ArgumentSpec {
                    name: "--refresh".to_owned(),
                    style: "flag".to_owned(),
                    required: false,
                    arg_type: "bool".to_owned(),
                    description: "Bypass cached remote schema metadata and mark the cache status as refresh.".to_owned(),
                    example: Some("--refresh".to_owned()),
                },
            ],
            examples: vec![
                "roswire schema discover --remote --json".to_owned(),
                "roswire schema discover --remote --refresh --json".to_owned(),
            ],
            errors: vec![
                "USAGE_ERROR".to_owned(),
                "CONFIG_ERROR".to_owned(),
                "REMOTE_SCHEMA_UNAVAILABLE".to_owned(),
            ],
        },
    ]
}
