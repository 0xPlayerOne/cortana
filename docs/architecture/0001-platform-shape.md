# ADR 0001: Canonical evidence platform with optional memory providers

Status: accepted

## Context

The predecessor Hermes infrastructure stores code and personal knowledge in separate Chroma
collections, exports a Markdown vault, supervises a local Qwen embedding service, and optionally
duplicates durable facts into Hindsight. Those parts work, but retrieval semantics, provenance,
project scope, permissions, and agent access are not unified.

## Decision

Cortana owns one normalized `Document`/`Chunk`/`Evidence` contract and a generation-aware index.
Every connector emits documents without knowing the storage implementation. The canonical store
retains source IDs, revisions, content hashes, timestamps, ACL scopes, provenance URLs, and raw
metadata. Derived chunks and embeddings can be rebuilt.

Retrieval combines lexical, semantic, rare-token, and recency rankings with reciprocal-rank fusion,
then applies source diversity, optional reranking, and neighboring-context expansion. MCP tools
expose retrieval primitives rather than an opaque answer endpoint.

Postgres and pgvector are the production store. Local/cloud embedding providers implement the same
OpenAI-compatible contract. A model/dimension fingerprint defines an index generation, preventing
vectors from different models from being compared.

Hindsight remains an optional sink/source for reflective temporal memory. It cannot become the
canonical store because its abstractions do not preserve Cortana's complete source and access
contract. Honcho may be added through the same interface after an evaluation suite proves value.

Rust owns every long-running process and the canonical ingestion, storage, retrieval, API, MCP, and
CLI paths. Python is a connector subprocess boundary only for vendor SDKs and macOS automation that
would otherwise require reimplementing stable platform integrations. Connector communication uses
versioned JSON Lines so Python cannot leak into the core runtime.

## Consequences

- Existing Hermes adapters can migrate incrementally.
- Agents receive fast, structured evidence without forcing an extra synthesis call.
- Human answers and agent retrieval share ranking behavior.
- Provider changes require a new index generation, not an unsafe in-place model swap.
- Operational complexity includes Postgres in the production profile, migrations, backups, and
  explicit access filtering.
