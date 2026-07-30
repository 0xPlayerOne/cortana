# Query pipeline

Cortana separates agent retrieval from human-facing answers.

- MCP, CLI search, `/v1/search`, and `/v1/context` are low-latency evidence primitives. They never
  require a language model.
- The workspace uses `/v1/answer`, which can plan several searches and synthesize a cited response.
- Both paths share the same project/source filters and hybrid lexical, semantic, IDF, and recency
  ranking.

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
