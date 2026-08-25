# Security and trust model

This is the canonical `cortana.security.v1` contract for the local Cortana brain. It records durable
controls and residual risks; current rollout status and evidence remain in GitHub issues and release
records.

## Assets and principals

Assets include source credentials and tokens, connector authorization state, canonical documents and
memories, embeddings and caches, SQLite databases and backups, audit metadata, release artifacts,
update manifests, Desktop settings, and provider responses. Principals are the local owner, a
configured bearer agent, a Desktop renderer, a connector subprocess, and a future hosted operator.

## Trust boundaries

1. External provider → authorization/discovery adapter.
2. Connector subprocess → bounded JSONL spool and Rust validator.
3. Rust core → canonical SQLite store and derived indexes.
4. HTTP/MCP/CLI → auth policy, ACL intersection, and typed public envelopes.
5. Tauri renderer → narrow native commands and loopback bearer credentials.
6. Backup/update/release artifact → integrity and restore verification.
7. Future remote host → TLS/bearer, tenancy, operator, and deletion boundaries not implied by local mode.

## Threats and controls

| Threat                         | Required control                                                                         | Current evidence boundary  |
| ------------------------------ | ---------------------------------------------------------------------------------------- | -------------------------- |
| Prompt injection in evidence   | Treat indexed text as data; synthesis cites only authorized evidence                     | Retrieval/answer tests     |
| Cross-workspace leakage        | ACL intersection before query, context, memory, cache, export, and status serialization  | Rust auth/API/MCP tests    |
| Credential exfiltration        | Secrets remain in owner-controlled env/keychain/private files; public payloads omit them | Auth and payload tests     |
| Unsafe remote exposure         | Loopback default; remote bind requires bearer policy and explicit config                 | readiness/API tests        |
| Incomplete reconciliation      | Only fresh, complete, config-matched snapshots reconcile; partial runs never delete      | ingestion/store tests      |
| Symlink/path escape            | Reject symlinked connector paths and bound spool reads/writes                            | connector validation tests |
| Malicious provider response    | Bound response bytes, timeouts, redirects, citations, and fallback                       | embedding/query tests      |
| Supply-chain/update compromise | Pinned actions/runtime, checksums, updater signatures, package verification              | release workflows          |
| Destructive restore/forget     | Explicit operator action, verified backup, metadata audit, revision invalidation         | operations/memory tests    |

## Authorization matrix

| Operation                                         | Query | Status | Memory | Admin |
| ------------------------------------------------- | ----: | -----: | -----: | ----: |
| Search/evidence                                   |   yes |     no |     no |   yes |
| Context without memory                            |   yes |     no |     no |   yes |
| Context with memory                               |   yes |     no |    yes |   yes |
| Remember/recall/forget/export memory              |    no |     no |    yes |   yes |
| Source/status/readiness                           |    no |    yes |     no |   yes |
| Auth reload, audit, settings, restore, scheduling |    no |     no |     no |   yes |

Workspace selection is not a permission. A principal must carry both the operation scope and an ACL
that intersects the requested project/source. Failed authorization returns a stable denial without
revealing whether a hidden record exists.

## Data handling and incident response

Audit events contain principal, operation, scope labels, outcome, bounded result count, and latency;
they do not contain query text, document content, memory content, tokens, or private absolute paths.
Backups are owner-only files and must pass verify/restore drills before being trusted. A suspected
credential compromise requires revoke/rotate, auth reload, audit review, backup review, and a clean
release/restore decision. Hosted deployment requires a separate tenancy and incident ADR.

## Residual risks

Local malware with owner privileges, compromised OS/keychain, malicious authorized sources, and
unnotarized packages remain outside the local application boundary. Provider-backed synthesis,
recurring sync, automatic memory formation, and remote hosting remain explicit activation gates.
