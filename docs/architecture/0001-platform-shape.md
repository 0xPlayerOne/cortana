# ADR 0001: Canonical evidence and native memory platform

Status: accepted

## Context

The predecessor Hermes infrastructure stores code and personal knowledge in separate Chroma
collections and exports a Markdown vault. Those parts work, but retrieval semantics, provenance,
project scope, permissions, knowledge, and agent memory are not unified.

## Decision

Cortana owns one normalized `Document`/`Chunk`/`Evidence` contract and a generation-aware index.
Every connector emits documents without knowing the storage implementation. The canonical store
retains source IDs, revisions, content hashes, timestamps, ACL scopes, provenance URLs, and raw
metadata. Derived chunks and embeddings can be rebuilt.

Retrieval combines lexical, semantic, rare-token, and recency rankings with reciprocal-rank fusion,
then applies source diversity, optional reranking, and neighboring-context expansion. MCP tools
expose retrieval primitives rather than an opaque answer endpoint.

SQLite is the current production store for the local-first Desktop and agent installation. It keeps
the canonical documents, chunks, provenance, ACL metadata, audit events, and embedding generations
in one owner-controlled data directory with verified backups. A future hosted or multi-user profile
may add Postgres/pgvector behind the same store contract; that deployment is not required by the
current release and is not silently provisioned. Local/cloud embedding providers implement the same
OpenAI-compatible contract. A provider-endpoint/model/dimension fingerprint defines an index
generation, preventing vectors from different services or models from being compared or mixed.

Native memory is stored beside canonical documents in SQLite, but it has an explicit lifecycle and
never bulk-copies source content. Typed memories carry provenance, confidence, importance, validity,
supersession, redaction, and workspace ACLs. Agents use the same MCP/HTTP/CLI contract for knowledge
context and memory recall; each context bundle keeps relevant memories separate from cited source
evidence, so retention and deletion remain inside Cortana's audited backup boundary.

Rust owns every long-running process and the canonical ingestion, storage, retrieval, API, MCP, and
CLI paths. Python is a connector subprocess boundary only for vendor SDKs and macOS automation that
would otherwise require reimplementing stable platform integrations. Connector communication uses
versioned JSON Lines so Python cannot leak into the core runtime.

## Consequences

- Existing Hermes adapters can migrate incrementally.
- Agents receive fast, structured evidence without forcing an extra synthesis call.
- Human answers and agent retrieval share ranking behavior.
- Provider changes require a new index generation, not an unsafe in-place model swap.
- Operational complexity includes SQLite migrations, verified backups, index-generation changes,
  and explicit access filtering; a future hosted Postgres profile would add its own migration and
  deployment gates.
