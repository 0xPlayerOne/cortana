# Operations

Cortana is designed to run as a private per-user service. The default server binds only
`127.0.0.1:7331`; use a TLS-terminating reverse proxy for network access. A non-loopback bind is
refused unless both `--allow-remote` and `--api-token-env NAME` are provided. The workspace stores
that bearer token only in browser session storage.

## Health and telemetry

- `GET /healthz` is an unauthenticated process-liveness check.
- `GET /readyz` verifies both the SQLite index and a real embedding request.
- `GET /v1/status` reports source freshness, index counts, runtime counters, and cache telemetry.
- `POST /v1/answer` runs the bounded human-facing query pipeline.
- `GET /metrics` exports low-cardinality Prometheus metrics.

HTTP requests emit structured tracing spans to stderr. Set `RUST_LOG`, for example
`RUST_LOG=cortana=debug,tower_http=info`, to change verbosity. Request headers and evidence content
are never logged.

SQLite runs in WAL mode so retrieval remains available during connector syncs and compatible
embedding imports. Query-side cache hit counters and new query-vector cache writes are
best-effort while another process owns SQLite's writer lock; Cortana serves the retrieval result
instead of failing a request for cache telemetry. Canonical ingestion writes remain strict and
use SQLite's bounded busy timeout.

Planned answers use a persistent cache keyed by the query contract, corpus revision, embedding
fingerprint, model endpoint/name, scope, and retrieval limits. Any changed/deleted document or
changed source timestamp increments the corpus revision. `cache_ttl_seconds = 0` disables reads and
`cache_max_entries = 0` disables writes. `/v1/status` and `/metrics` expose answer counts, cache
entries, and cache hits without logging queries or evidence.

`/v1/status` also reports whether recurring ingestion is installed, the global and per-source
safety budgets, every configured source including disabled or not-yet-indexed sources, and the
latest persisted outcome for each source. Sync outcomes are recorded as `running`, `succeeded`,
`failed`, `cancelled`, or `budget_exceeded`. A process interruption intentionally leaves a
`running` record behind so the workspace can distinguish an interrupted run from a source that
never started. The workspace refreshes this status every 15 seconds and keeps query availability
separate from ingestion health. Cortana retains the newest 100 run records per source to keep this
operational history bounded.

Interactive query embeddings have a five-second latency budget. If the local or cloud embedding
queue is saturated or unavailable, HTTP and MCP retrieval immediately fall back to exact-term FTS
evidence; returned rows have no `semantic_rank`, and a warning records the degraded mode. Cached
query embeddings still provide normal hybrid retrieval without touching the provider.

Google Drive content is bounded to 50,000 characters per file by default. Oversized exports keep
equal head and tail samples plus `content_truncated` and `content_original_chars` metadata, avoiding
hours of low-value embedding work for multi-megabyte CSVs. Set `max_content_chars` on an individual
`google-drive` source when a different evidence budget is justified.

## Backup and recovery

`cortana backup` creates a consistent online SQLite snapshot with `VACUUM INTO`, runs a full
integrity check, and retains the newest 14 scheduled snapshots by default:

```bash
cortana --config ~/.config/cortana/config.toml backup
cortana --config ~/.config/cortana/config.toml verify /path/to/snapshot.sqlite3
```

Stop the server before recovery. Restore requires explicit confirmation and automatically preserves
the previous database as a verified `pre-restore-*.sqlite3` snapshot:

```bash
cortana service uninstall
cortana --config ~/.config/cortana/config.toml restore /path/to/snapshot.sqlite3 --force
cortana --config ~/.config/cortana/config.toml service install --web-dir /path/to/web
```

Test recovery periodically on a copy. A backup that has never been restored is not a proven
recovery path.

## macOS launchd

Build the release binary and workspace first, then install three per-user jobs: local embedding
supervision, the API/workspace, and daily verified backups. Recurring ingestion is intentionally
opt-in so installing or upgrading Cortana cannot unexpectedly start a large first sync.

```bash
cargo build --release
bun run build
./target/release/cortana --config ~/.config/cortana/config.toml service install \
  --web-dir ./apps/web/dist
./target/release/cortana service status
```

Use `--no-embedding-service` for a cloud embedding provider. Logs are written beneath
`data_dir/logs`. `service uninstall` stops and removes only Cortana's four launchd jobs; it does not
delete configuration, data, logs, or backups.

After planning each enabled source and choosing explicit budgets, opt in to the recurring job:

```bash
cortana --config ~/.config/cortana/config.toml sync --source SOURCE --plan
cortana --config ~/.config/cortana/config.toml service install \
  --web-dir /path/to/web --enable-sync-service
```

Re-running `service install` without `--enable-sync-service` removes any prior recurring sync job
and leaves Cortana in query-only mode.

The generated Qwen/TEI profile keeps `max-batch-tokens=512`, which was faster than larger batches
in the macOS Metal benchmark, and admits up to 128 queued inputs so background ingestion can share
the provider with interactive agents without avoidable 429 responses. Cortana itself sends at most
eight inputs per request and applies bounded retry/backoff for transient provider pressure.
Up to four ordered requests run concurrently by default; lower `request_concurrency` when a cloud
provider has a stricter rate limit.

## Linux systemd

Templates live in [`packaging/systemd`](../packaging/systemd). Install the binary, built workspace,
and user-unit files at the paths shown in the templates, then run:

```bash
systemctl --user daemon-reload
systemctl --user enable --now cortana-embedding.service cortana.service
systemctl --user enable --now cortana-sync.timer cortana-backup.timer
```

For cloud embeddings, omit `cortana-embedding.service`. Adjust `ReadWritePaths` when `data_dir`
differs from the XDG default.

## Read-only production readiness

Run `cortana readiness` to check API liveness, embedding availability, database integrity, backup
freshness, query mode, and recurring-sync state. The command does not call connectors or mutate
the corpus. Recurring sync fails the safe default unless the operator explicitly supplies
`--allow-sync-service`; see the [evaluation guide](evaluation.md).

## Secrets

An optional `[runtime].env_file` supplies connector, cloud-provider, and HTTP-token environment
variables without putting values in launchd or systemd definitions. On Unix, Cortana refuses to
read this file if any group or other permission bit is set. Use mode `0600`; process environment
variables take precedence.

For shared agents, configure one bearer principal per environment variable under `[[auth.tokens]]`.
`query`, `status`, and `admin` scopes are enforced independently. Document ACLs are public when
empty and otherwise require a matching principal label; `*` is reserved for the implicit local
owner and legacy single-token mode. Answer-cache keys include the sorted ACL labels, preventing
reuse across authorization boundaries. `GET /v1/audit` requires `admin` and returns at most 500
metadata-only events. Audit records contain principal, action, project/source scope, outcome,
result count, latency, and timestamp—never query text, evidence, bearer tokens, or token hashes.
HTTP clients send the token as a bearer credential. Stdio MCP clients pass only its environment
variable name with `cortana mcp --token-env NAME`; Cortana resolves the value privately, maps it to
the configured principal, and enforces the same scopes and ACLs. Omitting `--token-env` keeps the
MCP process in the unrestricted local-owner profile and must not be used for a shared agent.

Before adding the first shared principal, assign matching ACL defaults to every configured source
in that trust domain, then preview legacy rows:

```bash
cortana acl plan --project work=work --project personal=personal
```

The plan is read-only and reports configuration mismatches. After reviewing the exact counts,
`cortana acl apply ... --force` updates only empty/public ACL rows, increments the corpus revision
once, and leaves already restricted documents unchanged. Apply refuses to run when any configured
source in a mapped project has a different ACL, preventing the next sync from silently making the
rows public again. `cortana readiness` fails whenever shared token principals coexist with public
legacy rows.
