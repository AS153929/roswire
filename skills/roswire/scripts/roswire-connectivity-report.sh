#!/usr/bin/env bash
# Collect read-only connectivity evidence for a RouterOS device profile.
#
# Usage:
#   roswire-connectivity-report.sh --profile studio --out ./connectivity-evidence
#
# This script writes JSON artifacts only. It does not mutate RouterOS state.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: roswire-connectivity-report.sh --profile <profile> [--out <dir>]

Options:
  --profile <profile>  Required roswire profile name for remote read-only checks.
  --out <dir>          Output directory for JSON artifacts (default: ./roswire-connectivity-<profile>-<timestamp>).
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
  out_dir="./roswire-connectivity-${profile}-${timestamp}"
fi

mkdir -p "$out_dir"

collect() {
  local name="$1"
  shift
  echo "collecting ${name}.json" >&2
  if "$@" >"${out_dir}/${name}.json" 2>"${out_dir}/${name}.stderr.json"; then
    rm -f "${out_dir}/${name}.stderr.json"
  else
    local rc=$?
    echo "command failed for ${name}; stderr saved to ${out_dir}/${name}.stderr.json" >&2
    return "$rc"
  fi
}

# Keep going after individual remote failures so the agent can inspect partial evidence.
collect local-doctor roswire --json doctor || true
collect remote-doctor roswire --json --profile "$profile" doctor --include-remote || true
collect ip-address roswire --json --profile "$profile" ip address print || true
collect interfaces roswire --json --profile "$profile" interface print || true
collect routes roswire --json --profile "$profile" ip route print || true
collect netwatch roswire --json --profile "$profile" tool netwatch print || true
collect cloud roswire --json --profile "$profile" raw /ip/cloud/print || true
collect dns roswire --json --profile "$profile" raw /ip/dns/print || true

cat <<EOF
Connectivity evidence collection complete.
Profile: ${profile}
Output directory: ${out_dir}

Ask the agent to summarize:
- local doctor status and warnings;
- remote selected protocol and any error_code;
- interface/address placement;
- default route and WAN evidence;
- Netwatch, Cloud, and DNS clues.
EOF
