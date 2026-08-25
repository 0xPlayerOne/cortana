# Bounded reflection contract

`cortana.reflection.v1` is the read-only reasoning contract over already
authorized native memory and, optionally, already authorized retrieval
evidence. It is intentionally separate from `remember`, candidate promotion,
and ingestion.

## Request

`ReflectRequest` contains:

- a bounded `objective`;
- one optional `project` (a non-owner request may not combine workspaces);
- independent memory filters for legacy kind, content type, retention tier,
  scope, and a bounded record limit;
- `include_evidence`, which requires evidence to have been retrieved for the
  same project;
- a bounded token budget and deadline;
- `provider_policy`: `deterministic-only`, `prefer-provider`, or
  `require-provider`.

The caller supplies `ReflectionInputs` from the existing scoped retrieval
paths, including principal ACL and the memory revision observed before the
call. The module rejects memories outside the principal ACL and owner-global
records for non-owners. Evidence uses the legacy evidence shape, which has no
ACL field, so the caller must provide its retrieval project and must not pass
unscoped evidence.

All limits are enforced before synthesis: objectives are 512 bytes, memory is
limited to 100 records, evidence to 50 chunks, the token budget to 256–8,192
tokens, and the deadline to 30 seconds. The response exposes only opaque
supporting IDs for evidence; source content and ACL labels are not echoed.

## Response and grounding

`ReflectResponse` includes a stable request digest, privacy-scope digest,
provider outcome, claims, patterns, tensions, chronology, recommendations,
evidence IDs, and proposed candidates. Every derived item carries supporting
memory IDs (and claims may carry evidence IDs). A provider response is rejected
if it references an unknown ID, emits an unsupported candidate, or returns an
ungrounded chronology entry.

The deterministic implementation is provider-free and extractive. It groups
visible memories by content type, detects bounded polarity tensions, orders
records chronologically, and may emit a proposed working candidate for review.
Proposed candidates always set `approval_required: true`; a transport must use
the explicit retain/remember flow for promotion.

Provider behavior is explicit:

- `deterministic-only` returns `completed`;
- `prefer-provider` returns `completed` when the provider succeeds, or
  `fallback` with a bounded failure/unavailable detail;
- `require-provider` returns `provider-unavailable` or `provider-failed` with
  no synthesized result.

Deadline exhaustion returns `deadline-exceeded` and clears partial derived
results. The pure reflection engine has no mutation capability; first-party
adapters may read through the scoped Store and retrieval paths but never write
canonical memory, create a candidate row, or advance `memory_revision`.
The response reports the unchanged observed revision and
`canonical_memory_mutated: false`.

HTTP (`POST /v1/memory/reflect`), MCP (`reflect_memory`), CLI
(`cortana memory reflect`), and the Desktop Reflect action call this contract
only after normal memory-scope authorization. They never turn a proposed
candidate into canonical memory implicitly.
