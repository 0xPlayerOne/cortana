#!/usr/bin/env bash
set -euo pipefail

# Run bounded source authorization/validation checks and, when explicitly
# requested, non-reconciling trial syncs. This is intentionally a small
# operator probe: it never enables a source, installs a schedule, or deletes
# records from a partial snapshot. Filesystem/code validations pass --sample
# so an oversized root records a bounded sample; connector sources keep
# ordinary fail-closed validation, and no token value is ever read or printed.

usage() {
  cat <<'EOF'
Usage: scripts/source-smoke.sh [OPTIONS] [SOURCE ...]

Validate configured sources within a bounded budget. Filesystem/code sources
are validated as a bounded sample (--sample); connector sources keep ordinary
fail-closed validation. With --sync, also ingest a bounded, non-reconciling
trial; filesystem/code trials require --include-filesystem.

Options:
  --config PATH             Cortana TOML configuration
  --binary PATH             Cortana executable
  --max-documents N         Per-source document cap (default: 25)
  --max-bytes N             Per-source content-byte cap (default: 5242880)
  --max-seconds N           Per-source wall-clock cap (default: 60)
  --sync                    Run a non-reconciling trial sync after validation
  --include-disabled        Validate disabled configured sources too
  --include-filesystem      Allow trial syncs for filesystem/code sources
  -h, --help                Show this help

Environment overrides:
  CORTANA_BINARY, CORTANA_CONFIG, CORTANA_SOURCE_SMOKE_MAX_DOCUMENTS,
  CORTANA_SOURCE_SMOKE_MAX_BYTES, CORTANA_SOURCE_SMOKE_MAX_SECONDS
EOF
}

binary_path="${CORTANA_BINARY:-${HOME}/.local/bin/cortana}"
config_path="${CORTANA_CONFIG:-${XDG_CONFIG_HOME:-${HOME}/.config}/cortana/config.toml}"
max_documents="${CORTANA_SOURCE_SMOKE_MAX_DOCUMENTS:-25}"
max_bytes="${CORTANA_SOURCE_SMOKE_MAX_BYTES:-5242880}"
max_seconds="${CORTANA_SOURCE_SMOKE_MAX_SECONDS:-60}"
run_sync=0
include_disabled=0
include_filesystem=0

while (($#)); do
  case "$1" in
    --config)
      [[ $# -ge 2 ]] || { echo "--config requires a path" >&2; exit 2; }
      config_path="$2"
      shift 2
      ;;
    --binary)
      [[ $# -ge 2 ]] || { echo "--binary requires a path" >&2; exit 2; }
      binary_path="$2"
      shift 2
      ;;
    --max-documents)
      [[ $# -ge 2 ]] || { echo "--max-documents requires a value" >&2; exit 2; }
      max_documents="$2"
      shift 2
      ;;
    --max-bytes)
      [[ $# -ge 2 ]] || { echo "--max-bytes requires a value" >&2; exit 2; }
      max_bytes="$2"
      shift 2
      ;;
    --max-seconds)
      [[ $# -ge 2 ]] || { echo "--max-seconds requires a value" >&2; exit 2; }
      max_seconds="$2"
      shift 2
      ;;
    --sync)
      run_sync=1
      shift
      ;;
    --include-disabled)
      include_disabled=1
      shift
      ;;
    --include-filesystem)
      include_filesystem=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -* )
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      break
      ;;
  esac
done

if [[ ! -x "$binary_path" && -x "${binary_path}.exe" ]]; then
  binary_path="${binary_path}.exe"
fi
[[ -x "$binary_path" ]] || { echo "Cortana binary is not executable: $binary_path" >&2; exit 1; }
[[ -f "$config_path" ]] || { echo "Cortana config does not exist: $config_path" >&2; exit 1; }

for value_name in max_documents max_bytes max_seconds; do
  value="${!value_name}"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || {
    echo "$value_name must be a positive integer" >&2
    exit 2
  }
done

declare -a requested_sources=("$@")
declare -a configured_sources=()

# Use Python's standard TOML parser rather than fragile grep/sed matching.
# Emit only non-secret source metadata; token values are never read or printed.
while IFS=$'\t' read -r name kind enabled; do
  [[ -n "$name" ]] || continue
  if ((${#requested_sources[@]})); then
    selected=0
    for requested in "${requested_sources[@]}"; do
      if [[ "$requested" == "$name" ]]; then
        selected=1
        break
      fi
    done
    ((selected)) || continue
  fi
  if [[ "$enabled" != "true" && "$include_disabled" -ne 1 ]]; then
    continue
  fi
  configured_sources+=("${name}"$'\t'"${kind}"$'\t'"${enabled}")
done < <(
  CONFIG_PATH="$config_path" python3 - <<'PY'
import os
import pathlib
import sys
import tomllib

path = pathlib.Path(os.environ["CONFIG_PATH"])
try:
    with path.open("rb") as handle:
        config = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as error:
    print(f"unable to read TOML config: {error}", file=sys.stderr)
    raise SystemExit(1)

for source in config.get("sources", []):
    name = source.get("name", "")
    kind = source.get("kind", "")
    enabled = "true" if source.get("enabled", True) else "false"
    if isinstance(name, str) and isinstance(kind, str) and name and kind:
        print(f"{name}\t{kind}\t{enabled}")
PY
)

if ((${#configured_sources[@]} == 0)); then
  echo "No configured sources matched the selection." >&2
  exit 1
fi

failures=0
printf 'source\tkind\tenabled\tvalidation\ttrial_sync\tnote\n'

classify_failure() {
  local log_path="$1"
  if grep -Eqi 'timed out|timeout' "$log_path"; then
    printf 'timeout'
  elif grep -Eqi '403 forbidden|401 unauthorized|authorization denied|permission denied' "$log_path"; then
    printf 'authorization denied'
  elif grep -Eqi 'no such file or directory|does not exist|not found' "$log_path"; then
    printf 'credential or path missing'
  elif grep -Eqi 'exceeds .*budget|safety bound|budget exceeded' "$log_path"; then
    printf 'configured budget exceeded'
  else
    printf 'connector or validation error'
  fi
}

for entry in "${configured_sources[@]}"; do
  IFS=$'\t' read -r source kind enabled <<< "$entry"
  validation_status="failed"
  sync_status="not-requested"
  note=""

  validation_log="$(mktemp "${TMPDIR:-/tmp}/cortana-source-validation.XXXXXX")"
  sync_log="$(mktemp "${TMPDIR:-/tmp}/cortana-source-sync.XXXXXX")"
  # Filesystem/code validations explicitly opt into a bounded sample so a root
  # larger than the budget records a partial validation instead of failing;
  # connector sources keep the ordinary fail-closed preflight. The sample can
  # authorize only the equally bounded non-reconciling trial below.
  validation_args=(
    validate-source "$source"
    --max-documents "$max_documents"
    --max-bytes "$max_bytes"
    --max-seconds "$max_seconds"
  )
  if [[ "$kind" == "filesystem" ]]; then
    validation_args+=(--sample)
  fi
  if "$binary_path" --config "$config_path" "${validation_args[@]}" \
      >"$validation_log" 2>&1; then
    validation_status="passed"
  else
    note="validation: $(classify_failure "$validation_log")"
    failures=$((failures + 1))
  fi

  if ((run_sync)); then
    if [[ "$enabled" != "true" ]]; then
      sync_status="skipped-disabled"
      note="${note:+$note; }source is disabled"
    elif [[ "$kind" == "filesystem" && "$include_filesystem" -ne 1 ]]; then
      sync_status="skipped-filesystem"
      note="${note:+$note; }filesystem trial requires --include-filesystem"
    elif [[ "$validation_status" != "passed" ]]; then
      sync_status="skipped-validation"
      note="${note:+$note; }trial requires successful validation"
    elif "$binary_path" --config "$config_path" sync --source "$source" \
        --no-reconcile \
        --max-documents "$max_documents" \
        --max-bytes "$max_bytes" \
        --max-seconds "$max_seconds" \
        --require-validation >"$sync_log" 2>&1; then
      # The trial is equally bounded (same budgets as the validation above)
      # and non-reconciling, so for filesystem sources it may rely on the
      # matching sampled validation via --require-validation while never
      # authorizing a full-corpus sync.
      sync_status="passed"
    else
      sync_status="failed"
      note="${note:+$note; }trial: $(classify_failure "$sync_log")"
      failures=$((failures + 1))
    fi
  fi

  # Keep the default output safe and compact. Detailed connector diagnostics
  # remain in the Cortana status/audit surfaces, not in this summary table.
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$source" "$kind" "$enabled" "$validation_status" "$sync_status" "$note"
  rm -f -- "$validation_log" "$sync_log"
done

if ((failures)); then
  echo "source smoke completed with $failures failure(s)" >&2
  exit 1
fi
echo "source smoke passed"
