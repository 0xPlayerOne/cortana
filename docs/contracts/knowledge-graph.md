# Knowledge graph contract

The canonical graph projection is `cortana.knowledge-graph.v1`. It is a bounded,
rebuildable index over authorized evidence, native memory, and accepted code intelligence. It is
never canonical evidence, memory, reconciliation state, or a prerequisite for exact document
access and retrieval.

## Node identity

Every node ID combines a typed namespace with a percent-encoded stable canonical key. Supported
types are `workspace`, `source`, `document`, `chunk`, `entity`, `memory`, `observation`,
`mental-model`, `repository`, `file`, and `symbol`. IDs do not derive from labels, list position,
layout coordinates, or mutable display names. A node retains its project, ACL, canonical record ID,
update time, lifecycle status, and content revision. The renderer may discard and rebuild nodes at
any time.

## Relationship identity and meaning

Supported relationship types are `contains`, `references`, `backlink`, `nearby`, `same-thread`,
`authored-by`, `mentions`, `temporal`, `semantically-related`, `contradicts`, `reinforces`,
`supersedes`, `observes`, `derives`, `depends-on`, `defines`, `calls`, and `imports`.

Each relationship carries:

- source and target typed node IDs;
- relationship type and origin (`explicit`, deterministic `derived`, or heuristic `inferred`);
- contract and derivation versions;
- confidence for every inferred relationship and no fabricated confidence on explicit metadata;
- update time, project, and the ACL intersection of all supporting records;
- canonical supporting record IDs and revision-bound invalidation keys;
- a human-readable explanation generated from those fields.

Semantic similarity is always `semantically-related`, inferred, confidence-bearing, and disabled by
default. It cannot imply causality, authorship, contradiction, or factual support. Contradiction,
reinforcement, supersession, observation, and mental-model relationships use the accepted native
memory contract; code edges use the accepted revision-aware code graph contract.

## Visibility and invalidation

Authorization happens before node or edge serialization. An edge ACL is the intersection of its
supporting records; traversal cannot union labels or reveal the existence, label, degree, or error
state of a hidden record. Missing and unauthorized targets are indistinguishable.

Canonical mutations invalidate edges by their recorded invalidation keys. A derivation-version
change invalidates only that derivation's projections. Rebuild writes a new bounded projection and
publishes it atomically; failure leaves canonical evidence and the preceding complete projection
unchanged. Deterministic input order produces the same deduplicated relationship set.

## Pagination and traversal

Root pages, search, grouped relationships, and neighborhoods use opaque keyset cursors bound to
the principal-visible ACL fingerprint, workspace/source filters, graph revision, sort order, node
and edge filters, and derivation configuration. A mismatch is a stale-cursor error rather than a
mixed snapshot.

Defaults are 50 rows, one hop, 200 nodes, and 400 edges. Hard limits are 100 rows and three hops.
Cycles are deduplicated by typed node ID and relationship identity. Expansion stops at every depth,
node, edge, byte, time, and cancellation budget and reports partial/truncated state plus a cursor.
Core retrieval and exact document reads never depend on graph availability.

## Compatibility

HTTP and Desktop consume this same contract. Additive fields are compatible within v1. Removing a
type, weakening provenance or authorization, changing relationship semantics, or changing stable
identity requires a new contract and migration path. Older clients may ignore unknown additive
node or edge types; servers never relabel a new inference as a legacy explicit edge.
