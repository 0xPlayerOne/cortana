# ADR 0002: Derived revision-aware code intelligence

Status: accepted

## Context

Canonical filesystem Documents preserve exact source evidence, but text-only chunks cannot reliably
answer definition, reference, dependency, or change-impact questions. Code parsing is fallible and
operates on untrusted repositories, so parser output cannot become a second authority or bypass the
existing project, ACL, audit, backup, and reconciliation boundaries.

## Decision

Cortana stores code intelligence as a rebuildable `code_indexes` projection owned by the Rust store.
Each projection is keyed to its canonical document, repository/revision identity, content hash,
parser version, language, and bounded parser configuration. SQLite foreign-key cascading removes a
projection when its canonical document is reconciled; changing source or identity transactionally
replaces only the affected projection. Observation timestamps are provenance, not searchable-payload
identity, so repeated unchanged scans reuse parsing, chunks, embeddings, and relationships.

The parser boundary is replaceable, cancellable, and resource-bounded. Unsupported, oversized,
cancelled, timed-out, malformed, and partial parses preserve the canonical Document and use generic
chunking. Symbols and relations retain exact byte/line spans and repository/revision provenance.
Relations label direct syntax, resolution, inference, ambiguity, unresolved targets, and dynamic
dispatch explicitly. Visible cross-file definitions resolve within repository/revision scope;
callers, callees, dependencies, neighborhoods, and reverse impact use cycle-safe traversal with a
maximum impact depth of three. APIs apply project and ACL scope before serialization, cap reads at
50 rows, bind opaque cursors to scope and corpus revision, fail closed at fixed scan budgets, and
write metadata-only audit events, including invalid requests. Serialized evidence metadata is
credential-filtered and size-bounded.

Code retrieval reuses the current text embedding generation and applies exact-symbol local fusion
to search and ContextBundle evidence, including canonical source spans and provenance.
A separate code embedding generation is rejected until measured benefit justifies its migration,
routing, cache, backup, and rollback costs. Index activation remains an explicit, sampled,
non-reconciling operator action over approved roots; generated, vendor, and worktree content is not
promoted as authoritative code evidence.

## Consequences

- Canonical source, deletion authority, backup, and ACL ownership remain unchanged.
- Derived indexes can be deleted and rebuilt without rewriting source evidence.
- Repository moves and revision changes invalidate deterministic identity; observation-only scans do
  not cause re-embedding.
- Parser coverage may be partial, but failures degrade to ordinary file retrieval instead of losing
  evidence or inventing graph edges.
- Adding a parser or separate embedding generation requires a versioned contract and repeatable
  evaluation before rollout.
