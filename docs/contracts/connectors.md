# Connector and reconciliation contract

Every connector is an adapter that produces a complete or explicitly incomplete snapshot. The
provider-neutral contract is `cortana.connector.v1`; source-specific OAuth, discovery, and API
quirks stay behind the adapter.

## Normalized input

The Rust core accepts one JSONL `Document` per line:

```json
{
  "source": "google-drive",
  "source_id": "opaque-provider-id",
  "title": "Example",
  "content": "…",
  "uri": "https://provider/item",
  "updated_at": "2026-01-01T00:00:00Z",
  "project": "work",
  "acl": ["work"],
  "metadata": {"mime_type": "text/plain"}
}
```

`source` and `source_id` are stable identity; title, URI, content, and metadata are replaceable
attributes. Connector output must be UTF-8, bounded per line, free of credential values, and
terminated by an explicit completion result. External-command connectors cannot write SQLite.

## Operation phases

Authorization and discovery are separate from validation and synchronization:

1. authorize or revoke the provider account;
2. discover account/source metadata without indexing content;
3. validate a bounded sample with zero canonical writes;
4. run a bounded snapshot with cursor, timeout, byte/document/spool/concurrency budgets;
5. retain a completed prefix for retry diagnostics;
6. reconcile only when the snapshot is complete, fresh, configuration-matched, and operator-approved.

Cancellation, timeout, provider error, malformed JSONL, budget exhaustion, sampling, or stale
configuration produces a non-reconciling run. A complete snapshot with zero documents is valid and
may reconcile deletions; a failed empty output never does.

## Status and certification

Every run reports source/project, phase, status, cursor presence, progress documents/bytes, budgets,
configuration fingerprint, started/completed timestamps, error class, and deletion count. Public
status is metadata-only. A future connector is certified by fixtures covering identity stability,
ACL normalization, malformed rows, duplicate rows, cursor restart, retry/cancel cleanup, complete
versus partial snapshots, deletion safety, and bounded resource use.
