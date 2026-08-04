#!/usr/bin/env bash
#
# Disposable Cortana desktop control plane drill.
#
# Verifies the offline CLI control plane end to end - init, bounded ingestion
# of a small JSONL fixture, bounded search/context retrieval, metadata-only
# audit export, verified backup, restore into a second temporary data
# directory, and a final verify - entirely inside a fresh temporary directory.
#
# Safety boundary: the drill never reads or mutates the live Cortana
# configuration or index, never starts a server, connector, embedding
# service, recurring service, or sync, and removes its temporary directory on
# exit unless CORTANA_KEEP_DRILL=1. Every invocation passes --offline with an
# explicit --config inside the drill directory, and CORTANA_CONFIG is exported
# so a command that somehow lost its --config flag still resolves to the
# drill configuration instead of the live default.
set -euo pipefail

usage() {
  cat <<'EOF'
Run a disposable Cortana desktop control plane drill.

The drill exercises the offline CLI only (init, ingest, search/context, audit
export, backup, restore, verify) inside a fresh temporary directory that is
removed on exit. It never touches the live configuration or index and never
starts a server, connector, embedding service, recurring service, or sync.
It is not a proof of the Desktop GUI, OAuth flows, tray integration, or
updater behavior.

Environment:
  CORTANA_BINARY      Cortana executable (default: cortana)
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
keep="${CORTANA_KEEP_DRILL:-0}"

[[ "$keep" == "0" || "$keep" == "1" ]] || {
  echo "CORTANA_KEEP_DRILL must be 0 or 1" >&2
  exit 2
}
command -v "$binary" >/dev/null 2>&1 || {
  echo "Cortana binary not found: $binary (set CORTANA_BINARY)" >&2
  exit 1
}

drill_dir="$(mktemp -d "${TMPDIR:-/tmp}/cortana-control-plane-drill.XXXXXX")"
log="$drill_dir/control-plane.log"

cleanup() {
  local status=$?
  if [[ "$keep" == "1" ]]; then
    echo "Control plane drill retained: $drill_dir" >&2
  else
    # Only ever remove the exact fresh mktemp directory; anything else is a
    # refusal so a corrupt path can never broaden the deletion.
    case "$drill_dir" in
      "${TMPDIR:-/tmp}"/cortana-control-plane-drill.*)
        rm -rf -- "$drill_dir"
        ;;
      *)
        echo "Refusing to remove unexpected drill directory: $drill_dir" >&2
        status=1
        ;;
    esac
  fi
  return "$status"
}
trap cleanup EXIT

# Fallback for every invocation; commands below also pass --config explicitly.
export CORTANA_CONFIG="$drill_dir/config.toml"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_file() {
  [[ -s "$1" ]] || fail "expected non-empty file: $1"
}

assert_contains() {
  grep -qF -- "$2" "$1" || fail "expected \"$2\" in $1"
}

assert_absent() {
  if grep -qF -- "$2" "$1"; then
    fail "unexpected \"$2\" in $1"
  fi
}

# Tee every step to the log and to a per-step output file used by assertions.
run_step() {
  local step="$1"
  shift
  local output="$drill_dir/step-${step}.out"
  echo "==> [${step}] $*" | tee -a "$log"
  "$@" 2>&1 | tee -a "$log" | tee "$output"
}

data_dir_1="$drill_dir/data-1"
data_dir_2="$drill_dir/data-2"
config_1="$drill_dir/config-1.toml"
config_2="$drill_dir/config-2.toml"
fixture="$drill_dir/fixture.jsonl"
snapshot="$drill_dir/snapshot.sqlite3"
audit_export="$drill_dir/audit.jsonl"

echo "==> Control plane drill directory: $drill_dir" | tee -a "$log"
echo "==> Binary: $binary" | tee -a "$log"

# 1. Initialize the first temporary configuration and data directory with
#    deterministic offline embeddings (dimension 256).
run_step init "$binary" init --offline --config "$config_1" --data-dir "$data_dir_1"
assert_file "$config_1"
assert_contains "$config_1" "data_dir = \"$data_dir_1\""

# 2. Small JSONL fixture: two documents only.
cat >"$fixture" <<'EOF'
{"source":"control-plane-drill","source_id":"drill-doc-001","title":"Cortana control plane drill guide","content":"The control plane drill verifies offline initialization, bounded ingestion, retrieval, metadata-only audit export, verified backup, restore, and final verify. A backup that has never been restored is not a proven recovery path."}
{"source":"control-plane-drill","source_id":"drill-doc-002","title":"Offline query bounds","content":"Search and context queries in the drill stay bounded and offline. The drill index lives in a disposable temporary data directory and is deleted on exit."}
EOF
assert_file "$fixture"

# 3. Ingest the fixture into the disposable index.
run_step ingest "$binary" ingest --offline --config "$config_1" "$fixture"
assert_file "$data_dir_1/cortana.sqlite3"

# 4. Bounded retrieval: search (limit 3) and context (limit 3) must return the
#    fixture document; context also records a metadata-only audit event.
run_step search "$binary" search --offline --config "$config_1" \
  "verified backup restore drill" --limit 3
assert_contains "$drill_dir/step-search.out" "drill-doc-001"
assert_contains "$drill_dir/step-search.out" "Cortana control plane drill guide"

run_step context "$binary" context --offline --config "$config_1" \
  "verified backup restore drill" --limit 3
assert_contains "$drill_dir/step-context.out" "drill-doc-001"

# 5. Export the metadata-only audit trail and prove it is metadata only: the
#    context event must be present, but query text and document content must
#    never appear.
run_step audit "$binary" audit export --offline --config "$config_1" "$audit_export"
assert_file "$audit_export"
assert_contains "$audit_export" '"action":"local-cli/context"'
assert_contains "$audit_export" '"outcome":"succeeded"'
assert_absent "$audit_export" "verified backup restore drill"
assert_absent "$audit_export" "proven recovery path"

# 6. Create a verified backup snapshot.
run_step backup "$binary" backup --offline --config "$config_1" "$snapshot"
assert_file "$snapshot"
assert_contains "$drill_dir/step-backup.out" "backup verified"

# 7. Initialize a second temporary configuration and data directory.
run_step init-restore "$binary" init --offline --config "$config_2" --data-dir "$data_dir_2"
assert_file "$config_2"
assert_contains "$config_2" "data_dir = \"$data_dir_2\""

# 8. Restore the backup into the second disposable data directory.
run_step restore "$binary" restore --offline --config "$config_2" "$snapshot" --force
assert_file "$data_dir_2/cortana.sqlite3"
assert_contains "$drill_dir/step-restore.out" "database restored"

# 9. Verify the restored index integrity, then prove the restored content is
#    searchable (a backup that has never been restored is not a proven path).
run_step verify "$binary" verify --offline --config "$config_2"
assert_contains "$drill_dir/step-verify.out" "database verified"

run_step verify-search "$binary" search --offline --config "$config_2" \
  "verified backup restore drill" --limit 3
assert_contains "$drill_dir/step-verify-search.out" "drill-doc-001"
assert_contains "$drill_dir/step-verify-search.out" "Cortana control plane drill guide"

echo "Control plane drill passed" | tee -a "$log"
if [[ "$keep" == "1" ]]; then
  echo "  record: $log" >&2
fi
