#!/usr/bin/env bash
set -uo pipefail

# RouterOS / CHR acceptance harness for RosWire.
# Secrets must be supplied through environment variables or roswire profiles.
# This script never enables shell tracing and does not print secret values.

ROSWIRE_BIN="${ROSWIRE_BIN:-./target/release/roswire}"
OUT_DIR="${ROSWIRE_ACCEPTANCE_OUT:-target/acceptance/routeros}"
REMOTE_ENABLED="${ROSWIRE_ACCEPTANCE_REMOTE:-0}"
FILE_WORKFLOWS_ENABLED="${ROSWIRE_ACCEPTANCE_RUN_FILE_WORKFLOWS:-0}"
TRANSFER_DRY_RUN_ENABLED="${ROSWIRE_ACCEPTANCE_RUN_TRANSFER_DRY_RUN:-0}"
ACCEPTANCE_SSH_HOST_KEY="${ROSWIRE_ACCEPTANCE_SSH_HOST_KEY:-}"
ACCEPTANCE_ALLOW_FROM="${ROSWIRE_ACCEPTANCE_ALLOW_FROM:-}"

mkdir -p "$OUT_DIR"

json_escape() {
  local value="$1"
  value=${value//\\/\\\\}
  value=${value//"/\\"}
  value=${value//$'\n'/\\n}
  value=${value//$'\r'/\\r}
  printf '%s' "$value"
}

write_meta() {
  local name="$1"
  local rc="$2"
  local command="$3"
  local meta="$OUT_DIR/$name.meta.json"
  printf '{"case":"%s","exit_code":%s,"command":"%s"}\n' \
    "$(json_escape "$name")" \
    "$rc" \
    "$(json_escape "$command")" > "$meta"
}

run_case() {
  local name="$1"
  shift
  local stdout="$OUT_DIR/$name.stdout.json"
  local stderr="$OUT_DIR/$name.stderr.json"
  local command="${ROSWIRE_BIN} $*"
  local rc=0

  printf '==> %s\n' "$name"
  if "$ROSWIRE_BIN" "$@" > "$stdout" 2> "$stderr"; then
    rc=0
  else
    rc=$?
  fi
  write_meta "$name" "$rc" "$command"
  printf '    exit=%s stdout=%s stderr=%s\n' "$rc" "$stdout" "$stderr"
}

skip_case() {
  local name="$1"
  local reason="$2"
  printf '==> %s (skipped: %s)\n' "$name" "$reason"
  printf '{"case":"%s","skipped":true,"reason":"%s"}\n' \
    "$(json_escape "$name")" \
    "$(json_escape "$reason")" > "$OUT_DIR/$name.meta.json"
}

if [[ ! -x "$ROSWIRE_BIN" ]]; then
  printf 'roswire binary not executable: %s\n' "$ROSWIRE_BIN" >&2
  printf 'Build first, for example: cargo build --release --locked\n' >&2
  exit 2
fi

printf 'RosWire acceptance output: %s\n' "$OUT_DIR"

run_case local-doctor doctor --json
run_case local-commands commands --json
run_case local-help help --json
run_case local-schema-ip-address-add schema command ip address add --json

if [[ "$REMOTE_ENABLED" != "1" ]]; then
  skip_case remote-doctor "set ROSWIRE_ACCEPTANCE_REMOTE=1 after configuring a RouterOS/CHR target"
  skip_case remote-schema-discover "set ROSWIRE_ACCEPTANCE_REMOTE=1 after configuring a RouterOS/CHR target"
  skip_case remote-interface-print "set ROSWIRE_ACCEPTANCE_REMOTE=1 after configuring a RouterOS/CHR target"
  skip_case remote-system-resource-print "set ROSWIRE_ACCEPTANCE_REMOTE=1 after configuring a RouterOS/CHR target"
else
  run_case remote-doctor doctor --include-remote --json
  run_case remote-schema-discover schema discover --remote --json
  run_case remote-interface-print interface print --json
  run_case remote-system-resource-print system resource print --json
  run_case remote-explicit-api --protocol api system resource print --json
  run_case remote-explicit-api-ssl --protocol api-ssl system resource print --json
  run_case remote-explicit-rest --protocol rest system resource print --json
fi

if [[ "$TRANSFER_DRY_RUN_ENABLED" == "1" || -n "$ACCEPTANCE_SSH_HOST_KEY" || -n "$ACCEPTANCE_ALLOW_FROM" ]]; then
  sample="$OUT_DIR/roswire-acceptance.rsc"
  printf ':put "roswire acceptance"\n' > "$sample"
  transfer_args=(file upload "$sample" flash/roswire-acceptance.rsc --dry-run --json)
  if [[ -n "$ACCEPTANCE_SSH_HOST_KEY" ]]; then
    transfer_args+=(--ssh-host-key "$ACCEPTANCE_SSH_HOST_KEY")
  fi
  if [[ -n "$ACCEPTANCE_ALLOW_FROM" ]]; then
    transfer_args+=(--allow-from "$ACCEPTANCE_ALLOW_FROM")
  fi
  run_case transfer-file-upload-dry-run "${transfer_args[@]}"
else
  skip_case transfer-file-upload-dry-run "set ROSWIRE_ACCEPTANCE_RUN_TRANSFER_DRY_RUN=1 to use profile ssh_host_key/allow_from, or set ROSWIRE_ACCEPTANCE_SSH_HOST_KEY/ROSWIRE_ACCEPTANCE_ALLOW_FROM"
fi

if [[ "$FILE_WORKFLOWS_ENABLED" != "1" ]]; then
  skip_case file-workflows-live "set ROSWIRE_ACCEPTANCE_RUN_FILE_WORKFLOWS=1 only on disposable lab targets"
else
  sample="$OUT_DIR/roswire-acceptance-live.rsc"
  printf ':put "roswire live acceptance"\n' > "$sample"
  live_upload_args=(file upload "$sample" flash/roswire-acceptance-live.rsc --json)
  live_export_args=(export download "$OUT_DIR/roswire-export.rsc" --cleanup --json)
  if [[ -n "$ACCEPTANCE_SSH_HOST_KEY" ]]; then
    live_upload_args+=(--ssh-host-key "$ACCEPTANCE_SSH_HOST_KEY")
    live_export_args+=(--ssh-host-key "$ACCEPTANCE_SSH_HOST_KEY")
  fi
  run_case live-file-upload "${live_upload_args[@]}"
  run_case live-export-download "${live_export_args[@]}"
fi

printf 'Acceptance harness finished. Review *.meta.json plus stdout/stderr payloads under %s.\n' "$OUT_DIR"
