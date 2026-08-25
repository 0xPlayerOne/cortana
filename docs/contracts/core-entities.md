# Core entity and lifecycle contract

This document is the canonical contract for persisted Cortana entities. Rust models and SQLite
tables are the implementation of this contract; derived indexes, embeddings, graph edges, and
UI projections are rebuildable and never become a second source of truth.

## Contract identity

- **Entity contract:** `cortana.entity.v1`
- **Storage schema:** the SQLite schema version recorded by migrations
- **Identifier encoding:** lowercase URL-safe strings; IDs are opaque to clients
- **Time encoding:** RFC3339 UTC with millisecond precision for new writes
- **Revision encoding:** unsigned decimal counters, monotonically increasing per canonical domain

An incompatible field removal, semantic change, identifier change, or lifecycle change requires a
new contract version, a migration, and an ADR. Additive optional fields remain compatible when
unknown fields are ignored by older readers.

## Canonical records

| Entity | Required canonical fields | Lifecycle / notes |
| --- | --- | --- |
| `Workspace` | `id`, `name`, `owner_principal`, `created_at`, `updated_at`, `status` | `active`, `quarantined`, `deleted`; display name is not authorization |
| `Project` | `id`, `workspace_id`, `name`, `acl`, `created_at`, `updated_at`, `status` | Project is the persisted scope used by the current store; workspace mapping is explicit |
| `Source` | `id`, `name`, `kind`, `project_id`, `authorization_ref`, `config_fingerprint`, `status` | `configured`, `authorized`, `validated`, `enabled`, `disabled`, `revoked`, `quarantined` |
| `Document` | `id`, `source`, `source_id`, `project`, `title`, `content`, `uri`, `updated_at`, `content_hash`, `acl`, `metadata` | Upsert by `(source, source_id)`; disappearance is a tombstone/reconciliation decision, never an implicit delete |
| `Chunk` | `id`, `document_id`, `ordinal`, `content`, `content_hash` | Derived from a canonical document and rebuildable |
| `Evidence` | `chunk_id`, source identity, title, URI, content, score, ranks, `updated_at` | Query projection only; never independently mutated |
| `MemoryRecord` | `id`, `kind`, `project`, content/provenance, validity, status, ACL, revision fields | Explicit operational conclusion; see [memory contract](memory.md) |
| `Principal` | `id`, `kind`, credential reference, scopes, ACL, status | Credential values never enter the record or public payload |
| `SyncRun` | `id`, source/project, status, budgets, progress, timestamps, outcome counters | `running`, `succeeded`, `failed`, `cancelled`, `budget_exceeded`; only complete runs may reconcile |
| `ValidationRecord` | source/project, limits, config fingerprint, result, completed_at, evidence reference | Freshness and configuration match are required before reconciliation |
| `CorpusRevision` | `revision`, timestamp, cause | Incremented only when canonical documents/chunks change |
| `MemoryRevision` | `revision`, timestamp, cause | Incremented only when canonical memory changes |
| `EmbeddingFingerprint` | provider class, endpoint class, model, dimension, generation fingerprint | Identifies a vector generation; changing it invalidates derived caches |

## Identity and scope

The stable document identity is derived from the connector's source name and opaque source ID. A
connector must not use titles, paths, timestamps, or content as identity. Project and ACL values are
canonical fields, sorted and deduplicated before persistence. A read or write is authorized only
after principal ACL intersection; workspace selection alone never grants access.

## Lifecycle invariants

1. Canonical writes are transactional and revisioned.
2. Derived chunks, FTS rows, vectors, caches, graph edges, and UI projections may be deleted and
   rebuilt from canonical records.
3. Failed, sampled, cancelled, capped, or stale operations cannot create deletion authority.
4. Forget/redaction creates an auditable lifecycle transition; it does not silently rewrite source
   history.
5. Supersession points to an existing canonical record in the same authorized scope.
6. Expiry removes a memory from recall while retaining bounded audit/export history until retention
   policy removes it.

## Storage and migration policy

SQLite migrations are forward-only and idempotent. Every migration must:

- detect the prior schema safely;
- preserve canonical values or fail closed;
- update the schema marker transactionally;
- include a rollback/backup procedure in the owning issue;
- add a fixture covering both the pre-migration and post-migration shape.

No migration may require a live connector, a model provider, a credential, or a private absolute
path. The current `Store::corpus_revision`, `Store::memory_revision`, embedding fingerprint, and
bounded migrations implement these rules for the existing v1 store.
