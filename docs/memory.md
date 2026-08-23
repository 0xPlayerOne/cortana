# Native agentic memory

Cortana is vertically integrated: the same private SQLite store owns both
source-backed knowledge and explicit agent memory. Documents and code remain
evidence; memories are small, deliberate conclusions that agents choose to
retain.

## Current release boundary

The current protected source and published package are `v0.34.30`. Native memory remains the only
supported memory engine: it is local, explicit-write, ACL-filtered, auditable, exportable, and
separate from source knowledge. External memory providers are not product dependencies.

## Memory model

Each memory has one of five types:

- `semantic` — a durable fact or relationship;
- `episodic` — an event, decision, or interaction;
- `procedural` — a repeatable workflow or preference for how work is done;
- `preference` — a stable user preference;
- `working` — short-lived task state that can be superseded or redacted.

Working memories may include an RFC3339 `valid_until` timestamp. Recall and answer
context automatically exclude expired records, so agents can keep short-lived task
state without a cleanup race. Durable facts should omit the expiry and be replaced
or forgotten explicitly when they change.

Every record carries a workspace/project, ACL, provenance, source and source
id, confidence, importance, timestamps, and a lifecycle status. Writes are
idempotent when an agent supplies a `dedupe_key`. Replacements atomically mark
the previous record `superseded`; forgetting redacts content and leaves only a
minimal tombstone for auditability.

## Agent contract

Agents should retrieve evidence and matching durable memories together with
`context`; use `recall` when memory-only results are needed. The context bundle
keeps memories in a separate, clearly labelled section so agents can use
operational context without presenting it as a source citation. They should
call `remember` only for an explicit, bounded conclusion and include provenance
in the same call. Never copy an entire email, note, transcript, or code file
into memory. Use `forget` when a user withdraws a memory.

The human-facing `/v1/answer` path follows the same contract for principals with
the `memory` scope: matching native memories are returned separately and may
help the synthesizer, while only indexed evidence can satisfy numbered citation
requirements. Query-only principals receive evidence-only answers. Memory writes
advance a dedicated revision so cached answers cannot retain retracted or stale
operational context.

The native MCP tools are:

- `remember` — write one bounded memory;
- `recall` — ACL-filtered prefix-aware recall with a precise all-term pass and a bounded natural-language fallback. Candidates are ranked locally by query coverage, lexical relevance, confidence, importance, freshness, and exact-vs-fallback match using Cortana's own store;
- `forget` — redact one memory;
- `context` — retrieve cited source evidence plus relevant native memory in a
  token-bounded bundle.
- `export` — export bounded native records, including redacted tombstones, for
  an operator-controlled backup or migration.

The equivalent CLI is:

```sh
cortana memory remember --kind preference --project work \
  --title "Release notes" \
  --content "Prefer concise release notes with explicit risks" \
  --dedupe-key work:release-notes
cortana memory remember --kind working --project work \
  --title "Current task" --content "Validate the release" \
  --valid-until "2026-08-16T18:00:00Z"
cortana memory recall "release notes" --project work
cortana memory export --project work --limit 10000 > work-memory.json
cortana memory forget MEMORY_ID
```

HTTP clients can use `POST /v1/memory`, `POST /v1/memory/recall`, and
`POST /v1/memory/forget`, or `GET /v1/memory/export`. Shared agents need the `memory` scope in addition to
their normal query/status scopes; ACLs are enforced before content is returned
or redacted.

## Operating boundaries

Memory is not an automatic mirror of ingestion. Source sync remains the
authority for world knowledge, while explicit memory writes are the authority
for agent conclusions. The store is local-first and protected by the operator's
filesystem policy, auditable, exportable through the scoped export and backup
paths, and bounded by content, provenance, ACL, and recall limits.

A retry with the same dedupe key and identical normalized payload is a true
no-op: it does not advance the memory revision, so answer-cache entries remain
reusable. `brain_status` reports active, expired, retracted, superseded, and
total records. Expired working memories remain exportable for audit and backup,
but are excluded from recall and active-capacity accounting.

Recall is deliberately local and bounded. SQLite FTS5 produces the candidate
set, then Cortana applies a stable salience score so a precise, recent memory
beats a weak one-term match even when the latter has a high importance value.
The score is returned as `relevance_score` for agent diagnostics; it is not a
confidence claim and does not override ACL, expiry, or lifecycle checks.

Dedupe keys and supersession targets are workspace-scoped: a memory in one project
cannot overwrite or supersede a memory in another project, including for the owner.
Retired records keep their dedupe keys reserved, so replacements use a new key and
cannot resurrect tombstones or create lifecycle cycles. This keeps work, personal,
and special operational context isolated even when agents reuse generic retry keys.

The supported product path keeps retention, deletion, ACL, and backup semantics
in one database and makes offline operation deterministic. Owner-local CLI
remember, recall, and forget commands also emit metadata-only audit events;
memory titles, content, queries, and identifiers never enter the audit trail.
