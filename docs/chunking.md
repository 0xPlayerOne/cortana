# Structured chunking

Cortana stores the exact connector `Document` as the canonical record. Chunk rows are derived
retrieval units and can be rebuilt without changing source identity, content, provenance, ACLs,
or citations. The current contract is `cortana.chunking.v1`.

## Selection

The Rust chunker chooses a strategy from non-secret source metadata:

- Markdown and HTML/exported documents use heading/section boundaries before the bounded splitter.
- Gmail, Slack, and Discord records use message/thread boundaries when an exported transcript
  provides them.
- Calendar events and compact structured records keep field boundaries together before splitting.
- Unknown or malformed records use the generic compatibility splitter.

Code AST/symbol chunking is intentionally deferred to the code-intelligence milestone.

## Stable output

Each derived row records a SHA-256 `chunk_key` over the contract version, strategy, byte span, and
chunk text. Storage prefixes this key with the opaque document ID. The key is therefore stable for
unchanged canonical content under the same contract, while a content or policy change naturally
creates a new derived identity. `start_byte` and `end_byte` are UTF-8 byte offsets on the original
canonical content and are always character-boundary aligned. `parent_key`, `previous_key`, and
`next_key` describe section/message/record lineage and neighboring retrieval context.

The default target is 1,600 UTF-8 bytes with up to 200 bytes of overlap. Bounds are applied without
splitting Unicode scalar values. Trimming affects only derived chunk text; canonical `Document`
content is never normalized or rewritten.

## Migration and rollback

New ingestion uses `Store::needs_structured_update` to detect legacy ordinal-only rows and rebuilds
only the derived chunks for that document. The canonical row and its source ACL/provenance remain
unchanged. The transaction replaces all chunks and FTS rows atomically, increments the corpus
revision once, and is safe to retry after interruption. Unknown sources retain generic behavior.

Operators should stage this migration on an approved fixture or backup, compare `ChunkingStats`
against the prior generic output, and promote only after retrieval/citation, ACL, and latency gates
pass. A verified SQLite backup is the rollback boundary: restoring it returns the prior derived
index without contacting a connector. No live source or embedding provider is required to test
chunk generation.

## Quality/resource hooks

`cortana::chunking::stats` reports strategy, source bytes, chunk count, derived bytes, and overlap
bytes for bounded fixture comparisons. Evaluation should pin the corpus, chunking contract, and
embedding fingerprint, then compare recall/MRR, citation validity, duplicate-source crowding,
embedding reuse, and p50/p95 retrieval latency. Reports must contain IDs and metrics only, never
raw queries, content, credentials, or private paths.
