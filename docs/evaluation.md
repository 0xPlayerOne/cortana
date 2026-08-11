# Evaluation and readiness

Cortana ships a synthetic, deterministic quality gate that never opens the configured index or
contacts a live source:

```bash
cortana eval
```

The command creates a unique temporary SQLite index, embeds four synthetic documents with the
deterministic test embedder, evaluates hybrid retrieval, and removes the index when complete. It
emits JSON and exits nonzero when a threshold fails. The required fixture covers:

- recall@k and mean reciprocal rank;
- project/source scoping and ACL denial;
- extractive answer fallback and citation validity;
- planner bounds, cache hits, and corpus-revision invalidation;
- a deterministic retrieval latency ceiling.

The built-in thresholds and data live in `eval/fixtures.json`. Use
`cortana eval --fixture /path/to/synthetic.json` for a versioned project-specific fixture. Never
put personal or production content into committed evaluation data.

## Bounded disposable load benchmark

Use the benchmark when you need repeatable latency or concurrency evidence without touching a
personal index:

```bash
python scripts/benchmark_query.py \
  --binary target/release/cortana \
  --iterations 8 \
  --concurrency 2 \
  --timeout-seconds 30 \
  --max-p95-ms 5000
```

Every iteration runs `cortana --offline eval` in a separate process. The evaluator creates and
removes a disposable SQLite index, uses deterministic local embeddings, and never opens the active
configuration, calls a connector, or contacts a model provider. The command emits machine-readable
JSON with per-iteration status and min/p50/p95/max latency. It exits nonzero on a failed evaluation,
timeout, missing iteration, or an optional p95 threshold breach. Keep the iteration and concurrency
bounds small enough to leave local resources available for the running Cortana service.

The CLI integration test runs the built-in evaluation in CI, so deterministic quality regressions
block promotion. Model quality is deliberately separate because it depends on local hardware and
the configured provider; enable synthesis and run a bounded query-only benchmark only after the
deterministic gate passes.

```bash
# deterministic gate (default, offline)
cortana eval

# model-backed evaluator (planner+synthesis), bounded and fixture-only
cortana --config /path/to/config.toml eval --model
```

`--model` always runs against synthetic fixtures only, does not open or modify a personal index,
and does not trigger syncs or connector activity. Route discovery can succeed even when the
provider-backed endpoint is unavailable; an unavailable provider is not model-backed proof, and
the extractive fallback remains the safe production mode. The model-backed run extends `answer` report fields
with provider-backed metrics:

- `attempted` / `passed`
- `planner_model_used` / `synthesis_model_used`
- `citations_valid`, `expected_evidence_present`, `forbidden_citations_absent`
- `fallback_mode`, `fallback_provider_unavailable`, and the stable non-secret
  `fallback_reason` (`provider_unavailable`, `invalid_citations`, `deadline`, or
  `extractive_fallback`)
- `latency_ms`, `deadline_ms`

The opt-in model gate applies a 30-second in-memory ceiling to the answer and provider request
timeouts, independent of the interactive query budget. It also caps the synthetic prompt context
at 2,048 tokens and the generated answer at 512 tokens, so a large personal query configuration
cannot turn this fixture-only check into a production load test. When the first planner or
synthesis call fails closed, the evaluator emits the failure report without repeating the same
provider outage for cache and post-update checks.

The command exits nonzero when model quality thresholds fail.

When the configured endpoint is the model-gateway adapter, Cortana removes an attribution
footer only when the response carries the gateway's explicit provider header and the footer
matches the gateway's exact shape and provider token (including its documented `Gateway` display
suffix). Ordinary bullets, uncited text, and responses without that header remain unchanged and
continue through citation validation fail-closed.

An earlier configured-provider attempt at source commit `339240e` passed the bounded model gate in
20,176 ms, and a packaged v0.29.31 rerun passed in 13,477 ms. However, the installed v0.29.33
evaluator failed closed twice (8,313 ms and 13,398 ms) after the planner call because the configured
provider appended an uncited attribution line to the synthesis response. The earlier passes are
historical evidence, not proof that the current provider is citation-safe. After raising the bounded
synthetic output cap to leave room for gateway reasoning, the source run passed with planner and
synthesis model use, valid citations, cache reuse, and revision invalidation in 22,866 ms. The
latest successful installed v0.29.60 aarch64 core binary rerun passed the same fixture-only evaluator in
17,928 ms; a prior cache-warm run passed in 10,323 ms. Both runs had
planner+synthesis model use, valid citations, cache reuse, and revision invalidation. The packaged
Desktop app was not launched; the verified v0.29.68 core is installed at
`/Users/amf/.local/bin/cortana` without starting the app or recurring sync. v0.29.67 and v0.29.68
are release-only version/changelog/lockfile bumps over v0.29.65 with no functional source changes.
A fresh v0.29.67 model-backed evaluation against the configured provider passed in 21,648 ms with
planner and synthesis model use, bounded planning, valid citations, cache reuse, and revision
invalidation. An
earlier bounded provider-unavailable attempt remains historical evidence of the fail-closed path;
the successful rerun re-established the provider gate without changing the safe extractive default.
Developer ID signing/notarization is not available in this environment. Extractive mode remains the
safe production default because synthesis is still an explicit opt-in in the production configuration.

```toml
[query]
synthesis_enabled = true
base_url = "http://127.0.0.1:8008/v1"
model = "auto-efficient"
```

## Production readiness

`cortana readiness` performs read-only operational checks and emits JSON:

```bash
cortana readiness \
  --api-url http://127.0.0.1:7331 \
  --max-backup-age-hours 48 \
  --storage-timeout-seconds 240
```

It checks the live API liveness endpoint, embedding probe, embedding/index generation compatibility,
SQLite integrity, backup freshness, query mode, and recurring-sync installation state. This is a
comprehensive read-only check, not the quick `/healthz` process-liveness probe: an observed run
scanned roughly 1 GB of database and backup data, taking about 130 seconds for database integrity
and about 80 seconds for the backup scan. A mismatch is reported with the index and configured
fingerprints instead of silently rebuilding or mixing vectors. The JSON also exposes `embedding_generation.stored` and
`embedding_generation.configured` so Desktop can offer the same explicit, confirmation-gated
adoption path. It never invokes a connector, starts a sync, or writes indexed content. The safe
default fails when a recurring sync service is installed. SQLite integrity and backup verification
run on dedicated blocking threads so they do not stall the async runtime; each probe has the explicit
`--storage-timeout-seconds` bound (1 to 300 seconds, 240 by default), and a timeout or worker error
is reported as a failed check rather than treated as degraded success.
After every source has been explicitly validated, `--allow-sync-service` records that operational
acknowledgement for the check; it does not install or start the service. Validations must also be
current: a record older than `[ingestion].validation_max_age_hours` (168 hours by default; `0`
disables the bound for read-only/manual checks and is rejected for recurring sync) fails the check
until `validate-source` is re-run, so a revoked credential or changed scope cannot keep an old
preflight blessing the schedule.
