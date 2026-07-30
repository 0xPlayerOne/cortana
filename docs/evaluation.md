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

## Production readiness

`cortana readiness` performs read-only operational checks and emits JSON:

```bash
cortana readiness \
  --api-url http://127.0.0.1:7331 \
  --max-backup-age-hours 48
```

It checks the live API liveness endpoint, embedding probe, SQLite integrity, backup freshness,
query mode, and recurring-sync installation state. It never invokes a connector, starts a sync,
or writes indexed content. The safe default fails when a recurring sync service is installed.
After every source has been explicitly validated, `--allow-sync-service` records that operational
acknowledgement for the check; it does not install or start the service.
