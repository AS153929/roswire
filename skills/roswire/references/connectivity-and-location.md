# Connectivity and Network Location

Use this playbook when the user asks whether a RouterOS device is reachable, where it sits in the network, whether it has internet/WAN connectivity, or what evidence explains an outage. Keep the workflow read-only.

## Safety boundary

- Use configured profiles; do not ask the user to paste credentials.
- Start with local diagnostics before touching a device.
- Add remote checks only when a profile is available and the user is asking about a real device.
- Do not use `--allow-write` in this scenario.
- Summarize evidence; do not dump secrets, passwords, tokens, or private key material.

## Baseline commands

Run local diagnostics first:

```bash
roswire --json doctor
```

When remote reachability is in scope, run:

```bash
roswire --json --profile <profile> doctor --include-remote
```

Collect network placement evidence:

```bash
roswire --json --profile <profile> ip address print
roswire --json --profile <profile> interface print
roswire --json --profile <profile> ip route print
roswire --json --profile <profile> tool netwatch print
```

Optional raw read-only evidence for WAN/DDNS/DNS/identity/resource context:

```bash
roswire --json --profile <profile> raw /ip/cloud/print
roswire --json --profile <profile> raw /ip/dns/print
roswire --json --profile <profile> raw /system/identity/print
roswire --json --profile <profile> raw /system/resource/print
```

## What to look for

- `doctor --include-remote`: `selected_protocol`, auth/network errors, warnings, RouterOS version, local config status.
- `ip address print`: address/interface pairs, disabled addresses, unexpected subnets, missing LAN/WAN addresses.
- `interface print`: running/link status, disabled ports, suspicious interface names, bridge/VLAN clues.
- `ip route print`: default route presence, active/inactive state, gateways, distances, RouterOS v7 routing tables.
- `tool netwatch print`: configured monitors and whether their current state supports the reported outage.
- `/ip/cloud/print`: public address/DDNS fields when RouterOS Cloud is enabled.
- `/ip/dns/print`: upstream DNS configuration and `allow-remote-requests` when relevant.

## Suggested response shape

1. State whether the device was reachable and which protocol was selected.
2. Summarize local configuration health and any warnings.
3. Describe the likely network location from addresses, interfaces, and routes.
4. Call out internet/WAN evidence: default route, WAN interface/address, DDNS/public IP, DNS, Netwatch.
5. List uncertainties and the next safest read-only command if more evidence is needed.

## Failure handling

If a command fails, report the structured `error_code`, `selected_protocol` if present, and whether the failure happened before or after remote login. Avoid retry loops that could lock accounts or flood the device.
