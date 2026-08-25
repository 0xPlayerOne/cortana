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

Memory content types are `semantic`, `episodic`, `procedural`, and `preference`. The public
`content_type` field is independent from `retention_tier` and `scope`. The existing public `kind`
field remains a compatibility projection: `working` means a semantic record with `working`
retention, while every durable record projects its content type (`semantic`, `episodic`,
`procedural`, or `preference`). Legacy rows migrate additively, so existing tombstones and meaning
are preserved and older clients can continue filtering by `kind`.

Retention tiers are `working` and `durable`. Scope values are `session`, `principal`, `workspace`,
and `owner-global`; the current authorization boundary still requires project and ACL checks for
every scope. Lifecycle operations are `observe` (candidate input),
`remember`/`retain` (approved canonical write), `recall` (read), `consolidate` (approved merge),
`reflect` (non-mutating derivation), `supersede`, and `forget`/`retract`.

## Observation candidates

`memory_candidates` is an isolated review queue for bounded proposals. A candidate has an
`observation_kind` (`harness-scratchpad`, `execution-event`, `evidence-backed`, or `user-authored`),
the same independent content/retention/scope axes as memory, a source and source id, explicit
provenance, confidence/importance, ACL, sensitivity, a dedupe hint, and a required expiry no more
than seven days away. Candidate content is capped at 8 KiB and provenance at 4 KiB; pending
proposals are capped at 1,000 per project. Sensitive or restricted proposals fail closed and are
reported as rejected without being stored. Candidate retries with an identical project/dedupe key
and payload are idempotent.

Candidates are never indexed by memory FTS, returned by `recall` or `context`, or counted in
`memory_revision`. They are visible only through the scoped candidate list path and may be
cancelled, expire automatically, or be redacted to a tombstone. Every create, rejection, list,
cancel, expiry, and redaction operation is audit-recorded with metadata only. ACL and owner-global
scope checks apply before candidate content is returned or changed; a candidate cannot cross its
principal or workspace boundary.

## Scope

Scope is one of `session`, `principal`, `workspace/project`, or explicitly approved owner-global.
The current store represents workspace/project scope in `project` plus ACL labels. Principal and
agent scope is enforced by the auth policy before recall or mutation. A selected workspace is a UI
filter, never authorization. Cross-workspace dedupe, cache, supersession, export, and revision paths
must remain isolated.

## Lifecycle rules

1. `remember` is explicit and idempotent for a `(project, dedupe_key)` pair.
2. A memory has independent content type, retention tier, scope, provenance, confidence,
   importance, valid-from/valid-until, ACL, and status.
3. `recall` excludes retracted, superseded, expired, or ACL-invisible records.
4. `reflect` is strictly non-mutating; it may propose candidates but cannot write them.
5. Consolidation, automatic retention, and contradiction resolution require an approved policy and
   preserve provenance. They are not enabled by retrieval alone.
6. `forget`/redaction removes the record from recall and increments `memory_revision`; audit data
   contains metadata only and never memory content.
7. Schema-axis migration is additive and idempotent. It does not increment `memory_revision`;
   revision changes only when a canonical memory record changes. Memory revisions invalidate
   dependent context/query caches, while source corpus revisions remain
   independent.

## Provider boundary

The built-in SQLite engine is the product default and the only required provider. External memory
providers are optional future adapters; they cannot become a hidden dependency or bypass Cortana's
ACL, audit, revision, export, or deletion contracts.
