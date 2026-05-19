# Raw Read-Only Playbook

Use this playbook when the built-in roswire command catalog does not yet cover a RouterOS read-only query. The default raw capability is strictly read-only: use RouterOS paths that end in `/print`.

## Safety boundary

- Only run raw paths that start with `/` and end with `/print` unless the user explicitly asks for a write operation.
- Do not add `--allow-write` in this playbook.
- Extra arguments must be `key=value` pairs.
- Avoid pasting large raw outputs into the final answer; summarize relevant fields.
- roswire redacts sensitive keys and local paths in structured errors, but the agent should still avoid repeating secrets.

## Discover first

Before using raw for a topic, check whether roswire has a built-in command or schema:

```bash
roswire --json commands
roswire --json schema command raw
```

If a built-in command exists, prefer it over raw.

## Safe examples

System identity and resources:

```bash
roswire --json --profile <profile> raw /system/identity/print
roswire --json --profile <profile> raw /system/resource/print
```

Bridge and DHCP client state:

```bash
roswire --json --profile <profile> raw /interface/bridge/print
roswire --json --profile <profile> raw /ip/dhcp-client/print detail=yes
```

Cloud/DDNS and DNS:

```bash
roswire --json --profile <profile> raw /ip/cloud/print
roswire --json --profile <profile> raw /ip/dns/print
```

Firewall detail when the built-in print command is not enough:

```bash
roswire --json --profile <profile> raw /ip/firewall/filter/print detail=yes
```

## Interpreting failures

- `USAGE_ERROR`: the raw path is malformed, missing `/`, contains whitespace, or tries to mutate state without explicit permission.
- `UNSUPPORTED_ACTION`: check the path/action shape or use `commands` and `schema command` first.
- `ROS_API_FAILURE`: RouterOS rejected the command; summarize the message and avoid guessing.
- `NETWORK_ERROR` or `AUTH_FAILED`: report whether the failure happened before command execution.

## Escalation boundary

If the user asks for a non-`/print` raw command, stop and explain that it may mutate RouterOS state. Ask for explicit confirmation and prefer a dry-run/change plan when the higher-level roswire command supports it.
