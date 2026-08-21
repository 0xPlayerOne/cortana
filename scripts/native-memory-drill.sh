#!/usr/bin/env bash
set -euo pipefail

# Disposable, offline proof of Cortana's native-memory lifecycle. The drill
# always creates a temporary config/data directory and never reads the live
# index, credentials, or source connectors.
binary="${CORTANA_BINARY:-cortana}"
python="${CORTANA_PYTHON:-python3}"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/cortana-native-memory-drill.XXXXXX")"
trap 'rm -rf -- "$tmp_dir"' EXIT

config="$tmp_dir/config.toml"
data_dir="$tmp_dir/data"

json_field() {
  local field="$1"
  "$python" -c 'import json, sys; print(json.load(sys.stdin)[sys.argv[1]])' "$field"
}

json_list_length() {
  "$python" -c 'import json, sys; print(len(json.load(sys.stdin)))'
}

"$binary" --offline init --config "$config" --data-dir "$data_dir" >/dev/null

first="$("$binary" --offline --config "$config" memory remember \
  --kind preference \
  --project work \
  --title "Native-memory drill preference" \
  --content "Prefer bounded native memory checks" \
  --source drill \
  --source-id native-memory-drill \
  --dedupe-key drill:preference \
  --provenance '{"test":"native-memory-drill"}')"
memory_id="$(printf '%s' "$first" | json_field id)"

retry="$("$binary" --offline --config "$config" memory remember \
  --kind preference \
  --project work \
  --title "Native-memory drill preference" \
  --content "Prefer bounded native memory checks" \
  --source drill \
  --source-id native-memory-drill \
  --dedupe-key drill:preference \
  --provenance '{"test":"native-memory-drill"}')"
retry_id="$(printf '%s' "$retry" | json_field id)"
[[ "$retry_id" == "$memory_id" ]]

recall="$("$binary" --offline --config "$config" memory recall \
  "bounded native memory" --project work --limit 10)"
[[ "$(printf '%s' "$recall" | json_list_length)" == "1" ]]

expired="$("$binary" --offline --config "$config" memory remember \
  --kind working \
  --project work \
  --title "Expired drill state" \
  --content "This must not be recalled" \
  --dedupe-key drill:expired \
  --valid-until 2000-01-01T00:00:00Z)"
expired_id="$(printf '%s' "$expired" | json_field id)"
expired_recall="$("$binary" --offline --config "$config" memory recall \
  "must not be recalled" --project work --limit 10)"
[[ "$(printf '%s' "$expired_recall" | json_list_length)" == "0" ]]

exported="$("$binary" --offline --config "$config" memory export --project work --limit 10)"
"$python" -c '
import json, sys
rows = json.load(sys.stdin)
ids = {row["id"] for row in rows}
assert sys.argv[1] in ids and sys.argv[2] in ids
active = next(row for row in rows if row["id"] == sys.argv[1])
assert active["status"] == "active" and active["content"]
' "$memory_id" "$expired_id" <<<"$exported"

forgotten="$("$binary" --offline --config "$config" memory forget "$memory_id")"
[[ "$(printf '%s' "$forgotten" | json_field forgotten)" == "True" ]]

redacted="$("$binary" --offline --config "$config" memory export --project work --limit 10)"
"$python" -c '
import json, sys
rows = json.load(sys.stdin)
row = next(item for item in rows if item["id"] == sys.argv[1])
assert row["status"] == "retracted" and row["content"] == ""
' "$memory_id" <<<"$redacted"

printf '%s\n' "native memory drill passed (dedupe, recall, expiry, export, forget)"
