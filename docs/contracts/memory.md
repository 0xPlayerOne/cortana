# Native memory taxonomy and lifecycle contract

The native engine is the vertically integrated operational-memory layer. It shares the canonical
SQLite boundary with source knowledge but is never a bulk transcript or alternate truth store. The
contract is `cortana.memory.v1`.

## Layers and vocabulary

- **World knowledge:** source-backed `Document`, `Chunk`, and `Evidence` records.
- **Operational memory:** explicit, bounded conclusions retained by an agent or owner.
- **Task scratchpad:** ephemeral harness/session state; it is not Cortana memory and is not
  recalled after its owning task unless explicitly remembered.
- **Derived context:** a `ContextBundle` projection; it is rebuildable and never canonical.

Memory content types are `semantic`, `episodic`, `procedural`, and `preference`. The existing public
`working` kind is retained for compatibility and is a working-retention record, not a fifth content
type. Future schema work separates `content_type` from `retention_tier` without silently rewriting
existing records.

Retention tiers are `working` and `durable`. Lifecycle operations are `observe` (candidate input),
`remember`/`retain` (approved canonical write), `recall` (read), `consolidate` (approved merge),
`reflect` (non-mutating derivation), `supersede`, and `forget`/`retract`.

## Scope

Scope is one of `session`, `principal`, `workspace/project`, or explicitly approved owner-global.
The current store represents workspace/project scope in `project` plus ACL labels. Principal and
agent scope is enforced by the auth policy before recall or mutation. A selected workspace is a UI
filter, never authorization. Cross-workspace dedupe, cache, supersession, export, and revision paths
must remain isolated.

## Lifecycle rules

1. `remember` is explicit and idempotent for a `(project, dedupe_key)` pair.
2. A memory has provenance, confidence, importance, valid-from/valid-until, ACL, and status.
3. `recall` excludes retracted, superseded, expired, or ACL-invisible records.
4. `reflect` is strictly non-mutating; it may propose candidates but cannot write them.
5. Consolidation, automatic retention, and contradiction resolution require an approved policy and
   preserve provenance. They are not enabled by retrieval alone.
6. `forget`/redaction removes the record from recall and increments `memory_revision`; audit data
   contains metadata only and never memory content.
7. Memory revisions invalidate dependent context/query caches, while source corpus revisions remain
   independent.

## Provider boundary

The built-in SQLite engine is the product default and the only required provider. External memory
providers are optional future adapters; they cannot become a hidden dependency or bypass Cortana's
ACL, audit, revision, export, or deletion contracts.
