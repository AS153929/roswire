# Routing and Firewall Audit

Use this playbook for a read-only review of RouterOS routing, firewall address lists, filter rules, and NAT rules. It is intended for agent summaries, drift checks, and risk triage without changing the device.

## Safety boundary

- Read-only commands only.
- Do not enable, disable, add, remove, or reorder firewall/routing rules.
- Do not use `raw` write paths or `--allow-write`.
- When recommending changes, present them as suggestions and ask for explicit approval before any future write operation.

## Commands

Routing table:

```bash
roswire --json --profile <profile> ip route print
```

Firewall address lists, filters, and NAT:

```bash
roswire --json --profile <profile> ip firewall address-list print
roswire --json --profile <profile> ip firewall filter print
roswire --json --profile <profile> ip firewall nat print
```

If command coverage is missing for a read-only RouterOS path, inspect the schema first, then use safe raw `print` only:

```bash
roswire --json schema command raw
roswire --json --profile <profile> raw /ip/firewall/filter/print detail=yes
```

## Routing evidence

Look for:

- Missing default route or multiple conflicting default routes.
- `disabled`, inactive, or unreachable gateways.
- Unexpected `distance`, `scope`, or `target-scope` values.
- RouterOS v7 routing table names and policy-routing hints.
- Static routes pointing at wrong interfaces or stale gateways.

## Firewall evidence

Look for:

- Very broad source/destination prefixes in allow rules.
- Disabled rules that appear important for baseline protection.
- Rules that accept WAN-to-LAN traffic without a clear address-list or connection-state guard.
- Missing established/related accept rules, invalid drop rules, or input-chain protection.
- Address lists containing stale, overly broad, or unexplained entries.

## NAT evidence

Look for:

- Masquerade rules bound to the expected WAN interface/list.
- Destination NAT rules that expose management services or broad internal ranges.
- Disabled NAT rules that explain broken connectivity.
- Rule ordering issues suggested by comments, counters, or chain/action combinations.

## Suggested response shape

1. Routing summary: default route, active gateway, unusual routes.
2. Firewall summary: obvious allow/drop posture, address-list posture, suspicious broad rules.
3. NAT summary: outbound NAT health and inbound exposure.
4. Risks and uncertainties, grouped by severity.
5. Safe next read-only checks; do not provide write commands unless the user asks for a change plan.
