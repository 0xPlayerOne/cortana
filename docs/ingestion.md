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

Configured source names are index namespaces. This prevents two Gmail accounts, Drive accounts, or
Slack workspaces from deleting or colliding with one another. The original adapter kind is retained
in metadata for provenance.

## Configure and run

Start from [`config.example.toml`](../config.example.toml), then run:

```bash
cargo run -- sync
cargo run -- sync --source personal-drive
cargo run -- sync --source work-code --no-reconcile
```

`--no-reconcile` is useful for an intentionally partial external snapshot. Regular complete
snapshots should reconcile so removed source records do not remain searchable.

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
- Google Drive, Gmail, and Calendar accept an OAuth token JSON path. Refresh data is updated
  atomically and the token file is forced to mode `0600`.
- Apple Notes uses the local macOS Notes automation permission and stores no credential.
- Buzz opens the retention database read-only.

Never place secret values in `config.toml`, logs, or the repository. Use a secret manager,
launchd/systemd environment file with restrictive permissions, or the host platform's secret
injection.

## Connector contract

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
