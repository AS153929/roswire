---
name: roswire
description: "Use when managing RouterOS/MikroTik devices or demonstrating agent + roswire workflows with the local `roswire` CLI: list configured devices, inspect profiles, check connectivity and network location, audit routes, firewall, WireGuard, packages, users, discover command schemas, safely run read-only raw RouterOS print commands, and collect read-only JSON evidence for agent summaries."
---

# Roswire

## Ground Rules

Use the local `roswire` command to manage RouterOS devices through configured profiles. Prefer JSON output by default so results are easy to parse, filter, and summarize:

```bash
roswire --json <command tokens...>
roswire --json --profile <profile> <command tokens...>
```

Unless a schema explicitly shows that a flag belongs to a specific command, place global options before command tokens, for example `--json`, `--profile`, `--host`, `--user`, `--protocol`, `--routeros-version`, `--dry-run`, `--remote`, and `--refresh`.

Use these safe defaults:

- Add `--json` for inspection and query tasks unless the user explicitly asks for human-readable text.
- Use `--dry-run` before planning configuration changes.
- Do not write passwords directly into shell history or final replies; prefer `--stdin` when setting secrets.
- Do not use `--allow-write` unless the user explicitly requests a write operation and the risk has been explained clearly.
- Treat `raw` as read-only by default: only run RouterOS paths that end with `/print`.
- If the command shape is unclear, first run `roswire --json commands`, then run `roswire --json schema command <topic...>`.

## Agent + roswire Demonstration Loop

This skill is designed to demonstrate how an AI agent can use `roswire` safely and transparently:

1. Discover supported commands with `roswire --json commands`.
2. Inspect exact argument contracts with `roswire --json schema command <topic...>`.
3. Run read-only profile-based commands to collect JSON evidence.
4. Summarize the evidence, including `error_code`, `selected_protocol`, warnings, and uncertainties.
5. Avoid mutation unless the user explicitly asks for a change plan; use `--dry-run` before any planned change.

Prefer concise summaries over dumping raw JSON into the conversation. Mention the commands used so the user can reproduce the evidence.

## Scenario References

Use these focused playbooks for common read-only agent workflows:

- [Connectivity and Network Location](references/connectivity-and-location.md): reachability, WAN/DDNS/DNS, local network placement, and outage triage.
- [Routing and Firewall Audit](references/routing-and-firewall-audit.md): routes, address lists, filter rules, NAT posture, and risk summaries.
- [WireGuard and Users Audit](references/wireguard-and-users-audit.md): WireGuard interfaces/peers, users, packages, and sensitive-field handling.
- [Raw Read-Only Playbook](references/raw-readonly-playbook.md): safe `raw /.../print` usage when the built-in catalog does not yet cover a query.
- [Schema Discovery](references/schema-discovery.md): the agent self-discovery loop for command catalogs, schemas, and optional remote overlays.

## Demo Scripts

The `scripts/` directory contains read-only helper scripts for demos and repeatable evidence collection:

- `scripts/roswire-readonly-snapshot.sh`: collects a broad read-only JSON snapshot for a profile.
- `scripts/roswire-connectivity-report.sh`: collects doctor, route, interface, address, Netwatch, Cloud, and DNS evidence for connectivity triage.
- `scripts/roswire-schema-tour.sh`: demonstrates local command/schema discovery and optional remote schema discovery.

Run scripts from the skill directory or by absolute path. Each script writes JSON artifacts to an output directory and avoids `set -x`, secrets, and write operations.

## Command Discovery

List the built-in roswire command catalog:

```bash
roswire --json commands
roswire help
```

Inspect the argument schema for a command topic:

```bash
roswire --json schema command config device add
roswire --json schema command ip route print
roswire --json schema command raw
```

When a real device profile is available, discover the remote schema overlay:

```bash
roswire --json --profile <profile> --remote schema discover
roswire --json --profile <profile> --remote --refresh schema discover
```

## Device Profiles

List configured devices/profiles and the default profile:

```bash
roswire --json config profiles
```

If the local roswire home/config does not exist, initialize it first:

```bash
roswire --json config init
```

Add a new device profile:

```bash
roswire --dry-run --json config device add <profile> host=<host-or-ip> user=<username> protocol=auto routeros_version=auto transfer=ssh
roswire --json config device add <profile> host=<host-or-ip> user=<username> protocol=auto routeros_version=auto transfer=ssh
```

Connection options available from the help output:

- `protocol=auto|api|api-ssl|rest`
- `routeros_version=auto|v6|v7`
- `transfer=ssh`

Set profile passwords or other secrets through stdin. Prefer `type=keychain`; use `plain` or `encrypted` only when appropriate for the environment.

```bash
read -rs ROUTER_PASSWORD
printf '%s' "$ROUTER_PASSWORD" | roswire --stdin --json config secret set <profile> password type=keychain
unset ROUTER_PASSWORD
```

Update an existing device profile:

```bash
roswire --dry-run --json config device set <profile> host=<new-host> user=<username> protocol=auto routeros_version=auto transfer=ssh
roswire --json config device set <profile> host=<new-host> user=<username> protocol=auto routeros_version=auto transfer=ssh
```

Inspect the fully resolved configuration for the default profile or a specific profile. This command shows configuration source precedence and redacts secret values:

```bash
roswire --json config inspect
roswire --json --profile <profile> config inspect
```

## Connectivity and Network Location

Run local diagnostics first. When the user asks about real device reachability, internet connectivity, or network location, add read-only remote checks:

```bash
roswire --json doctor
roswire --json --profile <profile> doctor --include-remote
```

Use these read-only commands to describe the device's network location:

```bash
roswire --json --profile <profile> ip address print
roswire --json --profile <profile> interface print
roswire --json --profile <profile> ip route print
roswire --json --profile <profile> tool netwatch print
```

For public WAN/DDNS, DNS, device identity, or resource status, use safe raw `print` commands:

```bash
roswire --json --profile <profile> raw /ip/cloud/print
roswire --json --profile <profile> raw /ip/dns/print
roswire --json --profile <profile> raw /system/identity/print
roswire --json --profile <profile> raw /system/resource/print
```

When summarizing connectivity, prefer combining this evidence:

- `doctor --include-remote`: local configuration, dependencies, selected protocol, read-only remote login/resource diagnostics, and warnings.
- `ip address print`: interface addresses and local network placement.
- `interface print`: interface state, link state, and interface names.
- `ip route print`: default routes, gateways, whether routes are active, and RouterOS v6/v7 route fields.
- `tool netwatch print`: configured reachability monitors and current status.
- `/ip/cloud/print`: RouterOS Cloud DDNS/public address fields, when enabled and supported by the device.

## Routing Table

Inspect the routing table:

```bash
roswire --json --profile <profile> ip route print
```

When summarizing, focus on default routes, active/inactive or disabled routes, gateway, distance/scope/target-scope, RouterOS v7 routing table names, and whether any suspicious default route is missing.

## Common Read-Only Device Commands

Addresses and interfaces:

```bash
roswire --json --profile <profile> ip address print
roswire --json --profile <profile> interface print
```

Firewall and NAT:

```bash
roswire --json --profile <profile> ip firewall address-list print
roswire --json --profile <profile> ip firewall filter print
roswire --json --profile <profile> ip firewall nat print
```

WireGuard:

```bash
roswire --json --profile <profile> interface wireguard print
roswire --json --profile <profile> interface wireguard peers print
```

System and users:

```bash
roswire --json --profile <profile> system package print
roswire --json --profile <profile> user print
```

Netwatch and MAC server:

```bash
roswire --json --profile <profile> tool netwatch print
roswire --json --profile <profile> tool mac-server print
```

## Raw RouterOS Passthrough

Use `raw` for advanced read-only queries that are not covered by the command catalog:

```bash
roswire --json --profile <profile> raw /system/resource/print
roswire --json --profile <profile> raw /interface/bridge/print
roswire --json --profile <profile> raw /ip/dhcp-client/print detail=yes
```

`raw` rules:

- The first argument is a classic RouterOS API path that starts with `/`.
- Additional arguments use `key=value`.
- roswire redacts sensitive keys and local paths in errors and logs.
- Non-`/print` raw commands require `--allow-write`; avoid them unless the user explicitly asks for them.

## Script Workflow

Store a local `.rsc` file as a RouterOS system script without creating a RouterOS file:

```bash
roswire --dry-run --json --profile <profile> script put <script-name> --source @setup.rsc
roswire --json --profile <profile> script put <script-name> --source @setup.rsc
```

When `--source @<path>` is available, do not paste large script contents into the conversation.

## Troubleshooting

If a profile does not exist, run:

```bash
roswire --json config profiles
```

If local configuration is missing or status looks abnormal, run:

```bash
roswire --json doctor
```

If a remote check fails, summarize `error_code`, `selected_protocol`, warnings, and whether the failure happened before or after remote login. Use `--debug` only when more diagnostics are necessary, and avoid exposing credentials or secret values from logs.
