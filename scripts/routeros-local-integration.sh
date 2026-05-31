#!/usr/bin/env bash
set -euo pipefail

# One-command local RouterOS read-only integration loop.
#
# Builds roswire, boots a disposable RouterOS CHR container (community
# QEMU-wrapped image; MikroTik ships no official Docker image), waits for it to
# come up, and runs the existing read-only harness
# (scripts/routeros-ci-integration.sh) against it.
#
# Scope: READ-ONLY smoke only. v6/v7 matrix, REST, and live SSH/file workflows
# are intentionally out of scope here (see docs/routeros-local-integration.md).
#
# The CHR image manifest is multi-arch, so Docker selects the host architecture
# automatically (arm64 on Apple Silicon) — same-arch, no cross-arch emulation.
#
# Useful overrides:
#   ROUTEROS_IMAGE          CHR image (default evilfreelancer/docker-routeros:latest)
#   ROUTEROS_API_PORT       host API port (default 8728)
#   ROUTEROS_SSH_PORT       host SSH port (default 2222)
#   ROSWIRE_KEEP_ROUTEROS=1 leave the CHR container running after the run
#   ROSWIRE_SKIP_BUILD=1    skip `cargo build` (use an existing ./target/debug/roswire)

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ROUTEROS_IMAGE="${ROUTEROS_IMAGE:-evilfreelancer/docker-routeros:latest}"
ROUTEROS_CONTAINER="${ROUTEROS_CONTAINER:-roswire-routeros-local}"
ROUTEROS_HOST="${ROUTEROS_HOST:-127.0.0.1}"
ROUTEROS_API_PORT="${ROUTEROS_API_PORT:-8728}"
ROUTEROS_SSH_PORT="${ROUTEROS_SSH_PORT:-2222}"
ROUTEROS_API_SSL_PORT="${ROUTEROS_API_SSL_PORT:-8729}"
ROUTEROS_PASSWORD="${ROUTEROS_PASSWORD:-}"
OUT_DIR="${ROSWIRE_ROUTEROS_CI_OUT:-target/routeros-ci}"
COMPOSE_FILE="$OUT_DIR/docker-compose.routeros-local.yml"

if ! command -v docker >/dev/null 2>&1; then
  printf 'docker is required but was not found on PATH\n' >&2
  exit 2
fi

mkdir -p "$OUT_DIR"

cleanup() {
  if [[ "${ROSWIRE_KEEP_ROUTEROS:-0}" == "1" ]]; then
    printf 'ROSWIRE_KEEP_ROUTEROS=1 set; leaving %s running\n' "$ROUTEROS_CONTAINER"
    return
  fi
  printf 'Tearing down RouterOS CHR container\n'
  docker compose -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

if [[ "${ROSWIRE_SKIP_BUILD:-0}" != "1" ]]; then
  printf 'Building roswire\n'
  cargo build --locked
fi

# Render a disposable compose file. /dev/kvm is only wired in when the host
# exposes it (faster KVM acceleration); otherwise QEMU falls back to TCG.
{
  cat <<YAML
services:
  routeros:
    image: ${ROUTEROS_IMAGE}
    container_name: ${ROUTEROS_CONTAINER}
    restart: "no"
    cap_add:
      - NET_ADMIN
    devices:
      - /dev/net/tun:/dev/net/tun
YAML
  if [[ -e /dev/kvm ]]; then
    printf '      - /dev/kvm:/dev/kvm\n'
  fi
  cat <<YAML
    ports:
      - "${ROUTEROS_HOST}:${ROUTEROS_SSH_PORT}:22"
      - "${ROUTEROS_HOST}:${ROUTEROS_API_PORT}:8728"
      - "${ROUTEROS_HOST}:${ROUTEROS_API_SSL_PORT}:8729"
YAML
} > "$COMPOSE_FILE"

printf 'Pulling %s\n' "$ROUTEROS_IMAGE"
docker pull "$ROUTEROS_IMAGE"

printf 'Starting RouterOS CHR (%s)\n' "$ROUTEROS_CONTAINER"
docker compose -f "$COMPOSE_FILE" up -d
docker ps -a --filter "name=${ROUTEROS_CONTAINER}"

set +e
ROSWIRE_BIN="${ROSWIRE_BIN:-./target/debug/roswire}" \
  ROUTEROS_HOST="$ROUTEROS_HOST" \
  ROUTEROS_USER="${ROUTEROS_USER:-admin}" \
  ROUTEROS_PASSWORD="$ROUTEROS_PASSWORD" \
  ROUTEROS_API_PORT="$ROUTEROS_API_PORT" \
  ROSWIRE_ROUTEROS_CI_OUT="$OUT_DIR" \
  scripts/routeros-ci-integration.sh
status=$?
set -e

printf 'Collecting RouterOS logs\n'
docker logs "$ROUTEROS_CONTAINER" > "$OUT_DIR/docker-routeros.log" 2>&1 || true

if [[ "$status" == "0" ]]; then
  printf 'Local RouterOS integration passed. Evidence: %s\n' "$OUT_DIR"
else
  printf 'Local RouterOS integration FAILED (exit %s). Evidence: %s\n' "$status" "$OUT_DIR" >&2
fi
exit "$status"
