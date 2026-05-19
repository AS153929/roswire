#!/usr/bin/env bash
# Collect a read-only RouterOS JSON snapshot for an existing roswire profile.
#
# Usage:
#   roswire-readonly-snapshot.sh --profile studio --out ./roswire-snapshot
#
# This script does not mutate RouterOS state. It writes JSON artifacts to the
# output directory and avoids printing secrets or command traces.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: roswire-readonly-snapshot.sh --profile <profile> [--out <dir>]

Options:
  --profile <profile>  Required roswire profile name for remote read-only checks.
  --out <dir>          Output directory for JSON artifacts (default: ./roswire-snapshot-<profile>-<timestamp>).
  -h, --help           Show this help.
USAGE
}

profile=""
out_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      [[ $# -ge 2 ]] || { echo "missing value for --profile" >&2; exit 2; }
      profile="$2"
      shift 2
      ;;
    --out)
      [[ $# -ge 2 ]] || { echo "missing value for --out" >&2; exit 2; }
      out_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$profile" ]]; then
  echo "--profile is required" >&2
  usage >&2
  exit 2
fi

if [[ -z "$out_dir" ]]; then
  timestamp=$(date -u +%Y%m%dT%H%M%SZ)
  out_dir="./roswire-snapshot-${profile}-${timestamp}"
fi

mkdir -p "$out_dir"

run_json() {
  local name="$1"
  shift
  echo "collecting ${name}.json" >&2
  roswire --json --profile "$profile" "$@" >"${out_dir}/${name}.json"
}

roswire --json doctor >"${out_dir}/local-doctor.json"
roswire --json --profile "$profile" doctor --include-remote >"${out_dir}/remote-doctor.json"
run_json ip-address ip address print
run_json interfaces interface print
run_json routes ip route print
run_json netwatch tool netwatch print
run_json system-resource system resource print
run_json cloud raw /ip/cloud/print
run_json dns raw /ip/dns/print
run_json identity raw /system/identity/print

cat <<EOF
Read-only snapshot complete.
Profile: ${profile}
Output directory: ${out_dir}

Suggested next step:
Ask the agent to summarize connectivity, routing, interface state, and warnings from these JSON artifacts.
EOF
