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

The metadata-only audit trail is retained to the configured `[auth].audit_max_events` bound (10,000
by default). It records principal, action, scope, outcome, result count, and latency, never query
text, document content, credentials, or bearer tokens. Administrators can export the retained
window for incident review without exposing it through a scoped agent principal:

```bash
cortana --config ~/.config/cortana/config.toml audit export ./audit.jsonl
cortana --config ~/.config/cortana/config.toml audit export --format json ./audit.json --force
```

Exports default to newline-delimited JSON and refuse to overwrite an existing file unless `--force`
is supplied. Destination files are created with owner-only permissions on Unix. Treat project and
source labels as operationally sensitive and store exports under the same retention policy as other
incident records. The HTTP `GET /v1/audit` endpoint remains capped at 500 rows for interactive
inspection.

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
safety budgets, every ACL-visible configured source including disabled or not-yet-indexed sources,
its configured ACL labels, a non-secret authorization summary, and the latest persisted validation
and sync outcomes for each source. Each per-source validation summary reports a `fresh` flag and a
bounded `age_seconds` computed against the configured
`[ingestion].validation_max_age_hours` bound (168 hours by default; `0` accepts any age), so a
succeeded validation that has lapsed is reported as expired rather than healthy. The local owner
sees the complete inventory; scoped principals
see only matching source/project counters, workspaces, validation state, and sync outcomes; an
admin-scoped principal may inspect the complete operational view. Sync outcomes for an
ACL-visible configured source stay visible to that principal even when the source has not
indexed any documents yet, so a failed, interrupted, or budget-exceeded run is not hidden from
the principals allowed to view that source. The
authorization summary reports only the connector method (`none`, `token`, or
(`google_oauth` or `github_oauth`) plus setup and authorization booleans; it never exposes token values, paths, OAuth
client paths, or environment contents. Validation proves bounded connector
access without mutating the corpus and is shown separately from synchronization health. Public
status exposes only a generic validation-failure marker; raw connector diagnostics remain in the
owner-local validation state and Desktop job log. Sync
outcomes are recorded as `running`, `succeeded`,
`failed`, `cancelled`, or `budget_exceeded`. A process interruption intentionally leaves a
`running` record behind so the workspace can distinguish an interrupted run from a source that
never started. Before a new sync starts, recovery only marks `running` rows as `cancelled` and
preserves any completed run status and outcome counters. The workspace refreshes this
status every 15 seconds and keeps query availability separate from ingestion health. Cortana retains the newest 100 run records per source to keep this
operational history bounded. Runtime request counters in a scoped status response are maintained
per authenticated principal; only the local owner or an admin-scoped principal receives the
process-wide totals used for operations dashboards.

`[ingestion].sync_freshness_hours` (48 hours by default) is a separate operational bound for a
successful sync. The API exposes it in `/v1/status`, and the workspace marks a successful run as
stale when its completion is older than that window. Set it to `0` only when a source is managed
outside Cortana's scheduler and stale-sync warnings are intentionally disabled; this does not
disable source-validation freshness or the recurring-sync safety gate.

### Bounded source smoke checks

Use the checked-in smoke harness when authorizing a new machine or verifying a credential rotation:

```bash
scripts/source-smoke.sh --config "$HOME/.config/cortana/config.toml"
```

It parses source names and connector kinds with Python's standard TOML reader, then runs each
selected `validate-source` within a positive document, byte, and wall-clock budget. Filesystem/code
validations pass `--sample`, so an oversized root is recorded as a bounded sample instead of
failing; connector sources keep ordinary fail-closed validation. It never enables
a source, installs a recurring job, writes indexed data, or prints token values. Pass source names
to limit the check, or `--include-disabled` to validate configured sources that are currently off.

After the read-only checks pass, `--sync` adds a deliberately bounded trial for connector sources,
and for filesystem/code sources when `--include-filesystem` is also supplied. Trials always use
`--no-reconcile --require-validation` with the same budgets as the validation, so a filesystem
trial can rely on the matching sampled validation while a partial snapshot can never delete
existing records or authorize a full-corpus sync.
The summary is a tab-separated operational result and the command exits nonzero if any validation
or requested trial fails:

```bash
scripts/source-smoke.sh --sync --max-documents 25 --max-bytes 5242880 --max-seconds 60
```

Interactive query embeddings have a five-second latency budget. If the local or cloud embedding
queue is saturated or unavailable, HTTP and MCP retrieval immediately fall back to exact-term FTS
evidence; returned rows have no `semantic_rank`. The HTTP search response keeps its evidence-array
shape and adds `x-cortana-retrieval-mode` plus `x-cortana-retrieval-degraded` headers. Context and
answer responses include the retrieval mode and warning, while MCP context includes the same
metadata and `brain_status` exposes a fallback counter. The fallback is also recorded as a
degraded audit outcome and in `cortana_retrieval_fallbacks_total`; provider error details remain
local logs only. Cached query embeddings still provide normal hybrid retrieval without touching the
provider.

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

### Disposable recovery drill

The checked-in drill snapshots the configured index, restores it into a new temporary data
directory, and verifies SQLite integrity. It never replaces the live database and never starts a
connector or sync:

```bash
CORTANA_CONFIG="$HOME/.config/cortana/config.toml" \
  scripts/backup-restore-drill.sh
```

Set `CORTANA_BINARY` when testing a checkout, and set `CORTANA_KEEP_DRILL=1` to retain the exact
temporary directory and `recovery.log` for an incident record. The default cleanup removes only the
freshly-created drill directory. Do not point the drill configuration at the production data
directory and do not use `restore` on a live installation as part of a routine health check.

## Release verification

Published releases have a final cross-platform asset gate. It checks that the core archives,
checksums, signed macOS/Linux/Windows desktop installers, and every updater platform entry are
present for the same tag before the release workflow succeeds. It also inspects the core archives
and macOS app archive for safe paths, executable runtime files, the connector resource, the web
bundle, the release installer scripts, and the published SHA-256 files against their downloaded
archives. Re-run the read-only verifier for an existing release with:

```bash
GH_REPO=0xPlayerOne/cortana scripts/verify-desktop-release.sh v0.23.1
```

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
./target/release/cortana service status --json
```

Use `--no-embedding-service` for a cloud embedding provider. Logs are written beneath
`data_dir/logs`. `service uninstall` stops and removes only Cortana's four launchd jobs; it does not
delete configuration, data, logs, or backups.

Installed jobs can be controlled independently without rewriting their launchd definitions:

```bash
cortana service start server
cortana service stop embedding
cortana service restart backup
```

The fixed service IDs are `embedding`, `server`, `sync`, and `backup`. Start and restart refuse an
uninstalled job. Stop unloads the job but preserves its plist so it can be started again. The
Desktop Services panel uses this same fixed command boundary, shows loaded state, PID, and last
exit status, and records metadata-only action audits. Core-service installation remains query-only;
the separate **Enable recurring sync** action requires confirmation and the same validation gate as
the CLI before it installs the sync schedule.
Desktop-at-login is a separate setting: enabling it starts the tray/control plane, not ingestion.
Desktop **Start all**, **Stop all**, and **Restart all** operate only on the embedding and server
jobs. Sync and backup are deliberately excluded from those aggregate actions.

Desktop keeps its scheduler intervals in the owner-only `service-schedule.toml` beside the active
configuration. The Services panel validates sync intervals from 60 seconds to 7 days and backup
intervals from 5 minutes to 30 days. **Save schedule** only persists the values; **Enable recurring
sync** is still the separate confirmation-gated action and passes the saved intervals to the
bundled runtime. If a sync job is already installed, saving a changed schedule exposes an explicit
**Apply recurring sync schedule** action; the running job keeps its previous interval until that
action is confirmed. The redacted portable settings export intentionally omits this machine-local
scheduler file.

After planning each enabled source and choosing explicit budgets, opt in to the recurring job:

```bash
cortana --config ~/.config/cortana/config.toml sync --source SOURCE --plan
cortana --config ~/.config/cortana/config.toml service install \
  --web-dir /path/to/web --enable-sync-service
```

The installer re-checks every enabled source before scheduling recurring sync and refuses to
install the job unless each source has a current successful validation covering its configured
document, byte, and duration budgets. Because the recurring job reconciles the full corpus, a
bounded sample recorded by `validate-source --sample` never satisfies this gate; only a complete
validation qualifies. A validation stays current for
`[ingestion].validation_max_age_hours` (168 hours by default; `0` accepts any age): re-run
`validate-source` (or the Desktop validation flow) after changing a source or its budgets, and
re-validate periodically so a revoked credential or changed scope cannot keep a stale record
blessing the schedule. The installed job runs `sync --require-validation` without `--source`, so
every scheduled run re-applies the same gate before any connector is contacted: a source whose
validation lapsed or failed, whose configuration changed since validation, whose resolved
budgets grew past the validated ones, or whose only validation was a sampled one fails the run
fast (nonzero exit, visible in the job log)
instead of ingesting against a stale validation. The same freshness bound gates
`sync --require-validation` and the readiness `source-validation` check, and both require a
complete validation for reconciling runs; a non-reconciling run (`--no-reconcile`) may rely on a
matching successful sample instead. Re-run `validate-source`
(or the Desktop validation flow) after changing a source or its budgets; the next scheduled run
picks up the new validation record automatically.
blessing the schedule. The installed job runs `sync --require-validation` without `--source`, so
every scheduled run re-applies the same gate before any connector is contacted: a source whose
validation lapsed or failed, whose configuration changed since validation, or whose resolved
budgets grew past the validated ones fails the run fast (nonzero exit, visible in the job log)
instead of ingesting against a stale validation. The same freshness bound gates
`sync --require-validation` and the readiness `source-validation` check. Re-run `validate-source`
(or the Desktop validation flow) after changing a source or its budgets; the next scheduled run
picks up the new validation record automatically. `/v1/status` marks a lapsed validation expired
so the workspace flags the source for re-validation instead of showing it as healthy.

Re-running `service install` without `--enable-sync-service` removes any prior recurring sync job
and leaves Cortana in query-only mode.

The generated Qwen/TEI profile keeps `max-batch-tokens=512`, which was faster than larger batches
in the macOS Metal benchmark, and admits up to 128 queued inputs so background ingestion can share
the provider with interactive agents without avoidable 429 responses. Cortana itself sends at most
eight inputs per request and applies bounded retry/backoff for transient provider pressure.
Up to four ordered requests run concurrently by default; lower `request_concurrency` when a cloud
provider has a stricter rate limit.

## Linux systemd

The core `service install` command and Cortana Desktop generate per-user systemd units under
`~/.config/systemd/user`, so no root access is required. Install the bundled query-only services
with:

```bash
cortana --config ~/.config/cortana/config.toml service install --no-web
```

Then inspect or control them through the same fixed service IDs used on macOS:

```bash
cortana service status --json
cortana service start server
cortana service stop embedding
```

The checked-in templates in [`packaging/systemd`](../packaging/systemd) remain useful for manual
package-manager installs and hardened deployments. The generated units use the current executable,
config path, working directory, and data directory, and recurring sync remains disabled unless
`--enable-sync-service` is explicitly supplied. The recurring sync unit is generated with the
same `sync --require-validation` guard as the macOS and Windows jobs, so each scheduled run
re-checks every enabled source's validation before contacting a connector. For a cloud embedding
provider, pass
`--no-embedding-service`.

The generated user units can also be managed directly:

```bash
systemctl --user daemon-reload
systemctl --user enable --now cortana-embedding.service cortana.service
systemctl --user enable --now cortana-sync.timer cortana-backup.timer
```

For cloud embeddings, omit `cortana-embedding.service`. Adjust `ReadWritePaths` when `data_dir`
differs from the XDG default.

## Windows Task Scheduler

Cortana Desktop and `cortana service install` use per-user Windows Task Scheduler tasks. They do
not require administrator access and keep the same fixed service IDs (`embedding`, `server`, `sync`,
and `backup`). Core services start immediately after installation and again at the user's next logon; sync and backup use bounded minute
intervals derived from the saved schedule. The Desktop Services panel reports task state and last
run result and can start, stop, or restart an installed task. A cloud embedding provider omits the
embedding task, and recurring sync remains opt-in and validation-gated on every platform.

## Read-only production readiness

Run `cortana readiness` to check API liveness, embedding availability, embedding/index generation
compatibility, database integrity, verified backup freshness, query mode, and recurring-sync state.
Readiness runs SQLite integrity verification on the newest backup candidates and ignores a corrupt
newer file when an older verified snapshot is still within the configured age bound. A generation
mismatch is reported with both fingerprints and readiness never changes the existing index. If the
provider endpoint changed but the model, dimension, and vector space are known to be identical, an
operator can adopt the exact stored generation without re-embedding the corpus:

Desktop readiness also reports the local `text-embeddings-router` executable when the configured
embedding provider is local. On macOS, if Homebrew is already installed, the Settings installer
offers the `text-embeddings-inference` formula after explicit approval. The model weights
are fetched by the embedding runtime on its first start; installing the binary does not start a
service or run ingestion. Cloud embedding configurations intentionally do not require this local
runtime.

```bash
cortana migrate-embedding \
  --from 'Qwen/Qwen3-Embedding-0.6B:1024' \
  --force
```

The command takes the sync lock, verifies SQLite integrity, creates a verified recovery snapshot,
updates only the generation metadata, and clears derived embedding/query caches. It never calls a
connector or rewrites indexed documents. Do not use it when the model, dimension, or vector space
changed; rebuild or re-import vectors into a new generation instead. Recurring sync fails the safe
default unless the operator explicitly supplies `--allow-sync-service`; see the
[evaluation guide](evaluation.md).

For a real model, dimension, or preprocessing change, rebuild the stored vectors instead of
adopting the old generation metadata:

```bash
cortana rebuild-embeddings \
  --from 'Qwen/Qwen3-Embedding-0.6B:1024' \
  --force
```

The rebuild requires explicit confirmation, probes the target provider, takes the sync lock,
verifies the database, and creates a recovery snapshot. Replacement vectors are staged separately
and committed in one SQLite transaction only after every chunk has a valid vector. A provider
failure leaves the old generation and live vectors usable; retrying starts a fresh staged rebuild.
The command never contacts connectors or reconciles documents.

With `--allow-sync-service`, readiness also runs a `source-validation` check that verifies every
enabled source has a current successful validation at equal or larger document, byte, and duration
budgets than its configured limits. The check reads only the owner-local validation state and never
contacts a connector: it fails when a source was never validated, its last validation failed, its
configuration changed since validation (the validation record stores a configuration fingerprint),
its resolved budgets grew past the validated ones — for example after raising `[ingestion]`
defaults behind an override-less source — or its validation lapsed past
`[ingestion].validation_max_age_hours` (168 hours by default; `0` disables the bound). This mirrors
the install-time recurring-sync gate, and because the installed recurring job re-checks the same
gate on every scheduled run, an operator who changed a source after installing the sync schedule
sees the mismatch in `cortana readiness` before the next scheduled run fails fast with the same
reason. Without the flag, source validation is not required for query-only readiness; per-source
validation state remains visible in `/v1/status` at any time.

## Secrets

An optional `[runtime].env_file` supplies connector, cloud-provider, and HTTP-token environment
variables without putting values in launchd or systemd definitions. On Unix, Cortana refuses to
read this file if any group or other permission bit is set. Relative paths are resolved from the
directory containing `config.toml`, so service working directories do not change which secrets are
loaded. Use mode `0600`; process environment variables take precedence.

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

### Rotate a shared-agent token

Rotate credentials without interrupting an agent by using a new environment-variable name:

1. Add the new secret value through **Settings → Access** (or the owner-only `secrets.env` file)
   and keep the old principal unchanged.
2. Point a new principal at that variable with the same least-privilege scopes and ACL labels.
3. Verify one bounded `status` or `context` request using the new token and confirm the audit event
   has the expected principal and scope. Do not put either token in shell history or a request body.
4. Remove the old principal and secret, save, and restart only the affected API/MCP process. A
   failed verification can be rolled back by restoring the previous principal from the local
   configuration backup; token rotation never changes the canonical index.

Desktop removes secret values that are no longer referenced by any source, provider, or principal
when settings are saved. Keep the old principal until the new credential has been tested, then
export the bounded metadata-only audit trail for the rotation record.

Cortana Desktop can create and edit these principals from **Settings → Access**. Token values are
write-only and stored in the managed owner-only secret file. The native process selects a matching
credential by scope for its fixed loopback requests; the webview never receives the value.
**Settings → Audit** shows at most 100 runtime and 100 Desktop events per refresh.

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
