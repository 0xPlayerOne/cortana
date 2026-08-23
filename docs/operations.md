# Operations

Cortana is designed to run as a private per-user service. The default server binds only
`127.0.0.1:7331`; use a TLS-terminating reverse proxy for network access. A non-loopback bind is
refused unless `--allow-remote` is paired with configured `[[auth.tokens]]` principals. The workspace stores
that bearer token only in browser session storage.

For a new installation, use the [Desktop-first download path](../README.md#desktop-first-launch-recommended)
or the CLI steps in the [README](../README.md#quick-start). Keep the runtime query-only until
readiness and a deliberately bounded source validation pass; this guide is the operator reference
for service state, backups, authentication, sync safety, and release evidence.

Always record the running version before diagnosing a package or service:

```bash
cortana --version
cortana service status --json
```

Compare that version with [Release history](releases.md). A source checkout can contain safety
hardening that is intentionally absent from the latest published package, so do not use a checkout
to certify an installed release.

## Operator quick path

Use this numbered path for the common lifecycle. It keeps the installation query-only until a
source action is explicitly approved and makes every destructive step recoverable.

1. **Install or update.** Download the matching package from the [latest verified
   release](releases.md), install it, then confirm `cortana --version` and run `cortana doctor`.
   To roll back a failed application update, stop Cortana services, reinstall the previously
   verified release archive, and run the readiness check below. Never replace an installed
   release with an unverified checkout.
2. **Operate safely.** Check `cortana service status --json`, `GET /healthz`, and
   `cortana readiness --max-backup-age-hours 48`. `/healthz` is a liveness-only probe; the
   lightweight HTTP `/readyz` provider check is also public on loopback but requires a scoped
   bearer token on non-loopback listeners. Keep recurring sync uninstalled unless the complete
   source-validation gate passes; a readiness failure is a stop condition.
3. **Back up before changes.** Run `cortana backup`, then verify the resulting snapshot with
   `cortana verify /path/to/snapshot.sqlite3`. Keep at least one verified snapshot outside the
   live data directory.
4. **Roll back data or configuration.** Stop and uninstall the per-user jobs, restore a verified
   snapshot with `cortana restore /path/to/snapshot.sqlite3 --force`, and reinstall the query-only
   services. Restore automatically preserves a verified `pre-restore-*.sqlite3` snapshot. Desktop
   settings import is preview-only until **Save changes**, which creates its rollback copy; do not
   edit the live TOML or secret files by hand.
5. **Uninstall without deleting data.** Run `cortana service uninstall`, then remove the Desktop
   application through the operating system's normal app removal flow. This removes Cortana's
   per-user jobs but preserves configuration, secrets, logs, backups, and the index. Take and
   verify a backup before separately removing those data directories.
6. **Exercise recovery.** Before relying on a backup, run the disposable
   `scripts/backup-restore-drill.sh` with `CORTANA_CONFIG` set. It restores into a temporary
   directory and never replaces the live index or starts a connector. Set `CORTANA_KEEP_DRILL=1`
   only when retaining the incident record is intentional.

## Health and telemetry

- `GET /healthz` is an unauthenticated process-liveness check.
- `GET /readyz` verifies both the SQLite index and a real embedding request. It is public on
  loopback, but requires a bearer principal with `status` scope on remote listeners.
- `GET /v1/status` reports source freshness, index counts, runtime counters, and cache telemetry
  through a bounded database-stats probe. If SQLite is temporarily contended after a successful
  snapshot, it returns the last ACL-scoped snapshot with `stats_stale=true` and an age so the
  Desktop can remain truthful without turning a transient read timeout into a blank dashboard;
  a first probe with no safe snapshot still fails closed instead of guessing.
- `POST /v1/answer` runs the bounded human-facing query pipeline.
- `GET /metrics` exports low-cardinality Prometheus metrics through the same bounded stats probe.
- MCP `brain_status` uses the same bounded stats probe so agent status requests fail closed rather
  than waiting indefinitely on a contended SQLite read.

`/healthz` only answers whether the process is alive; it does not perform the database or backup
integrity work used by the CLI's full readiness check. A recent read-only readiness run scanned
roughly 1 GB and took about 130 seconds for database integrity plus about 80 seconds for the backup
scan, so use `/healthz` for a quick liveness probe and `cortana readiness` when comprehensive
evidence is required.

Direct JSONL imports are also bounded: 2,000 documents, 128 MiB of content, 15 minutes, and an
8 MiB maximum line. Use separate reviewed batches for larger migrations.

On the current v0.34.30 source tree, mutating CLI startup acquires the same global `sync.lock`
before opening the store. This covers schema/backfill/fingerprint work as well as the later import
or sync operation. This is included in the published v0.34.30 source release.

Desktop settings and service-schedule saves use a shared owner-only per-config lock. The lock is
held across validation, secret/config or schedule backups, atomic replacement, and audit writing,
so concurrent Desktop windows or processes cannot lose updates or interleave credentials. This is
included in the published v0.34.30 source release; keep the same lock requirement when running a newer
source checkout or development build.

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
`[ingestion].validation_max_age_hours` bound (168 hours by default; `0` accepts any age only for
read-only/manual checks), so a
succeeded validation that has lapsed is reported as expired rather than healthy. The local owner
sees the complete inventory; scoped principals
see only matching source/project counters, workspaces, validation state, and sync outcomes; an
admin-scoped principal may inspect the complete operational view. Sync outcomes for an
ACL-visible configured source stay visible to that principal even when the source has not
indexed any documents yet, so a failed, interrupted, or budget-exceeded run is not hidden from
the principals allowed to view that source. The
authorization summary reports only the connector method (`none`, `token`, `google_oauth`, or
`github_oauth`) plus setup and authorization booleans; it never exposes token values, paths, OAuth
client paths, or environment contents. Validation proves bounded connector
access without mutating the corpus and is shown separately from synchronization health. Public
status exposes only a generic validation-failure marker; raw connector diagnostics remain in the
owner-local validation state and Desktop job log. Sync
outcomes are recorded as `running`, `succeeded`,
`failed`, `cancelled`, or `budget_exceeded`. A process interruption intentionally leaves a
`running` record behind so the workspace can distinguish an interrupted run from a source that
never started. Before a new sync starts, recovery only marks `running` rows as `cancelled` and
preserves any completed run status and outcome counters. While a run is active, the store also
persists bounded `progress_documents`, `progress_bytes`, and `progress_updated_at` counters after
each ingestion batch. These counters are resumable operational evidence only: they do not
authorize reconciliation, imply a complete snapshot, or expose connector payloads. The workspace
refreshes this
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

Requested trials retry only transient transport, timeout, rate-limit, and 5xx failures once by
default. `CORTANA_SOURCE_SMOKE_SYNC_ATTEMPTS=1` disables retries; values above `3` are rejected.
Credential and configuration failures remain fail-fast.

Credential failures are reported as `authorization denied` without exposing connector details;
this includes Google OAuth refresh failures such as `invalid_grant` and a `400` response from the
Google token endpoint. Re-authorize that source before enabling a recurring schedule.

### Native agentic memory

Native memory lives in the canonical SQLite store and is explicit-write only. Agents use `remember`
for bounded conclusions with provenance, `recall` for ACL-filtered retrieval, `forget` to redact
withdrawn memories, and `export` for a bounded operator-controlled snapshot. Working memories can
carry `valid_until` and are excluded automatically after expiry. The HTTP, MCP, and CLI `context`
surfaces automatically include matching native memories in a separate bounded section alongside
source evidence. `/v1/answer` follows the same scope boundary and includes memory context only for
principals with the `memory` scope; numbered source evidence remains the citation authority. Source
ingestion never bulk-copies documents into memory. Identical dedupe-key retries are no-ops, and
memory status reports distinguish valid active records from expired records retained for export.
The native store is the sole supported memory engine for this release. See
[Native agentic memory](memory.md) for the lifecycle and interface contract.

Run the disposable lifecycle drill when validating a new binary or local
installation:

\`\`\`bash
CORTANA_BINARY=/Users/amf/.local/bin/cortana scripts/native-memory-drill.sh
\`\`\`

The drill uses an offline temporary store and verifies idempotent deduplication,
expiry exclusion, bounded export, and redaction. It never reads the live index,
credentials, or configured source connectors; it is evidence for native-memory
behavior only, not for source authorization or recurring sync.

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
hours of low-value embedding work for multi-megabyte CSVs. PDFs larger than 32 pages also use a
bounded head/tail sample of at most 32 pages; they report `content_original_chars: null` because the omitted middle
was not parsed. Image-only or malformed PDFs are retained with `content_unavailable: true` and an
explicit original-item recovery message rather than aborting the rest of a strict listing. Other
unsupported binary Drive items receive the same metadata-only marker and remain linked to their
original item; Cortana does not claim to have extracted their contents. Set
`max_content_chars` on an individual `google-drive` source when a different
evidence budget is justified.
Unfiltered complete Drive snapshots also persist a provider changes cursor in the source cache;
subsequent runs apply additions, updates, trash/removal events, and shared-drive changes
atomically. Cursor expiry, account/workspace changes, and unavailable cursor support fall back to
a fresh complete listing without advancing a partial snapshot. Bounded or filtered trials never
advance this cursor.

## Backup and recovery

`cortana backup` creates a consistent online SQLite snapshot with `VACUUM INTO`, runs a full
integrity check, takes the global sync lock so ingestion cannot race the snapshot, and retains the
newest 14 scheduled snapshots by default:

```bash
cortana --config ~/.config/cortana/config.toml backup
cortana --config ~/.config/cortana/config.toml verify /path/to/snapshot.sqlite3
```

Stop the server before recovery. `restore` refuses to run while a Cortana sync holds the index
lock (the same `sync.lock` that guards backups), so a scheduled sync job cannot race the
replacement. Restore requires explicit confirmation and automatically preserves the previous
database as a verified `pre-restore-*.sqlite3` snapshot:

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

## Desktop control plane drill

`scripts/desktop-control-plane-drill.sh` verifies the offline CLI control plane end to end inside
one fresh `mktemp` directory: `init` of a temporary configuration and data directory, bounded
ingestion of a two-document JSONL fixture, bounded `search`/`context` retrieval, export of the
metadata-only audit trail, a verified `backup`, `init` of a second temporary data directory,
`restore` of the backup into it, and a final `verify` plus restored-content search. Assertions fail
the drill closed on any missing output, unexpected file, or leaked query/content in the audit
export.

```bash
scripts/desktop-control-plane-drill.sh
```

The drill is disposable and isolated: every invocation uses `--offline` with an explicit `--config`
(and an exported `CORTANA_CONFIG` fallback) pointing inside the drill directory, so the live
configuration and index are never read or mutated. It never starts the server, a connector, the
embedding service, a recurring service, or a sync, and default cleanup removes only the
freshly-created drill directory. Set `CORTANA_KEEP_DRILL=1` to retain the exact directory and its
`control-plane.log` for an incident record, and set `CORTANA_BINARY` when testing a checkout.

The drill proves the offline CLI control plane only; it is not a proof of the Desktop GUI, OAuth
flows, tray integration, or updater behavior, none of which it exercises.

The pre-upgrade v0.34.25 binary passed this drill on 2026-08-23. It created and searched the
disposable fixture, exported metadata-only audit output, verified a backup, restored it into a
second temporary data directory, and searched the restored index. The run never read or mutated
the live configuration, index, credentials, or source connectors.

The historical v0.32.12 host passed this drill on 2026-08-16. That installed-runtime result confirms
the disposable control-plane and recovery path; it does not authorize source synchronization or
replace native GUI, browser OAuth, tray, updater, or macOS trust acceptance.

## Shared-agent authorization drill

Use the disposable HTTP drill before onboarding a shared agent or changing its workspace ACL:

```bash
scripts/shared-agent-auth-drill.sh
```

The drill uses a fresh temporary config, private synthetic tokens, and two synthetic documents. It
proves query/status/admin scope separation, work-versus-personal ACL isolation, metadata-only audit
responses, and atomic token rotation/revocation. It is offline and non-destructive: it does not read
the live index, contact connectors, authorize accounts, enable recurring sync, or launch the Desktop
app. The historical v0.32.12 host passed this drill on 2026-08-16, including old-token rejection and
rotated-token acceptance after `/v1/auth/reload`. A successful drill is evidence for the HTTP contract
only; keep the MCP tests and packaged GUI,
browser OAuth, tray, native-dialog, updater, and operating-system trust gates separate.

## Approved-index evaluation

When a representative corpus has been explicitly approved for evaluation, use the read-only live
harness in `scripts/evaluate-live-index.py` with a private manifest copied from
`eval/live-manifest.example.json`. It calls only `/v1/search` and `/v1/answer`, bounds each request
to 60 seconds and the complete run to five minutes, and reports source IDs plus aggregate metrics
including retrieval/provider fallback rates without printing query text, answers, credentials, or
provider error bodies:

```bash
uv run python scripts/evaluate-live-index.py \
  /private/path/cortana-live-manifest.json \
  --base-url http://127.0.0.1:7331 \
  --require-synthesis
```

The flag does not change the running configuration; synthesis must already be enabled in the
approved evaluation environment. Omit it to measure the safe extractive path.

The current host has one recorded read-only retrieval run for the approved `work` /
`work-gmail` scope (2026-08-14): recall@k 1.0, MRR 1.0, hybrid retrieval, zero retrieval
degradation, zero forbidden-source leaks, 1.0 repeated-query cache-hit rate, and 1,750 ms
maximum latency. The private manifest and query text were not committed. Treat this as live
retrieval evidence only; answer/synthesis, full-budget source validation, shared-agent ACL, and
packaged-GUI gates remain independent.

A temporary synthesis-enabled retry against that same scope failed closed at its 45-second request
ceiling without a cited answer. No source leakage or provider-unavailable fallback was reported;
the result is retained as latency/failure evidence only. Keep production synthesis disabled until
a bounded provider-backed answer run passes.

For a shared agent, pass a scoped bearer token through `--token-env`. Run one manifest per ACL
principal/workspace and include forbidden IDs to test isolation. This harness is read-only: it does
not sync, reconcile, mutate the index, or test cache invalidation by editing corpus data. Keep the
deterministic fixture gate for invalidation and run this harness only after readiness is healthy.
Do not interpret a passing report as permission to install recurring sync; memory writes remain
explicit agent operations.

## Release verification

The current published release is `v0.34.30`; release-assets workflow `32625481582` and the strict
18-asset verifier passed. The audited host now runs v0.34.30, so host-install and personal-source
evidence below is explicitly evidence for that installed runtime.

Published releases have a final cross-platform asset gate. It checks that the core archives,
checksums, signed macOS/Linux/Windows desktop installers, and every updater platform entry are
present for the same tag before the release workflow succeeds. It also inspects the core archives
and macOS app archive for safe paths, executable runtime files, the expected
`ai.cortana.desktop` bundle identity/version in `Contents/Info.plist`, the connector resource, the
web bundle, the release installer scripts, and the published SHA-256 files against their downloaded
archives, then executes the published Linux core binary and asserts that its `--version` output
matches the release tag (on non-Linux hosts the executable check is skipped because the verifier
cannot run foreign-OS binaries).

The verifier decodes each Tauri base64-encoded `.sig` payload and cryptographically verifies the
published macOS, Linux, and Windows updater archives against the updater public key in
`apps/desktop/src-tauri/tauri.conf.json`. Signature verification is required by default and fails
closed when `minisign` is unavailable. The published-release workflow also sets
`CORTANA_REQUIRE_MINISIGN=1`; use the explicit `CORTANA_REQUIRE_MINISIGN=0` opt-out only for
offline fixture work, never for a release decision.

Desktop sidecar preparation is also single-writer on the current source tree: a bounded lock
serializes Cargo/build and publication, and the completed sidecar is staged and atomically renamed
into the bundle directory. A partially copied sidecar is never treated as a successful build. This
is separate from the published package's signature and GUI acceptance gates.

Each release-asset download uses a bounded three-attempt retry for transient transport failures;
the attempt budget is capped at five and the retry delay at 60 seconds. Set
`CORTANA_DOWNLOAD_RETRY_DELAY=0` for fast offline tests. Every attempt also has a hard timeout,
controlled by `CORTANA_DOWNLOAD_TIMEOUT_SECONDS` (1-600 seconds, 120 by default), so a wedged
GitHub CLI cannot stall the release gate indefinitely. Exhausting the budget still fails the
release gate, so missing or invalid assets are never hidden.

Historical v0.29.66–v0.29.69 evidence remains useful for archive and updater checks, but it is not
current-release proof. Those releases were version/changelog/lockfile updates over v0.29.65; the
installed v0.29.67 CLI was headless and passed its then-current doctor, disposable control-plane,
and configured-provider evaluation checks. Synthesis remains disabled by default in production.

The v0.30.0 release-assets workflow `31470374229` is historical evidence only: it published 14 of
18 assets because the Windows build exposed the missing non-Unix Discord RPC methods. The v0.30.1
release-assets workflow `31474156961` completed all five platform jobs. The v0.30.2, v0.30.7, and
v0.30.8 release-assets workflows are historical evidence. The historical v0.30.10 release-assets
workflow `31515684053` completed all five platform jobs; its strict verifier result is historical.
The historical `v0.31.0` release was published from tag commit `2d6ef86`; release-assets workflow
`31555962734` completed all five platform jobs and the strict verifier passed all 18 assets,
signatures, checksums, and updater-manifest checks. Its verified core reported v0.31.0 during that
historical installation; the packaged Desktop GUI was not launched or replaced on this host. The
bounded control-plane,
recovery, and model-backed checks are recorded in `docs/desktop-ux-audit.md`.

The fully verified follow-ups are v0.31.1 and v0.31.2. Release-assets workflow
`31559861575` completed all platform jobs and
`scripts/verify-desktop-release.sh v0.31.2` verified all 18 assets. Release
v0.31.3 was published from the protected promotion in PR #819. The preceding
v0.31.5 patch was promoted through PR #845 and Release Please PR #846;
release-assets workflow `31575709770` completed all platform jobs plus the
strict 18-asset verifier. Release v0.31.6 was then published through evidence
promotion PR #851 and Release Please PR #854; workflow `31578434124` completed
all platform jobs and the strict 18-asset verifier passed. Release v0.31.7 was
published through Release Please PR #895; release-assets workflow `31597160527`
completed all platform jobs and the strict v0.31.7 verifier passed. Release
v0.31.8 was then published through Release Please PR #956; release-assets
workflow `31667577467` completed all platform jobs and the strict v0.31.8
verifier passed. Release v0.31.9 was subsequently published after exact-tree
promotion PR #960 and reconciliation PR #962. Release v0.31.10 was then
published through Release Please PR #967. Release v0.31.11 followed through
Release Please PR #979; release-assets workflow `31675820099` completed all
platform jobs and the strict 18-asset verifier passed. The then-installed core was
v0.31.7 and its
doctor, query-only readiness, and disposable control-plane checks pass without
starting services or sync. The published v0.31.11 core archive's persistent-provider
fixture model gate passed on 2026-08-13 in 18,755 ms (the earlier v0.31.10-configured
run passed in 24,491 ms; the published v0.31.8 archive's earlier 14,879 ms and
the installed v0.31.7 14,258 ms, 15,542 ms, 22,015 ms, 18,270 ms, 10,083 ms, 17,734 ms, 20,027 ms,
13,416 ms, 17,145 ms, and 12,613 ms runs
and prior v0.31.6 runs passed in 25,789 ms, 21,409 ms, 15,728 ms, and 22,269 ms; the v0.31.5
runtime-baseline run passed in
13,871 ms; prior verified runs passed
in 19,524 ms, 15,774 ms, and
13,237 ms) with planner/synthesis, citations, cache reuse, and revision
invalidation. No packaged-GUI evaluation is claimed.

Release v0.31.12 followed through Release Please PR #1030; its release-assets
workflow `31699076439` and strict 18-asset verifier are historical evidence.
The provider-backed 12,319 ms fixture result above is historical installed-core
evidence. No packaged-GUI evaluation is claimed.

Release v0.31.13 followed through Release Please PR #1124; its release-assets
workflow `31764807122` and strict verifier are historical evidence.

Release v0.31.14 followed through Release Please PR #1130; release-assets
workflow `31767490416` completed all platform jobs and the strict 18-asset
verifier passed, including the packaged-core offline evaluator. The verified
v0.31.14 archive was checked without replacing the local binary or restarting
services. No packaged-GUI evaluation is claimed.

The then-installed v0.31.12 binary also passed the disposable offline control-plane
drill (bounded ingest, hybrid retrieval/context, metadata-only audit, verified
backup, restore, SQLite verify, and post-restore search). The v0.31.14 release
verifier's packaged-core gate is historical evidence; neither check
exercises the packaged GUI, browser OAuth, tray events, native dialogs, or signed updater.

Release v0.31.16 followed through Release Please PR #1163 and the protected staging
reconciliation. Release-assets workflow `31783540306` completed all platform jobs and the strict
18-asset verifier passed. The installed v0.31.16 core passed query-only readiness and the
provider-backed fixture evaluator in 14,660 ms. An explicit `readiness --allow-sync-service`
check failed closed on bounded/incomplete source validation, so recurring sync remains
uninstalled; no packaged-GUI evaluation is claimed.

The v0.31.16 package includes the hardening described above: direct ingestion and
source validation share the global `sync.lock`, and remote `/readyz` requests
require a bearer principal with `status` scope while `/healthz` remains public
liveness. Keep this release evidence separate from the still-open source,
shared-agent, native memory, and native GUI acceptance gates.

To verify a published macOS package without launching its GUI, run the static
package smoke check on macOS. It selects the host architecture automatically;
use `CORTANA_MAC_ARCH=arm64` or `CORTANA_MAC_ARCH=x86_64` to override it when
cross-checking a release. The release must contain the matching architecture's
app archive; the verifier fails explicitly when it does not.
The macOS verifier also verifies the published Tauri signature before extraction. Signature
verification is mandatory by default; set `CORTANA_REQUIRE_MINISIGN=0` only for offline fixture
work where `minisign` is intentionally unavailable.

```bash
  GH_REPO=0xPlayerOne/cortana bun run desktop:verify:mac v0.34.28
```

It checks the bundle version, executes only the bundled core's `--version`
command, and performs strict code-sign verification. Gatekeeper rejection is
reported as an expected Developer ID/notarization gap unless
`CORTANA_REQUIRE_GATEKEEPER=1` is set; the variable must be `0` or `1`. This command does not exercise OAuth,
tray, native dialogs, updater installation, or other GUI behavior.

The 2026-08-12 validation-only `scripts/source-smoke.sh` run is historical: it
passed the 21 sources enabled at that time at one document, 65,536 bytes, and 30
seconds per source. It performed no embedding, indexing, reconciliation, or
scheduler changes and cannot authorize the current source inventory or recurring
full-corpus sync.

minisign verification covers the Tauri updater archives only and fails closed in CI. The packaged
macOS Desktop app passes `codesign --verify --deep --strict` but remains ad-hoc signed (no Developer
ID notarization), so `spctl --assess` still rejects it and notarization remains a release blocker.

The current source release verifiers also execute the exact packaged `cortana` core's deterministic
`--offline eval` against a temporary configuration, with a hard 60-second timeout and a required JSON
`passed: true`. The v0.34.28 verifier recorded this packaged-core gate in addition to the
archive/signature/checksum/updater-manifest checks in the release-assets workflow.
The new check is credential-free; it does not open the live index, launch the GUI, exercise
OAuth/tray/dialog/updater interactions, or authorize ingestion.

On 2026-08-14 the installed v0.32.1 CLI also passed
`scripts/desktop-control-plane-drill.sh`. The disposable drill initialized a temporary index,
ingested two bounded documents, exercised hybrid search/context and metadata-only audit export,
created and verified a backup, restored into a second temporary data directory, verified SQLite,
and searched the restored index. It never touched the live index, credentials, configured sources,
or service scheduler; it is control-plane/recovery evidence only and not packaged GUI/OAuth/tray/
native-dialog/updater acceptance.

### Current local source rollout snapshot (2026-08-23; published and installed v0.34.30)

The operator installation is still manual/query-only (`ai.cortana.sync` is not installed). The
source-validation records below include a pre-upgrade v0.34.13 pass at the safe 25-document/5 MiB/
60-second bound plus a v0.34.15 Personal Gmail retry at the same scope with a 120-second cap. Ten of
13 enabled non-code profiles are now `complete=true`: all Apple Notes scopes, all Work Google scopes,
Personal Drive, Personal Gmail, Personal Calendar, and Buzz. The three Special Google scopes
(`special-drive`, `special-gmail`, and `special-calendar`) failed closed because the shared
`special.json` OAuth grant returned `invalid_grant`. These records do not prove any configured
production budget and make no index or reconciliation writes.

Personal Drive's earlier 1,800-second and 900-second validations failed closed at their connector
deadlines while processing a large PDF/media corpus. After explicit reauthorization, the current
bounded probe succeeded at 25 documents/5 MiB/60 seconds; the next production-budget run was
operator-cancelled after 147 documents when serialized Drive body fetching stalled on a large PDF.
Both cancelled attempts made zero index or reconciliation writes. The published v0.34.28 release
boundary includes bounded four-worker fetching from PR #1594; the
`readiness --allow-sync-service` gate must remain closed until every enabled source has a fresh
complete record at its configured budget and the Special Google grant is repaired. No
reconciliation or large sync has been run.

A fresh provider-backed `cortana eval --model` run on the installed v0.34.28 binary passed planner
and synthesis execution, valid citations, cache reuse, and revision invalidation in 15,279 ms under
the 55,000 ms bound, without provider fallback. This is synthetic fixture evidence only; it neither
authorizes source sync nor establishes personal-index quality, and it does not replace the separate
packaged GUI and signing gates. The approved-corpus provider gate remains open and the evaluator
remains opt-in.

### Historical rollout observations

Earlier bounded and production-budget runs are retained below for incident and recovery context;
they do not override the current records in `source-validations.json`. Discord and all code roots
are disabled by operator choice, and Slack remains an optional, unconfigured connector.

The historical bounded live pass on 2026-08-15 rechecked every enabled non-code source with
`--no-reconcile --require-validation` and 25-document/5 MiB/60-second caps. Work, Personal, and
Special Apple Notes, Drive, Gmail, and Calendar all completed; Special Calendar returned zero
records; Buzz completed 25 records. Every run reported zero deletions. The index reached 12,123
documents and 42,638 chunks. Query-only readiness passed and `readiness --allow-sync-service`
failed closed for the then-current bounded records. These are historical bounded,
non-reconciling observations only; recurring sync remains uninstalled. The current host status is
tracked above in the release/evaluation evidence: 10 of 13 enabled sources now have fresh bounded
validation, while the three Special Google sources require reauthorization and Personal Drive
remains below its configured production budget.

The v0.34.5 source uses the local embedding `/health` endpoint for steady-state
liveness and keep the real vector probe for startup/restart. The installed v0.32.4 Work Drive retry
completed a 100-document bounded no-reconcile trial with `changed=0` and `deleted=0` after the
transport-retry path recovered the local embedding connection. This is a successful bounded trial,
not a complete 478-document production trial; no source or recurring sync is authorized by it.

The earlier 2026-08-15 Work Drive retry reached the complete 478-record connector snapshot but then
failed closed when the embedding connection closed. It used `--no-reconcile`, so it performed no
deletions; the controlled importer may retain only its completed prefix. The supervisor restarted
the local router and query-only readiness passed after recovery. The subsequent v0.32.4 100-document
bounded retry completed successfully, but recurring sync remains uninstalled until a complete,
successful production-budget trial and every other enabled source meet the source gate.

Do not infer recurring-sync readiness from this snapshot. Re-run `readiness --allow-sync-service`
after the next source-scoped validation pass; it must remain fail-closed until every enabled source
has a fresh, complete record at its configured budget.

Before opening a protected promotion PR, run `uv run python scripts/check-docs-consistency.py` (or
`bun run docs:check`). It treats the `## Current release` heading in `docs/releases.md` as the
documentation boundary and verifies the bounded current sections in `README.md`, `docs/README.md`,
`docs/getting-started.md`, `docs/project-goal.md`, `docs/releases.md`, `docs/evaluation.md`,
`docs/desktop-ux-audit.md`, and `docs/operations.md`. Historical evidence may continue to mention
older releases, but current entry points must not silently drift.

Re-run the read-only verifier for the current release with:

```bash
GH_REPO=0xPlayerOne/cortana CORTANA_REQUIRE_MINISIGN=1 scripts/verify-desktop-release.sh v0.34.28
```

For historical incident investigation, the v0.29.69 release can still be verified
independently:

```bash
scripts/verify-release.sh cortana-v0.29.69-aarch64-apple-darwin.tar.gz
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
Desktop service install and start/stop/restart commands use a bounded five-minute cold-start budget
so a local embedding model can warm without being reported as a false failure; a genuine timeout
still terminates the isolated helper process and remains visible as a retryable error.
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
`[ingestion].validation_max_age_hours` (168 hours by default; `0` is rejected for recurring sync):
re-run
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
picks up the new validation record automatically. `/v1/status` marks a lapsed validation expired
so the workspace flags the source for re-validation instead of showing it as healthy.

Re-running `service install` without `--enable-sync-service` removes any prior recurring sync job
and leaves Cortana in query-only mode.

The generated Qwen/TEI profile keeps `max-batch-tokens=512`, which was faster than larger batches
in the macOS Metal benchmark, and admits up to 128 queued inputs so background ingestion can share
the provider with interactive agents without avoidable 429 responses. Cortana itself sends at most
eight inputs per request and applies bounded retry/backoff for transient provider pressure.
Up to four requests run concurrently by default; lower `request_concurrency` when a cloud provider
has a stricter rate limit. Completed documents are persisted immediately, so cancellation or a
duration budget leaves a resumable tail instead of discarding the whole in-flight batch.

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
newer file when an older verified snapshot is still within the configured age bound. Integrity and
backup probes run on dedicated blocking threads and use the explicit `--storage-timeout-seconds` bound
(1 to 300 seconds, 240 by default); a timeout or worker failure fails readiness closed. A generation
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
`[ingestion].validation_max_age_hours` (168 hours by default; `0` disables the bound for
read-only/manual checks). Recurring sync rejects `0`. The installed recurring job re-checks the
same gate on every scheduled run, so an operator who changed a source after installing the sync schedule
sees the mismatch in `cortana readiness` before the next scheduled run fails fast with the same
reason. Budget failures include the validated and required document/byte/second values so the
next validation can be sized without inspecting the raw validation-state file. Without the flag,
source validation is not required for query-only readiness; per-source
validation state remains visible in `/v1/status` at any time.

## Secrets

An optional `[runtime].env_file` supplies connector, cloud-provider, and HTTP-token environment
variables without putting values in launchd or systemd definitions. On Unix, Cortana refuses to
read this file if any group or other permission bit is set. Relative paths are resolved from the
directory containing `config.toml`, so service working directories do not change which secrets are
loaded. Use mode `0600`. Connector and provider settings use process-environment precedence; bearer
policies prefer the private file so a stable `token_env` can be rotated without restarting the
service or inheriting a stale process value.

For shared agents, configure one bearer principal per environment variable under `[[auth.tokens]]`.
`query`, `status`, and `admin` scopes are enforced independently. New source records inherit their
workspace ACL when no explicit source ACL is configured; existing empty-ACL rows are treated as
legacy public data until migrated. Restricted documents require a matching principal label; `*` is
reserved for the implicit local owner. Answer-cache keys include the sorted ACL labels, preventing
reuse across authorization boundaries. `GET /v1/audit` requires `admin` and returns at most 500
metadata-only events. Audit records contain principal, action, project/source scope, outcome,
result count, latency, and timestamp—never query text, evidence, bearer tokens, or token hashes.
HTTP clients send the token as a bearer credential. Stdio MCP clients pass only its environment
variable name with `cortana mcp --token-env NAME`; Cortana resolves the value privately, maps it to
the configured principal, and enforces the same scopes and ACLs. Omitting `--token-env` keeps the
MCP process in the unrestricted local-owner profile and must not be used for a shared agent.

### Rotate a shared-agent token

The HTTP service can atomically reload its complete bearer policy without dropping requests. The
stdio MCP process rereads a stable file-backed principal for each tool call. Replace that principal's
value in the private env file, then reload; changing a process-only variable or its name requires
reconnect.

1. Add the new secret value through **Settings → Access** (or the owner-only `secrets.env` file)
   and keep the old principal unchanged.
2. Keep the existing principal's least-privilege scopes and ACL labels unchanged while the new
   value is written to the same `token_env` in the private env file.
3. Verify one bounded `status` or `context` request using the new token and confirm the audit event
   has the expected principal and scope. Do not put either token in shell history or a request body.
4. Remove the old principal and secret, save, and call the owner/admin-only HTTP reload endpoint:

   ```bash
   curl -sS -X POST http://127.0.0.1:7331/v1/auth/reload \
     -H "Authorization: Bearer $CORTANA_ADMIN_AGENT_TOKEN"
   ```

   A failed verification can be rolled back by restoring the previous principal from the local
   configuration backup; token rotation never changes the canonical index. The active HTTP policy
   swaps atomically, so the old token is rejected immediately after a successful reload. Existing
   MCP clients using the private env file load the same policy on their next tool call; process-only
   clients must reconnect. Use the Desktop restart action only when the managed service set is
   intentionally being restarted together.

The reload endpoint refuses to remove the last bearer policy from a non-loopback listener. It also
preserves the last-good policy when the TOML, environment file, or token values are malformed or
unreadable. Reload failures are audited only as metadata and never include parser diagnostics,
secret values, or query content.

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
cortana acl plan --project work=work --project personal=personal \
  --project special=special --quarantine-unmapped
```

The plan is read-only and reports configuration mismatches. After reviewing the exact counts,
`cortana acl apply ... --quarantine-unmapped --force` updates only empty/public ACL rows, assigning
explicit mappings to known workspaces and the reserved `__quarantine__` label to every other
public project. It increments the corpus revision once and leaves already restricted documents
unchanged. Apply refuses to run when any configured source in a mapped project has a different ACL,
preventing the next sync from silently making rows public again. Review the plan output before
applying; quarantined records remain owner-visible but are unavailable to scoped agents until
explicitly mapped. `cortana readiness` fails whenever shared token principals coexist with public
legacy rows.
