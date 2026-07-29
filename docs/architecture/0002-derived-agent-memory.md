# ADR 0002: Keep agent memory derived and optional

Status: accepted

## Decision

Cortana does not replicate Hindsight or Honcho and does not bulk-copy the canonical corpus into
either system. Cortana owns durable evidence, provenance, incremental updates, hybrid retrieval, and
token-bounded agent context. Those are the requirements for the shared second brain.

Hindsight remains an optional agent-episode sidecar for workflows that explicitly need its
retain/recall/reflect model. It is disabled in the default deployment and must use Cortana document
IDs as external provenance when enabled. Hindsight's document API supports stable document IDs,
replacement on re-retain, and original chunks, which makes a future derived adapter feasible
without making it authoritative. See the official
[Hindsight document contract](https://hindsight.vectorize.io/developer/api/documents).

Honcho is deferred. Its current strengths are conversational state, peer representations, and
theory-of-mind reasoning rather than evidence-preserving personal knowledge retrieval. Its hosted
workspace and separate agent-integration model would add a second security, billing, and retention
boundary. See the official [Honcho integration overview](https://honcho.dev/docs/v3/guides/overview).

## Rationale

- Copying every email, file, note, and code chunk into an LLM-driven memory extractor duplicates
  storage and makes deletion, permissions, and provenance harder to prove.
- Hindsight reflection is valuable for curated outcomes and agent episodes, but it consumes model
  calls and produces derived observations that cannot replace source evidence.
- Cortana already provides semantic, lexical, IDF, recency, document deduplication, and cited
  context without requiring an additional synthesis call.
- One canonical store preserves deterministic rebuilds and makes cache hits measurable.

## Re-evaluation gates

A provider adapter can ship after an evaluation demonstrates a material improvement over Cortana
alone on a versioned personal-memory benchmark. It must have a durable outbox, idempotent document
IDs, deletion propagation, project/tag scope mapping, retry telemetry, and a complete export path.
Until those gates pass, agents should use Cortana's `context` MCP tool and add only deliberately
curated episodic memories to a separate provider.
