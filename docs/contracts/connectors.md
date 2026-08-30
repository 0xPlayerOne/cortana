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
  "metadata": { "mime_type": "text/plain" }
}
```

`source` and `source_id` are stable identity; title, URI, content, and metadata are replaceable
attributes. Connector output must be UTF-8, bounded per line, free of credential values, and
terminated by an explicit completion result. External-command connectors cannot write SQLite.
The reusable SDK contract also requires stable IDs, ACLs, progress counters, cursor/completeness,
cancellation and typed errors, deterministic configuration fingerprints, and a versioned manifest.

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
status is metadata-only. `python -m cortana.connectors certify external-reference` runs the offline
reference-adapter suite. Certification validates identity stability, ACL normalization, malformed
or duplicate rows, secrets, cursor/retry/cancel behavior, complete versus partial snapshots,
deletion safety, compatibility, and bounded document/line/total-byte use. An adapter cannot obtain
reconciliation authority unless the harness validates a complete successful snapshot.

Manifests declare contract/SDK/version, capabilities, package, dependencies/licenses, and one of
`supported`, `experimental`, `local-only`, or `rejected`. Compatibility is checked before a run.
Disabling or removing an adapter never implicitly deletes canonical evidence. External releases
need a security review and must repeat certification when the SDK, package, dependency, license, or
capability set changes.

## Slack and Discord certification

`python -m cortana.connectors certify slack` and `... certify discord` exercise disposable,
synthetic fixtures only. Both connectors remain disabled by default. The suite covers complete,
partial, cancelled, revoked-token, and rate-limited outcomes; only the complete fixture may
reconcile. Existing connector tests cover provider pagination, thread identity, cursor restart,
bounded retries, malformed histories, redaction-safe output, and cache behavior.

Certification does not authorize a production account, personal history, new OAuth scopes, or a
recurring schedule. Slack channel access depends on bot installation and workspace-admin policy;
Discord access depends on guild/channel permissions and its separate OAuth/client configuration.
History visibility may be incomplete because of provider retention, membership, or account tier.
A production enablement is a separate operator approval with account/source discovery, scope review,
and a bounded non-reconciling validation run.
