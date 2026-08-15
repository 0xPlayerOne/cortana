#!/usr/bin/env bash
#
# Disposable shared-agent HTTP authorization drill.
#
# This exercises scoped query/status/admin principals, ACL filtering, metadata-
# only audit output, and file-backed token rotation/revocation against a tiny
# synthetic index. It never reads the live Cortana configuration or index,
# never contacts a provider, and never enables a connector or recurring sync.
# The temporary directory is removed on exit unless CORTANA_KEEP_DRILL=1.
set -euo pipefail

usage() {
  cat <<'EOF'
Run a disposable shared-agent authorization drill.

The drill starts an offline HTTP server in a fresh temporary directory,
ingests two synthetic documents, verifies query/status/admin scopes and ACL
filtering, rotates the query token through /v1/auth/reload, and checks that
the old token is revoked. It also proves the audit response contains metadata
only. It never touches the live configuration/index, source credentials, or
sync services, and does not prove a packaged GUI or MCP transport.

Environment:
  CORTANA_BINARY       Cortana executable (default: cortana)
  CORTANA_KEEP_DRILL   Set to 1 to retain the temporary drill directory
  CORTANA_AUTH_PORT    Optional loopback port (default: an ephemeral port)
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
requested_port="${CORTANA_AUTH_PORT:-}"

[[ "$keep" == "0" || "$keep" == "1" ]] || {
  echo "CORTANA_KEEP_DRILL must be 0 or 1" >&2
  exit 2
}
command -v "$binary" >/dev/null 2>&1 || {
  echo "Cortana binary not found: $binary (set CORTANA_BINARY)" >&2
  exit 1
}
command -v curl >/dev/null 2>&1 || {
  echo "curl is required for the authorization drill" >&2
  exit 1
}
command -v python3 >/dev/null 2>&1 || {
  echo "python3 is required for JSON assertions" >&2
  exit 1
}

drill_dir="$(mktemp -d "${TMPDIR:-/tmp}/cortana-shared-agent-auth-drill.XXXXXX")"
server_log="$drill_dir/server.log"
server_pid=""

cleanup() {
  local status=$?
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ "$keep" == "1" ]]; then
    echo "Shared-agent auth drill retained: $drill_dir" >&2
  else
    case "$drill_dir" in
      "${TMPDIR:-/tmp}"/cortana-shared-agent-auth-drill.*)
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

fail() {
  echo "FAIL: $*" >&2
  if [[ -s "$server_log" ]]; then
    echo "--- server log ---" >&2
    tail -80 "$server_log" >&2 || true
  fi
  exit 1
}

assert_status() {
  local label="$1" expected="$2" actual_file="$3"
  local actual
  actual="$(<"$actual_file")"
  [[ "$actual" == "$expected" ]] || fail "$label returned HTTP $actual (expected $expected)"
}

write_secrets() {
  local query_token="$1"
  umask 077
  printf 'CORTANA_QUERY_AGENT_TOKEN=%s\nCORTANA_STATUS_AGENT_TOKEN=%s\nCORTANA_ADMIN_AGENT_TOKEN=%s\n' \
    "$query_token" "status-secret" "admin-secret" >"$secrets"
  chmod 600 "$secrets"
}

write_config() {
  python3 - "$config" "$secrets" <<'PY'
import json
import sys
from pathlib import Path

config_path = Path(sys.argv[1])
secrets_path = Path(sys.argv[2])
text = config_path.read_text()
needle = "[runtime]\n"
if needle not in text:
    raise SystemExit("generated config has no [runtime] table")
text = text.replace(
    needle,
    f"[runtime]\nenv_file = {json.dumps(str(secrets_path))}\n",
    1,
)
tokens = '''[[auth.tokens]]
principal = "query-agent"
token_env = "CORTANA_QUERY_AGENT_TOKEN"
scopes = ["query"]
acl = ["work"]

[[auth.tokens]]
principal = "status-agent"
token_env = "CORTANA_STATUS_AGENT_TOKEN"
scopes = ["query", "status"]
acl = ["work"]

[[auth.tokens]]
principal = "admin-agent"
token_env = "CORTANA_ADMIN_AGENT_TOKEN"
scopes = ["query", "status", "admin"]
acl = []
'''
if "tokens = []" not in text:
    raise SystemExit("generated config has no auth token list")
text = text.replace("tokens = []", tokens.rstrip(), 1)
config_path.write_text(text)
PY
}

request() {
  local label="$1" method="$2" path="$3" token="$4" body="$5"
  local body_file="$drill_dir/http-${label}.json"
  local status_file="$drill_dir/http-${label}.status"
  local -a args=(--silent --show-error --connect-timeout 2 --max-time 10
    --output "$body_file" --write-out '%{http_code}'
    --request "$method" "http://127.0.0.1:${port}${path}")
  if [[ -n "$token" ]]; then
    args+=(--header "Authorization: Bearer ${token}")
  fi
  if [[ -n "$body" ]]; then
    args+=(--header 'Content-Type: application/json' --data "$body")
  fi
  curl "${args[@]}" >"$status_file" || fail "$label request failed"
  echo "$label: HTTP $(<"$status_file")"
}

assert_json() {
  local file="$1" mode="$2"
  python3 - "$file" "$mode" <<'PY'
import json
import sys
from pathlib import Path

value = json.loads(Path(sys.argv[1]).read_text())
mode = sys.argv[2]
if mode == "work-search":
    if not isinstance(value, list) or not value:
        raise SystemExit("work search returned no evidence")
    if any(row.get("source_id") != "work-launch" for row in value):
        raise SystemExit("work search exposed a non-work document")
    if any("personal-secret" in json.dumps(row) for row in value):
        raise SystemExit("work search leaked personal content")
elif mode == "empty-search":
    if value != []:
        raise SystemExit("ACL-filtered personal search was not empty")
elif mode == "audit":
    raw = json.dumps(value)
    for forbidden in ("query-secret", "query-secret-rotated", "status-secret", "admin-secret", "launch phrase", "personal-secret"):
        if forbidden in raw:
            raise SystemExit(f"audit output leaked {forbidden}")
    if not isinstance(value, list) or not any(event.get("action") == "search" for event in value):
        raise SystemExit("audit output did not contain a search event")
else:
    raise SystemExit(f"unknown assertion mode: {mode}")
PY
}

data_dir="$drill_dir/data"
config="$drill_dir/config.toml"
secrets="$drill_dir/secrets.env"
fixture="$drill_dir/fixture.jsonl"
export CORTANA_CONFIG="$config"

if [[ -n "$requested_port" ]]; then
  port="$requested_port"
else
  port="$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
fi
[[ "$port" =~ ^[0-9]+$ && "$port" -ge 1024 && "$port" -le 65535 ]] || {
  echo "CORTANA_AUTH_PORT must be an unused TCP port between 1024 and 65535" >&2
  exit 2
}

echo "==> Initializing disposable auth drill"
"$binary" --offline --config "$config" init --data-dir "$data_dir" >/dev/null
write_secrets "query-secret"
write_config

cat >"$fixture" <<'EOF'
{"source":"auth-drill","source_id":"work-launch","title":"Work launch note","content":"The launch phrase belongs to the work workspace.","project":"work","acl":["work"]}
{"source":"auth-drill","source_id":"personal-secret","title":"Personal note","content":"personal-secret must never be visible to the work agent.","project":"personal","acl":["personal"]}
EOF
"$binary" --offline --config "$config" ingest "$fixture" >/dev/null

echo "==> Starting loopback HTTP API on port $port"
"$binary" --offline --config "$config" serve --address "127.0.0.1:${port}" --no-web >"$server_log" 2>&1 &
server_pid=$!
for _ in $(seq 1 80); do
  if curl --silent --show-error --connect-timeout 1 --max-time 2 \
    "http://127.0.0.1:${port}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
kill -0 "$server_pid" 2>/dev/null || fail "Cortana HTTP API exited before readiness"

request health GET /healthz "" ""
assert_status health 200 "$drill_dir/http-health.status"
request status-anonymous GET /v1/status "" ""
assert_status status-anonymous 401 "$drill_dir/http-status-anonymous.status"
request status-query GET /v1/status query-secret ""
assert_status status-query 403 "$drill_dir/http-status-query.status"
request status-scoped GET /v1/status status-secret ""
assert_status status-scoped 200 "$drill_dir/http-status-scoped.status"
request audit-scoped GET /v1/audit status-secret ""
assert_status audit-scoped 403 "$drill_dir/http-audit-scoped.status"
request search-work POST /v1/search query-secret '{"query":"launch phrase","project":"work","limit":10}'
assert_status search-work 200 "$drill_dir/http-search-work.status"
assert_json "$drill_dir/http-search-work.json" work-search
request search-personal POST /v1/search query-secret '{"query":"personal-secret","project":"personal","limit":10}'
assert_status search-personal 200 "$drill_dir/http-search-personal.status"
assert_json "$drill_dir/http-search-personal.json" empty-search
request audit-admin GET /v1/audit?limit=100 admin-secret ""
assert_status audit-admin 200 "$drill_dir/http-audit-admin.status"
assert_json "$drill_dir/http-audit-admin.json" audit

echo "==> Rotating query principal through /v1/auth/reload"
write_secrets "query-secret-rotated"
request reload POST /v1/auth/reload admin-secret ""
assert_status reload 200 "$drill_dir/http-reload.status"
request old-query POST /v1/search query-secret '{"query":"launch phrase","project":"work","limit":10}'
assert_status old-query 401 "$drill_dir/http-old-query.status"
request rotated-query POST /v1/search query-secret-rotated '{"query":"launch phrase","project":"work","limit":10}'
assert_status rotated-query 200 "$drill_dir/http-rotated-query.status"
assert_json "$drill_dir/http-rotated-query.json" work-search
request rotated-audit GET /v1/audit?limit=100 admin-secret ""
assert_status rotated-audit 200 "$drill_dir/http-rotated-audit.status"
assert_json "$drill_dir/http-rotated-audit.json" audit

echo "Shared-agent authorization drill passed"
if [[ "$keep" == "1" ]]; then
  echo "  record: $drill_dir"
fi
