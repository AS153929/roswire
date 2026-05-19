# WireGuard and Users Audit

Use this playbook for read-only inspection of WireGuard interfaces, peers, local users, and installed packages. It is designed for security posture summaries without exposing key material or changing the device.

## Safety boundary

- Read-only commands only.
- Never print or repeat private keys, preshared keys, passwords, or password hashes.
- If sensitive fields appear in output, summarize presence/absence and redact values.
- Do not create users, rotate keys, disable peers, or update packages unless the user later asks for an explicit change plan.

## Commands

WireGuard interfaces and peers:

```bash
roswire --json --profile <profile> interface wireguard print
roswire --json --profile <profile> interface wireguard peers print
```

Users and packages:

```bash
roswire --json --profile <profile> user print
roswire --json --profile <profile> system package print
```

Optional resource/identity context:

```bash
roswire --json --profile <profile> raw /system/resource/print
roswire --json --profile <profile> raw /system/identity/print
```

## WireGuard evidence

Look for:

- Disabled interfaces or peers that explain tunnels being down.
- Listen ports, interface names, MTU settings, and comments.
- Peer `allowed-address` ranges that are too broad or overlap unexpectedly.
- `endpoint-address`, `endpoint-port`, and persistent keepalive posture.
- Last-handshake or traffic counters when present in the RouterOS output.

When summarizing keys:

- Say whether a public key is configured when useful.
- Do not include private keys or preshared keys.
- If a preshared key field exists, summarize it as configured/not configured.

## User evidence

Look for:

- Accounts with broad groups such as `full`.
- Disabled vs enabled management users.
- Comments indicating ownership or automation accounts.
- Unexpected service accounts or shared users.

Do not request or display user passwords.

## Package evidence

Look for:

- RouterOS package versions and architecture.
- Disabled packages or packages inconsistent with the expected feature set.
- Version mismatch clues that may affect command availability.

## Suggested response shape

1. WireGuard posture: interfaces, peer count, disabled entries, handshake/endpoint clues.
2. User posture: enabled admin-like accounts and any suspicious accounts.
3. Package posture: version/architecture and notable package state.
4. Sensitive data handling note: explicitly state that key material and passwords were not reproduced.
5. Safe next read-only checks if more evidence is needed.
