# Code intelligence

Code intelligence is a revision-aware, derived projection over canonical filesystem Documents. It
does not make parser output authoritative and it does not activate indexing for any root that the
operator has not already approved.

## Repository and revision identity

Filesystem ingestion records `cortana.code-index.v1` metadata: an opaque repository ID, sanitized
remote, display name, branch/default branch, commit SHA, dirty/detached/shallow/worktree/submodule
flags, and observation time. Absolute local paths and remote credentials are never returned. Local
no-Git roots use a hash of the canonical path for uniqueness without disclosing that path. Document
IDs combine the opaque repository identity and relative path, so equal paths in different roots do
not collide. `code://` evidence URIs contain only that opaque identity and an encoded relative path.

The revision key includes commit, branch/detached state, and dirty state. It participates in the
document hash and derived symbol IDs. A commit, branch move, rename, deletion, parser version, or
content change therefore replaces the affected projection transactionally and advances the corpus
revision. Complete snapshot reconciliation removes deleted documents and their cascading code
indexes; partial runs never reconcile. The canonical document remains available if parsing is
unsupported or incomplete, and rollback consists of deleting/rebuilding derived `code_indexes`.

## Bounded parser contract

`CodeParser` is a replaceable boundary. The initial `BoundedSyntaxParser` detects Rust, Python,
TypeScript/JavaScript, Go, Java, C/C++, Swift, and Ruby. It emits normalized declarations, exact
UTF-8 byte/line spans, documentation, imports, and explicit resolved/unresolved relations.

Defaults cap a file at 2 MB, 20,000 symbols, 40,000 relations, and 250 ms. Cancellation is checked
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
references, and exports. Every edge carries source span, parser version, confidence, origin,
resolution, and dynamic state. `GET /v1/code/relations` and MCP `code_relations` expose one bounded,
paginated hop, so cycles cannot cause unbounded traversal. ACL and project filtering happens before
symbols or edges are returned. `POST /v1/code/symbols` and MCP `lookup_symbol` provide definitions;
generic `search_code` continues to provide hybrid concept search and neighboring source context.

Code chunking (`cortana.chunking.v2`) creates symbol/declaration units and preserves generic fallback
for unsupported or partial inputs. Evidence metadata is retained through search, ContextBundle, API,
MCP, web, and Desktop surfaces. Desktop renders repository, branch, abbreviated commit, and
dirty/committed state.

## Embedding decision and staged rollout

`uv run python scripts/evaluate-code-retrieval.py` runs the synthetic
`cortana.code-retrieval-eval.v1` matrix across exact identifier, architecture, error, API,
dependency, and impact queries. It compares lexical-symbol, the shared text representation, a local
code-specific representation, and local fusion/reranking. The report records recall@3, MRR, span
correctness, latency, index bytes, representation time, cache reuse, and peak memory.

The selected M9 strategy is shared text retrieval plus exact-symbol local reranking. The synthetic
matrix reached 1.0 recall@3, MRR, and span correctness for that candidate without creating a second
embedding generation. Production activation remains separately approval-gated: an operator must
approve the roots and a bounded, sampled, non-reconciling trial. Generated, vendor, and worktree
trees stay excluded. Failure degrades to lexical/symbol retrieval; canonical evidence is neither
deleted nor rewritten. A future second embedding generation requires explicit routing, cache,
fusion, provenance, backup, rebuild, and rollback contracts plus measured resource budgets.
