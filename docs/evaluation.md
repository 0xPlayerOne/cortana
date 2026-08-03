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
and does not trigger syncs or connector activity. The model-backed run extends `answer` report fields
with provider-backed-specific metrics:

- `attempted` / `passed`
- `planner_model_used` / `synthesis_model_used`
- `citations_valid`, `expected_evidence_present`, `forbidden_citations_absent`
- `fallback_mode`, `fallback_provider_unavailable`
- `latency_ms`, `deadline_ms`

The command exits nonzero when model quality thresholds fail.

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
  --max-backup-age-hours 48
```

It checks the live API liveness endpoint, embedding probe, embedding/index generation compatibility,
SQLite integrity, backup freshness, query mode, and recurring-sync installation state. A mismatch
is reported with the index and configured fingerprints instead of silently rebuilding or mixing
vectors. The JSON also exposes `embedding_generation.stored` and
`embedding_generation.configured` so Desktop can offer the same explicit, confirmation-gated
adoption path. It never invokes a connector, starts a sync, or writes indexed content. The safe
default fails when a recurring sync service is installed.
After every source has been explicitly validated, `--allow-sync-service` records that operational
acknowledgement for the check; it does not install or start the service. Validations must also be
current: a record older than `[ingestion].validation_max_age_hours` (168 hours by default; `0`
disables the bound) fails the check until `validate-source` is re-run, so a revoked credential or
changed scope cannot keep an old preflight blessing the schedule.
