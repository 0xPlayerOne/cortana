# Ingestion

Cortana treats every connector as a snapshot producer. Each successful run emits normalized
`Document` JSON Lines with stable source IDs. Cortana embeds only records whose searchable payload
changed, atomically replaces their chunks, and reconciles records that disappeared from the
completed snapshot. A failed connector never triggers deletion reconciliation.

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

## Credentials

- Slack and Discord tokens are read only from the configured environment-variable name.
- Google Drive and Gmail accept an OAuth token JSON path. Refresh data is updated atomically and
  the token file is forced to mode `0600`.
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
