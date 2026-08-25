# Query pipeline

The public compatibility and ContextBundle contracts are [Public API](contracts/public-api.md) and
[ContextBundle](contracts/context-bundle.md).

Cortana separates agent retrieval from human-facing answers.

## Current release boundary

The current protected source and published package are `v0.34.44`. Query-only retrieval, native
memory separation, ACL filtering, bounded context, citations, cache invalidation, and extractive
fallback remain the safe defaults; provider-backed synthesis is still an explicit opt-in.

- MCP, CLI `search`/`context`, `/v1/search`, and `/v1/context` are low-latency retrieval
  primitives. They never require a language model. Context surfaces also attach relevant native
  agent memories from the same ACL-filtered SQLite store.
- MCP also exposes `search_code`, `search_messages`, and `who_knows`. These tools search only the
  enabled source groups derived from configuration, embed the query once across the group, and
  return evidence rather than inferred people profiles.
- MCP request validation is identical to the HTTP and CLI retrieval contract: queries must be
  non-empty and at most 16 KiB, scope filters are bounded, and each tool returns at most 50 rows.
  Oversized or blank requests are rejected before embedding work begins.
- The workspace uses `/v1/answer`, which can plan several searches, attach authorized native
  memory, and synthesize a cited response. Native memory is returned separately from cited
  evidence and is included only for principals with the `memory` scope.
- Native memory recall is provider-free: bounded SQLite FTS candidates are ranked by query
  coverage, lexical match, confidence, importance, freshness, and exact-vs-fallback mode. Its
  `relevance_score` is diagnostic metadata and never replaces ACL or lifecycle checks.
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
- `GET /v1/graph` exposes the bounded, paginated graph contract used by the Desktop graph view. A
  page contains workspace, source, and document nodes plus `contains` edges; it accepts the same
  filters and cursor as the document list and never materializes the corpus at once. The Desktop
  renders a bounded hierarchical view of the returned nodes, with type filters for workspaces,
  sources, and documents, local text filtering, node-specific icons, and a selected-node
  relationship summary. Pagination remains incremental and loading is separate from retrieval and
  embedding work; the UI never loads the entire corpus into memory.
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

## CLI context bundle

`cortana context QUERY` returns the same citation-ready, token-bounded bundle as the MCP `context`
tool and `POST /v1/context`, through the identical local retrieval pipeline and context builder.
The bundle keeps `memories` separate from numbered `evidence`: memories are durable operational
context, not source citations.
It is the CLI fallback for agents without MCP or HTTP access and needs no running server:

```bash
cortana context "how do releases work?" --project engineering --source runbooks
```

The subcommand accepts the same optional filters and strict bounds as the API contracts:

- `--project` and `--source` scope retrieval exactly like the HTTP and MCP endpoints.
- `--limit` defaults to 10 and is strictly bounded to 1–50, the pipeline's shared result cap.
- `--max-tokens` defaults to the configured `[query].context_tokens` budget (8,000 by default)
  and is strictly bounded to 256–64,000, the context builder's clamp range.
- Queries must be non-empty and at most 16 KiB, matching the shared `MAX_QUERY_BYTES` bound.

Out-of-contract values are rejected at parse time. Output is stable JSON with the same shape as
`/v1/context`: the assembled `context` Markdown with numbered `[n]` citations, the included
`evidence` rows, relevant `memories`, and `metrics` (`retrieved`, `included`, `omitted`,
`memories_retrieved`, `memories_included`, `memories_omitted`, `estimated_tokens`, `source_tokens`,
`reduced_tokens`, `reduction_ratio`, and the applied token budget). The bundle also reports
`retrieval_mode` (`hybrid` or
`lexical-fallback`) and, when degraded, a non-secret `retrieval_warning`. Like every other command, it runs against the local index only; use
`--offline` for the deterministic embedding path when the index generation matches.

The CLI fallback is owner-local: it runs as the local machine user with no bearer credentials, so
it cannot enforce `[[auth.tokens]]` principals or document ACL labels. Shared or narrowly scoped
agents must use the MCP server with `--token-env` or the bearer-authenticated HTTP API instead.
Every successful or failed `cortana context` call records a metadata-only audit event under the
`local-cli` principal with action `local-cli/context` — project/source scope, outcome, result
count, and latency only. Query text and evidence content are never written to audit events, and an
unavailable audit store never fails the command.

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

## Provider model discovery

`cortana provider-models --kind embedding|query` lists the models the configured provider
advertises through its OpenAI-compatible `/models` endpoint, so Desktop settings can offer the
provider's real catalog instead of a fixed preset list. The call is read-only and strictly
bounded: the shared provider URL contract (HTTPS, or HTTP only on loopback) is enforced before any
request, redirects are never followed, the request has a fixed 10-second timeout, and the
response body, model count, and every echoed id are capped. Only sanitized model ids are
returned, plus explicit capability metadata when the provider advertises it — capabilities are
never inferred from model ids or names. The provider API key is used only for the request
`Authorization` header and is never printed, stored, or included in errors.

```bash
cortana provider-models --kind query
```

## Model-backed evaluation run

`cortana eval --model` is a separate, opt-in quality gate for the real planner+synthesis path. It
always uses local synthetic fixtures and the deterministic embedder. It never opens personal data,
does not run source syncs, and requires the configured `[query]` provider (and its optional API key)
to be valid. The command enables synthesis only in memory for this run; the safe production
default remains extractive fallback when `synthesis_enabled = false`.

```bash
cortana --config /path/to/config.toml eval --model
```

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
candidate_multiplier = 8
semantic_weight = 1.0
lexical_weight = 1.2
idf_weight = 0.08
recency_weight = 0.1
# Optional deterministic local second pass; disabled by default.
reranker_enabled = false
context_tokens = 8000
output_tokens = 1200
request_timeout_seconds = 45
answer_timeout_seconds = 55
request_concurrency = 4
```

The planner returns only bounded JSON search strings. Cortana preserves the original question,
deduplicates expansions, rejects empty/oversized output, and hard-clamps fan-out to eight.
Retrievals run concurrently and are fused by cross-query reciprocal rank. Fused rows are then
re-scored against the original question with meaningful-term coverage; when that signal is
strong, only the strongest lexical matches survive into the synthesis bundle, so rows that
merely match a tangential planner query cannot be cited. Purely semantic questions (no strong
lexical signal) keep the full fused set, and dropped rows are reported in `warnings` as an
`evidence focus` entry. The synthesizer sees a token-bounded evidence-and-memory bundle. Evidence
remains the citation authority; native memory is clearly labelled operational context and is never
counted as a source citation. The model must cite every non-empty paragraph with numbered passages.
Missing, out-of-range, or paragraph-incomplete citations cause an extractive fallback.
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

The ranking contract is `cortana.retrieval.ranking.v2`. It combines bounded semantic and lexical
candidate pools with reciprocal-rank fusion, IDF term coverage, recency, exact lexical matching,
and canonical-record deduplication. The tuning values above are clamped before use and are part of
the answer cache key. When enabled, the local reranker applies a bounded title/phrase/term-coverage
boost to the already bounded candidate set; it makes no provider calls and fails open to the fused
ranking. It remains disabled by default until approved-corpus evaluation demonstrates a quality
win within the latency budget.

Search responses expose non-secret ranking diagnostics in `x-cortana-retrieval-*` headers:
ranking contract, fused candidate count, deduplicated count, and returned count. They contain no
query, document, path, credential, or provider content.

## Cache behavior

Answers are keyed by:

- query contract version and corpus revision;
- native memory revision and memory ACL scope when memory context is enabled;
- query text plus project/source scope;
- embedding fingerprint;
- model endpoint/name;
- planner, retrieval, context, and output bounds.
- retrieval ranking contract and bounded tuning values.

Changed/deleted content and changed source timestamps invalidate prior keys. TTL and least-recently
used bounds are configurable:

```toml
[query]
cache_max_entries = 10000
cache_ttl_seconds = 3600
```

Set `cache_ttl_seconds = 0` to skip cache reads or `cache_max_entries = 0` to skip new writes.
When synthesis is enabled, only citation-validated synthesized answers are written to the
persistent answer cache; temporary planner/provider failures therefore recover on the next request
instead of being hidden until the TTL expires.

## Failure contract

Planner failure uses the original query. Individual retrieval failures are reported as warnings
while successful evidence continues. If the embedding provider is unavailable, returns no or
invalid vectors, has a dimension mismatch, or exceeds the
five-second interactive budget, retrieval falls back to lexical search and marks the response
`retrieval_degraded` (answers) or `retrieval_mode = lexical-fallback` (contexts); the fallback is
not cached for configured model-backed answers. Model unavailability, timeout, invalid JSON, missing
citations, or unknown citations produces an extractive answer. An empty index returns an explicit
insufficient-evidence response. The response reports `mode`, `cached`, `latency_ms`, the executed
plan, evidence, authorized native `memories`, and warnings so the workspace can make degradation
visible. The end-to-end deadline is hard-clamped to 55 seconds so a slow planner still leaves time
for a citation-stable fallback before the HTTP request deadline.

`cortana readiness` performs a minimal grounded completion against the configured query model when
synthesis is enabled. Configuration alone is not considered production-ready. The check fails
closed if the endpoint is unavailable or does not follow the evidence-and-citation contract.
