# Public interface and compatibility contract

HTTP, MCP, CLI JSON, and Desktop client fixtures are projections of the same public contract:
`cortana.api.v1`. They may use transport-specific envelopes, but shared operations expose the same
semantic fields, authorization behavior, limits, revisions, and degradation states.

## Shared rules

- Queries are non-empty and bounded to 16 KiB; scopes, IDs, cursors, and response bytes are bounded.
- Success responses carry `contract_version`, capability/degradation information where relevant,
  revision/fingerprint metadata, and an audit correlation ID when a transport supports headers.
- Errors carry a stable `code`, safe `message`, `retryable`, `contract_version`, and optional
  correlation ID. They never disclose credentials, hidden-record existence, private paths, or raw
  provider responses.
- Pagination uses opaque cursors tied to scope and revision. A stale cursor is rejected rather than
  silently mixing revisions.
- Additive fields are compatible; changing required fields, semantics, route/tool names, or CLI
  exit behavior requires a new contract version and migration notes.

## Surface mapping

| Operation                | HTTP                                                                                 | MCP                                                                              | CLI                                    | Canonical result                          |
| ------------------------ | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- | -------------------------------------- | ----------------------------------------- |
| Search                   | `POST /v1/search`                                                                    | `search`                                                                         | `search --json`                        | evidence list + retrieval metadata        |
| Context                  | `POST /v1/context`                                                                   | `context`                                                                        | `context --json`                       | [ContextBundle](context-bundle.md)        |
| Memory recall            | `POST /v1/memory/recall`                                                             | `memory_recall`                                                                  | `memory recall --json`                 | scoped memory records                     |
| Memory candidate         | `POST/GET /v1/memory/candidates`, `GET .../export`                                   | `propose_memory_candidate`, `list_memory_candidates`, `export_memory_candidates` | `memory candidate propose/list/export` | bounded, review-only proposals            |
| Candidate classification | `POST /v1/memory/candidates/{id}/classify`                                           | `classify_memory_candidate`                                                      | `memory candidate classify`            | deterministic, review-only recommendation |
| Memory reflection        | `POST /v1/memory/reflect`                                                            | `reflect_memory`                                                                 | `memory reflect`                       | bounded, grounded, non-mutating reasoning |
| Derived memory           | `GET /v1/memory/derived`                                                             | `inspect_memory_representations`                                                 | —                                      | versioned, non-canonical projections      |
| Candidate consolidation  | `POST /v1/memory/candidates/{id}/consolidate`                                        | `consolidate_memory_candidate`                                                   | `memory candidate consolidate`         | versioned policy decision and outcome     |
| Candidate review edit    | `POST /v1/memory/candidates/{id}/edit`                                               | —                                                                                | —                                      | validated pending candidate only          |
| Candidate working tier   | `POST /v1/memory/candidates/{id}/working`                                            | —                                                                                | —                                      | validated pending candidate only          |
| Candidate retry          | `POST /v1/memory/candidates/{id}/retry`                                              | —                                                                                | —                                      | explicit dead-letter requeue              |
| Consolidation control    | `POST /v1/memory/consolidation/pause\|resume`, `GET /v1/memory/consolidation/status` | —                                                                                | —                                      | readable state; owner-only mutation       |
| Status                   | `GET /v1/status`                                                                     | `brain_status`                                                                   | `status --json`                        | bounded health/store/source metadata      |
| Audit                    | `GET /v1/audit`                                                                      | `audit`                                                                          | `audit export`                         | metadata-only audit records               |

MCP schemas and generated Desktop TypeScript fixtures must be derived from these fields without
importing private Rust/store internals. Provider-backed answers remain opt-in and must preserve the
same evidence, memory, revision, and degradation semantics.

Desktop commands for candidate lists, classification/actions, consolidation control, canonical
export, and derived inspection are fixed-path projections of these HTTP operations. They select a
memory-scoped owner credential, validate candidate ids and bounded query inputs, and do not accept
arbitrary backend paths. UI confirmation supplements these controls; it never replaces HTTP ACL,
lifecycle, idempotency, or versioned-policy enforcement.
