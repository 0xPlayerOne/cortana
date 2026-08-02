# Ingestion

Cortana treats every connector as a snapshot producer. Each successful run emits normalized
`Document` JSON Lines with stable source IDs. Cortana embeds only records whose searchable payload
changed, atomically replaces their chunks, and reconciles records that disappeared from the
completed snapshot. A failed connector never triggers deletion reconciliation.

Connector output is first captured in an owner-only on-disk spool and then ingested in bounded
batches. This preserves complete-snapshot reconciliation without holding a large Drive, Gmail, or
chat export in memory. Temporary spools are removed after success or failure, and reconciliation
uses a temporary SQLite key table rather than an unbounded SQL parameter list.
If one configured source fails, Cortana records the failure, continues syncing the remaining
sources, and exits nonzero after the run so supervisors still detect the partial failure.
Connector subprocesses also have a configurable wall-clock timeout (six hours by default), which
prevents a wedged upstream API from holding the cross-process sync lock indefinitely.

Every source also has a fail-closed safety preflight. The default ceiling is 2,000 documents,
128 MiB of searchable content, and 15 minutes per source. Filesystem preflight walks metadata
without reading file contents; connector preflight validates the completed owner-only spool before
the first embedding or index write. A source that exceeds any ceiling fails without reconciliation.
Set global defaults in `[ingestion]`, tighter source-specific `max_documents`, `max_bytes`, and
`max_duration_seconds` values on `[[sources]]`, or one-run overrides on the command line.
Ingestion uses one embedding request at a time by default even when interactive queries allow more
concurrency.

Configured source names are index namespaces. This prevents two Gmail accounts, Drive accounts, or
Slack workspaces from deleting or colliding with one another. The original adapter kind is retained
in metadata for provenance.

## Configure and run

Start from [`config.example.toml`](../config.example.toml), then run:

```bash
cargo run -- sync
cargo run -- sync --source personal-drive
cargo run -- sync --source work-code --no-reconcile
cargo run -- sync --source work-code --plan
cargo run -- sync --source work-code --max-documents 250 --max-bytes 33554432
```

`--no-reconcile` is useful for an intentionally partial external snapshot. Regular complete
snapshots should reconcile so removed source records do not remain searchable.
`--plan` never starts an external connector or opens the index. For filesystem sources it reports
the metadata-only document and byte scope; for remote connectors it reports the configured budgets
and marks inspection as deferred. An explicitly named disabled source can be planned safely before
it is enabled.

`SIGINT` and `SIGTERM` cancel an active source before reconciliation. In-flight connector
subprocesses are terminated, and embedding work is interrupted at a bounded polling interval.
Already committed incremental batches remain valid searchable data, but a cancelled or
budget-exceeded snapshot never deletes records from the prior complete snapshot.

The Python adapter process writes only normalized JSON Lines to stdout. Counts and diagnostics go
to stderr, which makes the boundary safe to pipe into `cortana ingest` or supervise independently.
Google Calendar preserves one-off events individually and compacts expanded occurrences into one
stable document per recurring series, including its occurrence count, date range, participants,
and provenance. This prevents daily meetings from consuming thousands of redundant embeddings
without discarding their long-term history.

Google Drive and Gmail keep owner-only derived caches under
`data_dir/connector-cache/<source>/`. Complete runs still list every item ID for correct deletion
reconciliation, but Drive content is downloaded only when its modification timestamp changes and
immutable Gmail message bodies are downloaded only once. The caches are disposable and can always
be rebuilt from Google. First-time Drive content and Gmail detail retrieval use bounded
eight-worker pools; cache writes and emitted documents remain ordered on the main connector
thread. Drive installs pypdf's AES support. A single malformed, inaccessible, or unsupported file
does not abort the source: Cortana retains a prior cached body when available, marks it
`content_stale` in metadata, and emits only the exception class in diagnostics.

Gmail tolerates an isolated message that disappears or becomes inaccessible between list and
detail requests. If more than 10% of an uncached page (with a minimum allowance of ten messages)
is denied, the connector fails the snapshot so Cortana cannot reconcile against a broad permission
failure.

Discord also keeps an owner-only derived cache. After the first complete channel snapshot,
scheduled runs request only messages after the newest cached snowflake. A complete refresh runs
daily to capture edits and deletions, while every emitted snapshot remains complete for safe
reconciliation.

## Credentials

- Slack and Discord tokens are read only from the configured environment-variable name.
- Google Drive, Gmail, and Calendar accept an OAuth token JSON path. Desktop authorization uses a
  Google **Desktop app** OAuth client JSON, Authorization Code + PKCE, a random loopback callback,
  and the minimum read-only scopes required by the Google sources that share that token. Refresh
  data is updated atomically and the token file is forced to mode `0600`. Existing token and OAuth
  client files must be regular, non-symlink files with owner-only permissions on Unix.
- A Google source may use `token_env` instead of `token` when the named environment value contains
  an absolute OAuth token JSON path. The Desktop editor stores that path value write-only in its
  managed secret file; it does not accept inline token JSON.
- Apple Notes uses the local macOS Notes automation permission and stores no credential.
- Buzz opens the retention database read-only.

Never place secret values in `config.toml`, logs, or the repository. Use a secret manager,
launchd/systemd environment file with restrictive permissions, or the host platform's secret
injection.

### Authorize Google sources

Create a Desktop app OAuth client in Google Cloud, enable only the APIs the selected sources need,
then configure both absolute paths:

```toml
[[sources]]
name = "personal-drive"
kind = "google-drive"
project = "personal"
token = "/Users/example/.config/cortana/google-personal-token.json"
oauth_client = "/Users/example/.config/cortana/google-desktop-client.json"
```

Save the source before choosing **Authorize** in Desktop, or run:

```bash
cortana authorize-google personal-drive
```

Cortana opens Google's consent page in the system browser and waits up to five minutes for the
loopback callback. The command never prints tokens. Sources sharing the same token file are
authorized together so the stored grant contains the union of their minimum read-only scopes.
Use separate token paths for different Google accounts or trust domains. The OAuth client file is
configuration, not a user token, but Cortana still rejects symlinks, broad permissions, and
oversized client files.
The token destination must be outside a filesystem source root.

Authorization does not validate, sync, embed, index, or reconcile the source. After consent,
run the bounded validation described below. Google may not return a new refresh token on a later
grant; Cortana preserves the existing refresh token in that case.

## Connector contract

Before the first sync of any configured source, validate it with explicit small bounds:

```bash
cortana validate-source SOURCE_NAME \
  --max-documents 25 \
  --max-bytes 10485760 \
  --max-seconds 60
```

Validation can target a disabled source by exact name. It fetches only that connector snapshot,
enforces the wall-clock and live stdout/stderr spool bounds, parses every emitted document, then
deletes the private spool. It never opens the index, embeds content, or reconciles records. The
latest metadata-only outcome is written atomically to the owner-only
`data_dir/source-validations.json` file so operators can distinguish a proven connector from one
that is merely configured. This record contains counts, limits, timestamps, and a bounded error
summary—never credentials, source content, or connector output.
Filesystem validation performs the same bounded preflight walk used by sync, so start with a
narrow root and conservative limits. Desktop can also run this read-only validation at one of the
guided initial-sync budget tiers (100, 500, or 2,000 documents with matching byte and duration
limits) so the resulting record covers a subsequent initial sync; the limits shown in
`source-validations.json` always reflect the validation that actually ran.

Desktop exposes a separately confirmed guarded trial sync after validation. It invokes the fixed
equivalent of:

```bash
cortana sync --source SOURCE \
  --require-validation --no-reconcile \
  --max-documents 25 --max-bytes 5242880 --max-seconds 300
```

`--require-validation` fails before opening the index or embedding provider unless the selected
source is enabled and its latest validation succeeded for the exact current source configuration
at equal or larger document and byte limits. The validation record stores only a one-way
configuration fingerprint. Trial sync may embed and index committed batches, but it never deletes
records absent from the bounded snapshot. Cancellation preserves already committed batches.

Desktop adds a guided initial sync on top of the same boundary for first-time ingestion. It offers
exactly three fixed budget tiers — 100 documents/25 MiB/15 minutes, 500/64 MiB/30 minutes, or
2,000/128 MiB/60 minutes — and the renderer can select only the tier enum, never raw flags or
numbers. The flow is plan-then-confirm: a read-only plan request resolves the saved source and
returns the exact limits that execution will enforce, and execution requires that plan plus an
explicit confirmation and a successful validation recorded at equal or larger limits (run
`validate-source` with the same budget, or use the Desktop **Validate for this budget** action).
Execution invokes the fixed equivalent of:

```bash
cortana sync --source SOURCE \
  --require-validation --no-reconcile \
  --max-documents 100 --max-bytes 26214400 --max-seconds 900
```

with the numbers of the selected tier. It runs under the same single-job lock with cancellation
and metadata-only audit events that include the tier, and it never escalates beyond the selected
budget. Desktop initial syncs and trial syncs never reconcile deletions; a complete CLI or
scheduled sync remains the reconciliation path.

External connectors must emit one JSON object per line:

```json
{
  "source": "upstream",
  "source_id": "stable-123",
  "title": "Example",
  "content": "Evidence body",
  "uri": "https://example.test/item/123",
  "updated_at": "2026-07-29T12:00:00Z",
  "project": "work",
  "acl": [],
  "metadata": {}
}
```

`source_id` must remain stable across runs. `content` must be plain searchable text. Put
provenance, channel/account identifiers, participants, and source-specific fields in `metadata`;
never place credentials there.

Set `acl` on a source to apply a default access label to every document that connector emits
without its own ACL. Empty document ACLs are public when the source also has no default. A
document with one or more labels is returned only to a query principal with at least one matching
label; the implicit loopback owner can access all labels. Use stable trust-domain labels such as
`personal`, `work`, or `shared`, not user-controlled channel names.

## Pre-embedded import

`cortana import-embeddings` accepts trusted JSON Lines for migrations from a compatible vector
store. Every record declares `embedding_fingerprint`, one normalized `document`, and one or more
`chunks` containing text and a vector. Cortana rejects the stream on the first fingerprint,
dimension, empty-chunk, or JSON mismatch. Valid vectors are also written to the persistent
embedding cache, allowing later native source syncs to reuse identical chunks.

The exporter terminates the stream with a completion record containing the exact document count.
Cortana requires that trailer before it reconciles each `(source, project)` represented by the
stream, so a broken or truncated pipe cannot delete an earlier snapshot. Pass
`--no-reconcile` only when intentionally importing a partial snapshot. Do not import vectors whose
model, dimensions, or preprocessing are uncertain; rebuild those records through normal ingestion.
