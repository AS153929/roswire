#!/usr/bin/env bash
# Demonstrate the agent + roswire discovery loop with local schema commands.
#
# Usage:
#   roswire-schema-tour.sh --out ./schema-tour
#   roswire-schema-tour.sh --profile studio --remote --out ./schema-tour
#
# Local discovery is read-only and does not contact RouterOS. Remote schema
# discovery is also read-only, but it requires an explicit profile.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: roswire-schema-tour.sh [--profile <profile>] [--remote] [--out <dir>]

Options:
  --profile <profile>  Profile to use for optional remote schema discovery.
  --remote             Also run remote schema discovery. Requires --profile.
  --out <dir>          Output directory for JSON artifacts (default: ./roswire-schema-tour-<timestamp>).
  -h, --help           Show this help.
USAGE
}

profile=""
out_dir=""
remote="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      [[ $# -ge 2 ]] || { echo "missing value for --profile" >&2; exit 2; }
      profile="$2"
      shift 2
      ;;
    --remote)
      remote="true"
      shift
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

if [[ "$remote" == "true" && -z "$profile" ]]; then
  echo "--remote requires --profile" >&2
  usage >&2
  exit 2
fi

if [[ -z "$out_dir" ]]; then
  timestamp=$(date -u +%Y%m%dT%H%M%SZ)
  out_dir="./roswire-schema-tour-${timestamp}"
fi

mkdir -p "$out_dir"

collect() {
  local name="$1"
  shift
  echo "collecting ${name}.json" >&2
  "$@" >"${out_dir}/${name}.json"
}

collect commands roswire --json commands
collect help-index roswire --json help
collect schema-ip-route-print roswire --json schema command ip route print
collect schema-raw roswire --json schema command raw
collect schema-config-device-add roswire --json schema command config device add
collect schema-script-put roswire --json schema command script put

if [[ "$remote" == "true" ]]; then
  collect remote-schema-discover roswire --json --profile "$profile" --remote schema discover
fi

cat <<EOF
Schema tour complete.
Output directory: ${out_dir}

Suggested agent loop:
1. Read commands.json to find supported topics.
2. Read schema-*.json before constructing arguments.
3. Prefer read-only print commands for evidence.
4. Use --dry-run before any planned mutation.
EOF
