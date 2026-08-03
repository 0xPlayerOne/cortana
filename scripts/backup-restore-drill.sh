#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Run a disposable Cortana backup and restore verification drill.

Environment:
  CORTANA_BINARY      Cortana executable (default: cortana)
  CORTANA_CONFIG      Live configuration to snapshot
                      (default: $XDG_CONFIG_HOME/cortana/config.toml)
  CORTANA_KEEP_DRILL  Set to 1 to keep the temporary drill directory
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi
if [[ "$#" -ne 0 ]]; then
  usage >&2
  exit 2
fi

binary="${CORTANA_BINARY:-cortana}"
config="${CORTANA_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/cortana/config.toml}"
keep="${CORTANA_KEEP_DRILL:-0}"

[[ -f "$config" ]] || {
  echo "Cortana config is missing: $config" >&2
  exit 1
}
[[ "$keep" == "0" || "$keep" == "1" ]] || {
  echo "CORTANA_KEEP_DRILL must be 0 or 1" >&2
  exit 2
}

drill_dir="$(mktemp -d "${TMPDIR:-/tmp}/cortana-recovery-drill.XXXXXX")"
log="$drill_dir/recovery.log"
drill_config="$drill_dir/config.toml"

cleanup() {
  local status=$?
  if [[ "$keep" == "1" ]]; then
    echo "Recovery drill retained: $drill_dir" >&2
  else
    rm -rf -- "$drill_dir"
  fi
  return "$status"
}
trap cleanup EXIT

run_step() {
  "$@" 2>&1 | tee -a "$log"
}

echo "Creating verified backup from: $config"
run_step "$binary" --config "$config" backup "$drill_dir/source.sqlite3"

echo "Restoring into disposable data directory: $drill_dir/data"
run_step "$binary" --config "$drill_config" init --data-dir "$drill_dir/data"
run_step "$binary" --config "$drill_config" restore "$drill_dir/source.sqlite3" --force
run_step "$binary" --config "$drill_config" verify "$drill_dir/data/cortana.sqlite3"

echo "Recovery drill passed"
if [[ "$keep" == "1" ]]; then
  echo "  record: $log"
fi
