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
and `owner-global`. Workspace writes require project and ACL checks, and owner-global operations
require authenticated owner authorization even when ACL labels match. Session and principal are
reserved values that fail closed on writes until the canonical record stores their verified
identity binding. Lifecycle operations are `observe` (candidate input),
`remember`/`retain` (approved canonical write), `recall` (read), `consolidate` (approved merge),
`reflect` (non-mutating derivation), `supersede`, and `forget`/`retract`.

## Observation candidates

`memory_candidates` is an isolated review queue for bounded proposals. A candidate has an
`observation_kind` (`harness-scratchpad`, `execution-event`, `evidence-backed`, or `user-authored`),
the same independent content/retention/scope axes as memory, a source and source id, explicit
provenance, confidence/importance, ACL, sensitivity, a dedupe hint, and a required expiry no more
than seven days away. Candidate content is capped at 8 KiB and provenance at 4 KiB; pending
proposals are capped at 1,000 per project and 100 submissions per authenticated principal per
rolling hour. Sensitive or restricted proposals fail closed and are
reported as rejected without being stored. Candidate retries with an identical project/dedupe key
and payload are idempotent.

Candidates are never indexed by memory FTS, returned by `recall` or `context`, or counted in
`memory_revision`. They are visible only through bounded scoped list/export paths and may be
cancelled, expire automatically, or be redacted to a tombstone. Every create, rejection, list,
cancel, expiry, and redaction operation is audit-recorded with metadata only. ACL and owner-global
scope checks apply before candidate content is returned or changed; a candidate cannot cross its
principal or workspace boundary.

### Classification

`classify` is a deterministic, provider-independent, review-only comparison of one pending candidate with
visible canonical records having the same project, ACL, content type, retention tier, and scope.
It returns a traceable candidate id, supporting memory ids, explanation, confidence, proposed action,
and unresolved ambiguity. Results are one of `new`, `exact-duplicate`, `semantic-duplicate`,
`reinforcement`, `contradiction`, `supersession`, `temporary-working`, or `discard`.
Classification never writes canonical memory, advances `memory_revision`, resurrects tombstones, or
creates a supersession edge. Conflicting preferences, changed decisions, and low-confidence
ambiguity remain review-required. Cross-project records and invisible ACLs are not compared.
Retracted, superseded, expired, and not-yet-valid records are excluded. An optional model adapter
may refine the deterministic result; provider failure or invalid output returns a bounded,
review-required deterministic fallback without exposing provider errors.

Principal-scoped candidates persist the authenticated creator identity and are visible or mutable
only to that principal (or the owner). Session candidates fail closed until the transport supplies
a verified session binding. Creator identities support authorization and rate limiting but are not
serialized in candidate responses.

### Approval-aware consolidation

Promotion from `memory_candidates` is disabled by default and is controlled by the versioned
policy `cortana.memory.consolidation.v1`. The policy records confidence/importance thresholds,
queue and retry bounds, active-capacity limits, and working/durable retention ceilings. A
consolidation decision records only the candidate id, classification, policy version, decision,
reason code, priority, and expiry; explanations never copy candidate content into audit output.

The decision state is one of `auto-retain`, `approve`, `review`, `reject`, or `working`:

- only non-sensitive, in-scope, non-conflicting candidates above policy thresholds may
  `auto-retain`;
- explicit approval may retain a below-threshold candidate, subject to the same ACL, capacity,
  expiry, and canonical write invariants;
- sensitive, contradictory, low-confidence, and cross-scope candidates cannot auto-commit;
- `working` records are bounded by the working retention ceiling and never become durable by
  retrying the queue;
- rejected, cancelled, expired, and dead-letter candidates remain metadata-only lifecycle history.

Canonical promotion uses the same transactional remember path as explicit writes. The candidate
status, memory row, FTS row, revision increment, and consolidation job are committed atomically;
an identical policy/candidate retry is a no-op. Jobs are bounded, priority ordered, retry limited,
pausable, cancellable, and dead-lettered after the retry ceiling. Disabling consolidation does not
change explicit memory writes, recall, source retrieval, export, or deletion behavior.

## Scope

Scope is one of `session`, `principal`, `workspace/project`, or explicitly approved owner-global.
The current store represents workspace/project scope in `project` plus ACL labels. It does not yet
persist the principal or session identity needed to authorize those narrower scopes, so it rejects
those writes instead of approximating identity with ACL membership. A selected workspace is a UI
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
