# Optional Honcho Session Sidecar

Cortana keeps its canonical evidence store, provenance, ACLs, and retrieval pipeline authoritative.
The Honcho adapter is an opt-in sink behind the same durable memory outbox used by Hindsight. It
does not run during normal ingestion, copy the corpus automatically, or change MCP/HTTP/UI
retrieval.

## Contract

`HonchoHttpProvider` uses Honcho's v3 API:

- retain: `POST /v3/workspaces/{workspace_id}/sessions/{session_id}/messages`
- delete: `DELETE /v3/workspaces/{workspace_id}/sessions/{session_id}`

The provider creates a deterministic session named
`{session_prefix}-{cortana_document_id}` for each retained document. This intentionally uses one
session per document because Honcho's deletion boundary is a session; deleting a queued Cortana
document therefore cannot remove a neighboring document's messages. Retain sends one message with
the document title/content and metadata containing the stable Cortana document ID, project, source,
source ID, ACL tags, and source metadata.

Message content is capped at 128,000 characters with a deterministic head/tail truncation marker.
Remote endpoints must use HTTPS; local HTTP is accepted only for loopback hosts. Tokens are sent
only as bearer headers and never appear in diagnostics or provider errors.

## Enablement boundary

Construct the provider only from an explicit operator-selected memory configuration and drain the
outbox deliberately. The adapter is not wired into the default Rust sync service or Desktop
settings yet. Before enabling it for personal data, run a versioned comparison against Cortana's
native `context` retrieval and verify retention, deletion, ACL, and export behavior. Until those
gates pass, use Cortana's MCP `context` tool as the agent memory interface and keep Honcho
disabled.

The implementation uses Cortana's existing `httpx` dependency; no hosted SDK is required.
