#!/usr/bin/env bash
set -euo pipefail

for program in docker curl grep; do
  command -v "$program" >/dev/null || {
    echo "required program is missing: $program" >&2
    exit 1
  }
done

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${CORTANA_CONFORMANCE_IMAGE:-cortana:conformance}"
run_id="${CORTANA_CONFORMANCE_RUN_ID:-$$}"
container="cortana-provider-conformance-$run_id"
data_volume="cortana_provider_data_$run_id"
backup_volume="cortana_provider_backups_$run_id"
token="synthetic-provider-conformance-token"
port=""

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    docker logs --tail 100 "$container" >&2 2>/dev/null || true
  fi
  docker container rm --force "$container" >/dev/null 2>&1 || true
  docker volume rm "$data_volume" "$backup_volume" >/dev/null 2>&1 || true
  return "$status"
}
trap cleanup EXIT INT TERM

docker run --detach \
  --name "$container" \
  --publish 127.0.0.1::7331 \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --pids-limit 256 \
  --memory 2g \
  --cpus 2 \
  --env "CORTANA_OWNER_TOKEN=$token" \
  --mount "type=volume,source=$data_volume,target=/var/lib/cortana" \
  --mount "type=volume,source=$backup_volume,target=/var/lib/cortana/backups" \
  --mount "type=bind,source=$repo_dir/deploy/self-hosted/config.toml,target=/etc/cortana/config.toml,readonly" \
  "$image" \
  --offline --config /etc/cortana/config.toml serve \
  --address 0.0.0.0:7331 --web-dir /opt/cortana/web --allow-remote >/dev/null

port="$(docker port "$container" 7331/tcp | sed -E 's/.*:([0-9]+)$/\1/')"
test -n "$port"
base_url="http://127.0.0.1:$port"

for _attempt in $(seq 1 30); do
  if curl --fail --silent "$base_url/healthz" >/dev/null; then
    break
  fi
  sleep 1
done
curl --fail --silent "$base_url/healthz" >/dev/null

unauthorized_status="$(
  curl --silent --output /dev/null --write-out '%{http_code}' \
    "$base_url/v1/provider/capabilities"
)"
test "$unauthorized_status" = "401"

capabilities="$(
  curl --fail --silent \
    --header "Authorization: Bearer $token" \
    "$base_url/v1/provider/capabilities"
)"
grep -q '"contract_version":"cortana.provider.v1"' <<<"$capabilities"
grep -q '"direct_local"' <<<"$capabilities"
grep -q '"remote_broker"' <<<"$capabilities"

memory_payload='{"kind":"episodic","project":"work","title":"Provider restart fixture","content":"Synthetic provider state survives restart","source":"provider-conformance","source_id":"restart-1","dedupe_key":"provider-restart-1","acl":["work"],"provenance":{"fixture":"self_hosted_single_node"}}'
first_write="$(
  curl --fail --silent \
    --header "Authorization: Bearer $token" \
    --header 'Content-Type: application/json' \
    --data "$memory_payload" \
    "$base_url/v1/memory"
)"
second_write="$(
  curl --fail --silent \
    --header "Authorization: Bearer $token" \
    --header 'Content-Type: application/json' \
    --data "$memory_payload" \
    "$base_url/v1/memory"
)"
test "$first_write" = "$second_write"

context="$(
  curl --fail --silent \
    --header "Authorization: Bearer $token" \
    --header 'Content-Type: application/json' \
    --data '{"query":"provider restart fixture","project":"work","limit":5,"max_tokens":512}' \
    "$base_url/v1/context"
)"
grep -q '"contract_version":"cortana.context.v1"' <<<"$context"
grep -q 'Synthetic provider state survives restart' <<<"$context"

docker restart --timeout 20 "$container" >/dev/null
port="$(docker port "$container" 7331/tcp | sed -E 's/.*:([0-9]+)$/\1/')"
test -n "$port"
base_url="http://127.0.0.1:$port"
for _attempt in $(seq 1 30); do
  if curl --fail --silent "$base_url/healthz" >/dev/null; then
    break
  fi
  sleep 1
done
curl --fail --silent "$base_url/healthz" >/dev/null

recalled="$(
  curl --fail --silent \
    --header "Authorization: Bearer $token" \
    --header 'Content-Type: application/json' \
    --data '{"query":"provider state restart","project":"work","limit":5}' \
    "$base_url/v1/memory/recall"
)"
grep -q 'Synthetic provider state survives restart' <<<"$recalled"

docker exec "$container" cortana --offline --config /etc/cortana/config.toml backup --keep 3 \
  | grep -q 'backup verified:'
docker exec "$container" cortana --offline --config /etc/cortana/config.toml verify \
  | grep -q 'database verified:'
docker exec "$container" id | grep -q 'uid=10001(cortana)'

echo "Self-hosted provider conformance passed"
