# Query pipeline

Cortana separates agent retrieval from human-facing answers.

- MCP, CLI search, `/v1/search`, and `/v1/context` are low-latency evidence primitives. They never
  require a language model.
- MCP also exposes `search_code`, `search_messages`, and `who_knows`. These tools search only the
  enabled source groups derived from configuration, embed the query once across the group, and
  return evidence rather than inferred people profiles.
- The workspace uses `/v1/answer`, which can plan several searches and synthesize a cited response.
- Both paths share the same project/source filters and hybrid lexical, semantic, IDF, and recency
  ranking.

After document-level ranking and deduplication, Cortana expands each selected passage by one
neighboring chunk on either side from the same canonical document. Expansion is ACL-checked in the
database, reconstructs configured chunk overlap, and is capped at 16 KiB per result. The public
search limit remains 50 results, so neighboring context cannot turn a narrow query into an
unbounded corpus read. The later context builder applies its independent token budget.

## Canonical document browser

The Obsidian-style sidebar uses the canonical index rather than search results:

- `GET /v1/documents` returns at most 100 document summaries with project/source filters, an
  optional case-insensitive `query` filter over title/source/source ID, and an opaque keyset
  cursor. The desktop requests 50 at a time and virtualizes the visible rows.
- `GET /v1/documents/{id}` returns one ACL-authorized canonical document, its safe metadata, and
  display bounds. It also includes the stable source ID, ACL labels, up to 12 explicit metadata
  backlinks, and up to eight nearby documents from the same source. All relations are ACL-filtered
  before serialization. Missing and unauthorized IDs deliberately share the same `404` response.
- `GET /v1/graph` exposes the bounded, paginated graph contract used by the future corpus graph. A
  page contains workspace, source, and document nodes plus `contains` edges; it accepts the same
  filters and cursor as the document list and never materializes the corpus at once.
- Every list and read is filtered by the authenticated principal's ACL labels and recorded in the
  metadata-only audit trail. Document content and query strings are never written to audit events.

New ingestion stores exact canonical content alongside retrieval chunks. Existing indexes remain
compatible: a document read reconstructs legacy content while removing chunk overlap, and the
next ordinary refresh backfills exact content. On first open after upgrade, Cortana builds the
backlink lookup once from bounded values under explicit relationship fields such as `references`,
`links`, and source/document IDs. Unrelated metadata and credential fields are not indexed. The
upgrade does not read document bodies, run embeddings, or contact any source. Subsequent document
reads use the indexed lookup rather than a corpus scan. A single display response is capped at 2 MiB and
reports `truncated=true`; the original source link remains available for unusually large records.
Pagination is deterministic by update timestamp and stable document ID, so browsing does not load
the whole corpus into memory. The sidebar keeps workspace selection visible, supports collapsed
project/source nodes, filters on the server, and renders a fixed-height virtual document window.
Opening the app, changing workspace, expanding sources, and reading graph pages do not run
embeddings or a language model; retrieval begins only when the user submits a search or explicitly
builds an agent context bundle.

## Safe default

`[query].synthesis_enabled` defaults to `false`. The answer endpoint still works: it performs one
hybrid retrieval and returns a deterministic extractive brief with stable `[n]` citations. This is
the production fallback whenever the planner or synthesizer is unavailable or returns invalid
output.

```toml
[query]
synthesis_enabled = false
```

This setting does not affect ingestion and does not start any background work.

## Planned and synthesized answers

After an OpenAI-compatible model endpoint is healthy, enable synthesis:

```toml
[query]
synthesis_enabled = true
base_url = "http://127.0.0.1:8008/v1"
model = "auto-efficient"
max_planned_queries = 4
retrieval_limit = 10
result_limit = 20
context_tokens = 8000
output_tokens = 1200
request_timeout_seconds = 45
answer_timeout_seconds = 55
request_concurrency = 4
```

The planner returns only bounded JSON search strings. Cortana preserves the original question,
deduplicates expansions, rejects empty/oversized output, and hard-clamps fan-out to eight.
Retrievals run concurrently and are fused by cross-query reciprocal rank. The synthesizer sees
only a token-bounded evidence bundle and must cite every non-empty paragraph with numbered
passages. Missing, out-of-range, or paragraph-incomplete citations cause an extractive fallback.
Evidence is treated as historical unless it explicitly proves current state, so old runbooks and
status notes cannot silently become claims about the live deployment.

The default endpoint is the local model gateway on port 8008. Stable `x-session-id` values and
stable system prefixes let a compatible gateway reuse prompt caches across planner and synthesis
requests. Any OpenAI-compatible cloud endpoint can be substituted:

```toml
[query]
synthesis_enabled = true
base_url = "https://provider.example/v1"
model = "provider-model"
api_key_env = "CORTANA_QUERY_API_KEY"
```

Keep the key in the process environment or the private `[runtime].env_file`; never put its value in
the TOML file.

## Cache behavior

Answers are keyed by:

- query contract version and corpus revision;
- query text plus project/source scope;
- embedding fingerprint;
- model endpoint/name;
- planner, retrieval, context, and output bounds.

Changed/deleted content and changed source timestamps invalidate prior keys. TTL and least-recently
used bounds are configurable:

```toml
[query]
cache_max_entries = 10000
cache_ttl_seconds = 3600
```

Set `cache_ttl_seconds = 0` to skip cache reads or `cache_max_entries = 0` to skip new writes.

## Failure contract

Planner failure uses the original query. Individual retrieval failures are reported as warnings
while successful evidence continues. Model unavailability, timeout, invalid JSON, missing
citations, or unknown citations produces an extractive answer. An empty index returns an explicit
insufficient-evidence response. The response always reports `mode`, `cached`, `latency_ms`, the
executed plan, evidence, and warnings so the workspace can make degradation visible.
The end-to-end deadline is hard-clamped to 55 seconds so a slow planner still leaves time for a
citation-stable fallback before the HTTP request deadline.

`cortana readiness` performs a minimal grounded completion against the configured query model when
synthesis is enabled. Configuration alone is not considered production-ready. The check fails
closed if the endpoint is unavailable or does not follow the evidence-and-citation contract.
