# Code intelligence

Code intelligence is a revision-aware, derived projection over canonical filesystem Documents. It
does not make parser output authoritative and it does not activate indexing for any root that the
operator has not already approved. [ADR 0002](architecture/0002-derived-code-intelligence.md)
records the persistence, trust, invalidation, and rollout decision.

## Repository and revision identity

Filesystem ingestion records `cortana.code-index.v1` metadata: an opaque repository ID, sanitized
remote, display name, branch/default branch, commit SHA, dirty/detached/shallow/worktree/submodule
flags, and observation time. Absolute local paths and remote credentials are never returned. Local
no-Git roots use a hash of the canonical path for uniqueness without disclosing that path. Document
IDs combine the opaque repository identity and relative path, so equal paths in different roots do
not collide. `code://` evidence URIs contain only that opaque identity and an encoded relative path.

The revision key includes commit, branch/detached state, and dirty state. It participates in the
document hash, parser cache, and derived symbol IDs; observation time is retained as provenance but
excluded from searchable-payload identity. A commit, branch move, rename, deletion, parser version,
or content change therefore replaces the affected projection transactionally and advances the corpus
revision, while an observation-only rescan reuses existing work. Complete snapshot reconciliation removes deleted documents and their cascading code
indexes; partial runs never reconcile. The canonical document remains available if parsing is
unsupported or incomplete, and rollback consists of deleting/rebuilding derived `code_indexes`.

## Bounded parser contract

`CodeParser` is a replaceable boundary. The initial `BoundedSyntaxParser` detects Rust, Python,
TypeScript/JavaScript, Go, Java, C/C++, Swift, and Ruby. It emits normalized declarations, exact
UTF-8 byte/line spans, documentation, imports, and explicit resolved/unresolved relations.

Defaults cap a file at 2 MB, parser memory at 64 MiB, symbols at 20,000, relations at 40,000, and
wall time at 250 ms. The memory budget conservatively reserves bounded storage per derived symbol
and relation before parsing begins. Cancellation is checked
while scanning. Unsupported, malformed, oversized, timed-out, and cancelled inputs return a typed
status and diagnostics; they never alter the canonical source. Cache identity includes content hash,
language, parser version, and resource configuration. Adding a parser does not change persistence,
retrieval, or ACL ownership.

The parser uses only Rust standard-library scanning and the repository's existing Apache-2.0
package; it adds no native parser runtime or license. Git discovery invokes the installed Git CLI
read-only with optional locks disabled. No shell is involved. No-Git and Git-unavailable trees have
an explicit fallback, so Linux, macOS, and Windows packaging remains functional.

## Symbols, relations, retrieval, and access control

Symbols carry stable IDs, qualified names, kind, language, repository/revision/file/span,
signature, visibility, container, documentation, aliases, and generated state. Exact identifier
matches rank before signature/documentation matches. Duplicate visible definitions are returned as
`ambiguous`; hidden definitions cannot create visible ambiguity. Cortana never invents a target for
an unresolved or dynamic relation.

Relations represent imports, dependencies, inheritance/implementation, calls, containment,
references, exports, and overrides. Every edge carries source span, parser version, confidence,
and a typed `direct_syntax`, `resolved`, `inferred`, `dynamic`, `unresolved`, or `ambiguous` origin,
plus resolution and dynamic state. File/module symbols own top-level import and dependency edges, so
those edges remain reachable through the same relation query. Cross-file targets resolve only when
one visible definition matches within the same repository and revision; duplicates remain
`ambiguous`. `GET /v1/code/relations` and MCP `code_relations` expose bounded `neighborhood`,
`callers`, `callees`, `dependencies`, and reverse `impact` queries. Impact depth is capped at three,
visited symbols make cycles safe, and every page remains capped and paginated with an opaque cursor
bound to the query scope and corpus revision. Traversal fails closed before fixed index, symbol, or
relation scan budgets are exceeded. ACL and project filtering happens before
symbols or edges are returned. `POST /v1/code/symbols` and MCP `lookup_symbol` provide definitions;
both accept exact repository ID, revision, language, file, and qualified-name filters and return at
most 50 rows. Reads are recorded in the metadata-only audit log.
`search_code`, ordinary search, and ContextBundle retrieval promote an exact visible symbol to the
canonical definition span before weaker semantic matches while retaining hybrid concept search and
neighboring source context. Promoted evidence carries symbol/repository/revision/file/span metadata.
All evidence metadata is recursively stripped of credential-shaped fields and bounded before it is
serialized into search or ContextBundle responses.

Code chunking (`cortana.chunking.v2`) creates symbol/declaration units and preserves generic fallback
for unsupported, failed, or partial inputs. Evidence metadata is retained through search, ContextBundle, API,
MCP, web, and Desktop surfaces. Desktop renders repository, branch, abbreviated commit, and
dirty/committed state.

## Embedding decision and staged rollout

`uv run python scripts/evaluate-code-retrieval.py` runs the deterministic fixture
`cortana.code-retrieval-eval.v1` matrix across exact identifier, architecture, error, API,
dependency, and impact queries over twelve checked-in production source files. Its shared-text
path mirrors the runtime `DeterministicEmbedder` fingerprint and algorithm; the code-token arm is a
measured candidate generation, not a model-name assumption. It builds and queries each
representation, verifies byte spans against the checked-in source, performs a repeated cache build, and measures index bytes, representation
time, query latency, and peak memory. It compares lexical-symbol, the shared deterministic text
representation, a separate code-token representation, and shared-text local fusion/reranking. The
report records recall@3, MRR, span correctness, latency, index bytes, representation time, cache
reuse, and peak memory; CI asserts the selection and resource gates.
The report resolves and records the actual Git `HEAD`, records working-tree cleanliness, and checks
each declaration against a committed path, line, and signature fragment rather than deriving its
expected span from an unrestricted search. Activation requires a clean committed revision.
The report also performs and times a clean strategy-index build, records documents scanned,
re-embedding count, vector bytes written, and whether an additional generation would be created.

Selection is computed from measured MRR subject to span, latency, and no-second-generation gates;
the configured decision resolves to shared text retrieval plus exact-symbol local reranking.
Version-specific results belong in [release evidence](releases.md), not this architecture contract.
Production activation remains separately approval-gated: an operator must
approve the roots and a bounded, sampled, non-reconciling trial. Generated, vendor, and worktree
trees stay excluded. Failure degrades to lexical/symbol retrieval; canonical evidence is neither
deleted nor rewritten. A future second embedding generation requires explicit routing, cache,
fusion, provenance, backup, rebuild, and rollback contracts plus measured resource budgets.
