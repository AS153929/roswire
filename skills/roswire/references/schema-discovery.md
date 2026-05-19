# Schema Discovery

Use this playbook to demonstrate how an agent can discover roswire capabilities before choosing a command. This is the core agent + roswire loop: inspect the catalog, inspect a command schema, run a safe command, and summarize structured JSON.

## Safety boundary

- Discovery commands are local and read-only unless `--remote` is explicitly used.
- Remote schema discovery should only be used when a profile is available and the user wants device-specific hints.
- Do not invent command shapes when `commands` or `schema command` can answer the question.

## Local discovery commands

List available commands:

```bash
roswire --json commands
```

Inspect command help and schema:

```bash
roswire --json help ip address add
roswire --json schema command ip route print
roswire --json schema command raw
roswire --json schema command config device add
```

Use these outputs to learn:

- command names and token order;
- required and optional arguments;
- side-effect and idempotency hints;
- examples and repair hints;
- whether a command is read-only or may mutate state.

## Remote overlay discovery

When a real profile exists and the user wants device-specific command hints:

```bash
roswire --json --profile <profile> --remote schema discover
roswire --json --profile <profile> --remote --refresh schema discover
```

Remote discovery can report degraded results if configuration or connectivity is missing. Summarize `status`, `cache_key`, warnings, and any `error_code` instead of treating a degraded overlay as a hard failure.

## Recommended agent loop

1. Run `roswire --json commands` when the task mentions an unfamiliar RouterOS area.
2. Run `roswire --json schema command <topic...>` before constructing a command with arguments.
3. Prefer read-only `print` commands for evidence gathering.
4. Use `--dry-run` before any planned mutation.
5. Summarize the JSON evidence and cite the commands used.

## Example scenario

For a routing question:

```bash
roswire --json commands
roswire --json schema command ip route print
roswire --json --profile <profile> ip route print
```

For an unsupported read-only path:

```bash
roswire --json schema command raw
roswire --json --profile <profile> raw /system/resource/print
```
