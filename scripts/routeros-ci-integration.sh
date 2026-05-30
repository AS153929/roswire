#!/usr/bin/env bash
set -euo pipefail

# CI-focused RouterOS integration harness.
# It assumes a disposable RouterOS / CHR target is already listening on the
# configured host and API port. The script only runs read-only RouterOS commands.

ROSWIRE_BIN="${ROSWIRE_BIN:-./target/debug/roswire}"
OUT_DIR="${ROSWIRE_ROUTEROS_CI_OUT:-target/routeros-ci}"
ROSWIRE_HOME="${ROSWIRE_HOME:-$OUT_DIR/roswire-home}"
ROUTEROS_HOST="${ROUTEROS_HOST:-127.0.0.1}"
ROUTEROS_USER="${ROUTEROS_USER:-admin}"
ROUTEROS_PASSWORD="${ROUTEROS_PASSWORD:-}"
ROUTEROS_API_PORT="${ROUTEROS_API_PORT:-8728}"
ROUTEROS_PROFILE="${ROUTEROS_PROFILE:-ci-routeros}"
ROSWIRE_READY_ATTEMPTS="${ROSWIRE_READY_ATTEMPTS:-30}"
ROSWIRE_READY_TIMEOUT_SECONDS="${ROSWIRE_READY_TIMEOUT_SECONDS:-8}"

export ROSWIRE_HOME

mkdir -p "$OUT_DIR"
rm -rf "$ROSWIRE_HOME"

if [[ ! -x "$ROSWIRE_BIN" ]]; then
  printf 'roswire binary not executable: %s\n' "$ROSWIRE_BIN" >&2
  printf 'Build first, for example: cargo build --locked\n' >&2
  exit 2
fi

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
  printf '{"case":"%s","exit_code":%s,"command":"%s"}\n' \
    "$(json_escape "$name")" \
    "$rc" \
    "$(json_escape "$command")" > "$OUT_DIR/$name.meta.json"
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
  if [[ "$rc" != "0" ]]; then
    printf 'case failed: %s\n' "$name" >&2
    sed -n '1,160p' "$stderr" >&2
    exit "$rc"
  fi
}

run_stdin_case() {
  local name="$1"
  local stdin_value="$2"
  shift 2
  local stdout="$OUT_DIR/$name.stdout.json"
  local stderr="$OUT_DIR/$name.stderr.json"
  local command="${ROSWIRE_BIN} $*"
  local rc=0

  printf '==> %s\n' "$name"
  if printf '%s' "$stdin_value" | "$ROSWIRE_BIN" "$@" > "$stdout" 2> "$stderr"; then
    rc=0
  else
    rc=$?
  fi
  write_meta "$name" "$rc" "$command"
  printf '    exit=%s stdout=%s stderr=%s\n' "$rc" "$stdout" "$stderr"
  if [[ "$rc" != "0" ]]; then
    printf 'case failed: %s\n' "$name" >&2
    sed -n '1,160p' "$stderr" >&2
    exit "$rc"
  fi
}

assert_stdout_contains() {
  local name="$1"
  local needle="$2"
  local stdout="$OUT_DIR/$name.stdout.json"

  if ! grep -Fq "$needle" "$stdout"; then
    printf 'expected %s stdout to contain: %s\n' "$name" "$needle" >&2
    sed -n '1,160p' "$stdout" >&2
    exit 1
  fi
}

wait_for_tcp() {
  local host="$1"
  local port="$2"
  local attempts="${3:-90}"

  for _ in $(seq 1 "$attempts"); do
    if timeout 2 bash -c ":</dev/tcp/${host}/${port}" 2>/dev/null; then
      return 0
    fi
    sleep 2
  done

  printf 'timed out waiting for %s:%s\n' "$host" "$port" >&2
  return 1
}

wait_for_roswire_remote() {
  local attempts="${1:-$ROSWIRE_READY_ATTEMPTS}"
  local stdout="$OUT_DIR/routeros-ready.stdout.json"
  local stderr="$OUT_DIR/routeros-ready.stderr.json"

  for _ in $(seq 1 "$attempts"); do
    if timeout "$ROSWIRE_READY_TIMEOUT_SECONDS" \
      "$ROSWIRE_BIN" --profile "$ROUTEROS_PROFILE" doctor --include-remote --json \
      > "$stdout" 2> "$stderr" &&
      grep -Fq '"status": "ok"' "$stdout"; then
      return 0
    fi
    sleep 2
  done

  printf 'timed out waiting for roswire remote doctor to pass\n' >&2
  sed -n '1,160p' "$stdout" >&2 || true
  sed -n '1,160p' "$stderr" >&2 || true
  return 1
}

printf 'Waiting for RouterOS API at %s:%s\n' "$ROUTEROS_HOST" "$ROUTEROS_API_PORT"
wait_for_tcp "$ROUTEROS_HOST" "$ROUTEROS_API_PORT"

run_case config-init config init --json
assert_stdout_contains config-init '"schema_version": "roswire.config.init.v1"'

run_case config-device-add \
  config device add "$ROUTEROS_PROFILE" \
  "host=$ROUTEROS_HOST" \
  "user=$ROUTEROS_USER" \
  protocol=api \
  routeros_version=auto \
  transfer=ssh \
  "port=$ROUTEROS_API_PORT" \
  --json
assert_stdout_contains config-device-add '"schema_version": "roswire.config.device.v1"'

run_stdin_case config-secret-password "$ROUTEROS_PASSWORD" \
  --stdin config secret set "$ROUTEROS_PROFILE" password type=plain --json
assert_stdout_contains config-secret-password '"schema_version": "roswire.config.secret.v1"'

run_case config-inspect --profile "$ROUTEROS_PROFILE" config inspect --json
assert_stdout_contains config-inspect "\"active_profile\": \"$ROUTEROS_PROFILE\""
assert_stdout_contains config-inspect '"redacted": true'

printf 'Waiting for RouterOS API login through roswire\n'
wait_for_roswire_remote

run_case remote-doctor --profile "$ROUTEROS_PROFILE" doctor --include-remote --json
assert_stdout_contains remote-doctor '"schema_version": "roswire.doctor.v1"'
assert_stdout_contains remote-doctor '"status": "ok"'
assert_stdout_contains remote-doctor '"selected_protocol": "api"'

run_case remote-system-resource --profile "$ROUTEROS_PROFILE" system resource print --json
assert_stdout_contains remote-system-resource '"version"'

run_case remote-interface-print --profile "$ROUTEROS_PROFILE" interface print --json
assert_stdout_contains remote-interface-print '"name"'

run_case remote-ip-address-print --profile "$ROUTEROS_PROFILE" ip address print --json

run_case remote-raw-resource --profile "$ROUTEROS_PROFILE" raw /system/resource/print --json
assert_stdout_contains remote-raw-resource '"version"'

run_case remote-raw-interface-detail --profile "$ROUTEROS_PROFILE" raw /interface/print detail --json
assert_stdout_contains remote-raw-interface-detail '"name"'

run_case remote-schema-discover --profile "$ROUTEROS_PROFILE" schema discover --remote --json
assert_stdout_contains remote-schema-discover '"schema_version": "roswire.remote.schema.v1"'

printf 'RouterOS CI integration finished. Evidence: %s\n' "$OUT_DIR"
