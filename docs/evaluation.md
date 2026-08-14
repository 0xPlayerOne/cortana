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

### Current release boundary

The current protected source is `v0.32.0`, published through Release Please PR
#1182. Release-assets workflow `31795311797` completed all platform jobs and the
strict 18-asset verifier, including the credential-free packaged-core evaluator.
The v0.32.0 release includes the bounded read-only live-index evaluator, but that
harness still requires an operator-approved manifest and a successful run against an
approved corpus. This is separate from the deterministic fixture gate and is not a
packaged-GUI proof.
An explicit `readiness --allow-sync-service` check correctly failed closed because the
enabled sources have bounded or under-budget validation records; the recurring sync service
remains uninstalled.
The v0.32.0 release gate verifies the packaged core offline without credentials.
This remains fixture-only query-layer evidence and does not prove packaged GUI
behavior, personal-index sync, or Developer ID/notarization trust. The evaluator
remains bounded and opt-in; extractive mode is the production default.

The audited host is now version-aligned as well: `/Users/amf/.local/bin/cortana` and its managed
Python package both report `0.32.0`, while the embedding and HTTP services are running with
recurring sync uninstalled. This is local installation evidence, not proof of native GUI,
browser OAuth, updater, or operating-system trust behavior.

The same installed `0.32.0` core passed `eval --model` on 2026-08-14 in 16,649 ms with planner
and synthesis enabled, valid citations, cache reuse, revision invalidation, and no provider
fallback. This remains provider-backed fixture evidence: it does not query the personal index or
prove packaged GUI behavior. Query-only readiness also passed against the installed index with
database integrity, embedding/index generation, ACL, provider, API, and backup-freshness checks;
source validation was correctly not required and the recurring sync service remained uninstalled.

The v0.32.0 source retains the post-v0.31.6 Apple Notes executable hardening and
Buzz source-directory/log-size guards. The published archive evaluation above is
not packaged-GUI evidence.

The model fixture remains synthetic and does not authorize sources or recurring sync.

The current source tree also serializes Desktop settings and service-schedule writes through a
shared per-config lock, held across validation, backups, atomic replacement, and audit writing.
This protects concurrent Desktop windows/processes in the v0.32.0 source and package; it does
not authorize source ingestion or recurring sync.

The current source-tree evaluator also rejects oversized custom fixtures before parsing: the file,
document count, case count, document content, and query sizes are bounded. The direct JSONL import
path has its own bounded document, byte, wall-clock, and line-size limits. These protections are
resource-safety checks; they do not turn synthetic evaluation into a personal-index benchmark.

The built-in thresholds and data live in `eval/fixtures.json`. Use
`cortana eval --fixture /path/to/synthetic.json` for a versioned project-specific fixture. Never
put personal or production content into committed evaluation data.

## Read-only approved-index evaluation

The fixture gates above prove deterministic behavior and the bounded provider path, but they do
not measure retrieval quality on a real approved corpus. Cortana therefore ships
`scripts/evaluate-live-index.py`, a read-only HTTP harness for an operator-authored manifest of
queries and expected source IDs. It measures retrieval recall/MRR, ACL-safe source filtering,
answer citation validity, optional synthesized-mode use, latency, and a repeated-query cache hit.
It also reports embedding-retrieval degradation and provider-unavailable fallback rates so a
passing quality report cannot quietly treat a fallback path as provider-backed evidence.
It never calls ingestion, reconciliation, service management, backup/restore, or a source
connector. Reports contain only bounded metrics and source IDs; they do not echo queries, answers,
tokens, or provider error bodies.

Copy `eval/live-manifest.example.json` to a private, untracked file and replace its placeholders
with queries and expected IDs from an explicitly approved representative corpus. Run it against a
running query API with the smallest useful manifest:

```bash
cp eval/live-manifest.example.json /private/path/cortana-live-manifest.json
# Edit the private manifest with approved queries and expected/forbidden source IDs.
uv run python scripts/evaluate-live-index.py \
  /private/path/cortana-live-manifest.json \
  --base-url http://127.0.0.1:7331 \
  --require-synthesis
```

`--require-synthesis` expects the running API's query configuration to have synthesis explicitly
enabled; it does not modify configuration. Leave it off when evaluating the safe extractive
default, or use a separately approved temporary query configuration for provider-backed synthesis.

For a shared principal, pass its token through an environment variable rather than putting a
credential in the manifest or command history:

```bash
export CORTANA_EVAL_TOKEN='…'
uv run python scripts/evaluate-live-index.py \
  /private/path/cortana-live-manifest.json \
  --token-env CORTANA_EVAL_TOKEN
```

The harness bounds each request to 60 seconds, the complete run to five minutes, the manifest to
1 MiB, and the total case count to 100. A successful repeated query proves a cache hit only; live
cache invalidation is intentionally not attempted because mutating the approved corpus would
violate the read-only safety boundary. Keep the deterministic fixture's cache-revision test as
the invalidation gate, and record this live report separately for the approved principal and
workspace. A passing report is provider- and corpus-specific evidence, not permission to enable
recurring sync or optional Hindsight/Honcho memory.

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

The opt-in model gate applies a 55-second in-memory ceiling to the answer and provider request
timeouts, independent of the interactive query budget. It also caps the synthetic prompt context
at 2,048 tokens and the generated answer at 512 tokens, so a large personal query configuration
cannot turn this fixture-only check into a production load test. When the first planner or
synthesis call fails closed, the evaluator emits the failure report without repeating the same
provider outage for cache and post-update checks.

The current source applies that ceiling to the entire fixture run as well as to individual model
requests. A provider that ignores request-level cancellation therefore returns a stable deadline
failure instead of leaving the CLI waiting indefinitely; this outer guard is covered by a focused
regression test and does not enable synthesis in production.

The command exits nonzero when model quality thresholds fail.

When the configured endpoint is the model-gateway adapter, Cortana removes an attribution
footer only when the response carries the gateway's explicit provider header and the footer
matches the gateway's exact shape and provider token (including its documented `Gateway` display
suffix). Ordinary bullets, uncited text, and responses without that header remain unchanged and
continue through citation validation fail-closed.

### Historical provider-run notes (archived)

The run records below are retained for incident and migration history. They are not current
release evidence; use **Current release boundary** above for the v0.32.0 sign-off state.

An earlier configured-provider attempt at source commit `339240e` passed the bounded model gate in
20,176 ms, and a packaged v0.29.31 rerun passed in 13,477 ms. However, the installed v0.29.33
evaluator failed closed twice (8,313 ms and 13,398 ms) after the planner call because the configured
provider appended an uncited attribution line to the synthesis response. The earlier passes are
historical evidence, not proof that the current provider is citation-safe. After raising the bounded
synthetic output cap to leave room for gateway reasoning, the source run passed with planner and
synthesis model use, valid citations, cache reuse, and revision invalidation in 22,866 ms. The
latest successful installed v0.29.60 aarch64 core binary rerun passed the same fixture-only evaluator in
17,928 ms; a prior cache-warm run passed in 10,323 ms. Both runs had
planner+synthesis model use, valid citations, cache reuse, and revision invalidation. At that time,
the packaged Desktop app was not launched; the then-installed v0.30.10 core was used without
starting the app or recurring sync. v0.29.67 through v0.30.10 were release/version and
cross-platform compatibility fixes over the same query behavior.
An installed v0.30.10 run previously passed in 10,544 ms with planner and synthesis model use,
valid citations, cache reuse, and revision invalidation; retrieval recall, MRR, case pass rate, and
citation validity were all 1.0 within the 30,000 ms deadline. The earlier 13,117 ms and 9,105 ms
runs and older source-tree runs remain historical evidence. Earlier 2026-08-11 attempts failed
closed with `fallback_provider_unavailable=true` during transient configured-gateway outages. A
fresh bounded rerun on 2026-08-11 re-established provider availability: installed v0.30.10 passed
in 15,107 ms with planner and synthesis model use, valid citations, cache reuse, and revision
invalidation; the prior 13,472 ms run remains historical evidence from the same day. This is still
fixture-only evidence, not packaged-app proof; provider outages must
continue to fail closed, and extractive mode remains the safe production default.
The latest bounded `v0.31.0` archive rerun on 2026-08-12 passed in 18,312 ms with the same planner/synthesis,
citation, cache, and revision checks. The installed `v0.31.0` rerun passed in 18,979 ms with the
same result; the 15,107 ms result is now historical evidence. Retrieval quality remained perfect
(recall, MRR, case pass rate, and citation validity all 1.0) within the 30,000 ms answer deadline.
An additional operator probe on 2026-08-12 used a stricter 15-second wall-clock bound and timed
out without producing an evaluation report (exit 124). The provider's metadata endpoint remained
responsive and advertised 30 models, so this is recorded as a transient model-call latency
observation rather than a successful quality gate. It does not change the production configuration:
the evaluator remains opt-in and bounded, and extractive mode remains the safe default when the
provider is slow or unavailable.
Developer ID signing/notarization is not available in this environment. Extractive mode remains the
safe production default because synthesis is still an explicit opt-in in the production configuration.

At that point, the installed v0.31.1 core rerun on 2026-08-12 passed the same fixture-only
model gate in 24,546 ms: planner and synthesis were used, citations were valid,
cache reuse and post-update invalidation both passed, and recall, MRR, case pass
rate, and citation validity were all 1.0 within the 30,000 ms answer deadline.
That historical result did not prove packaged GUI behavior or authorize a personal-index sync. The v0.31.2
release archive and all 18 Desktop assets are independently verified, and the
v0.31.2 CLI was installed for that historical check, but its rerun exceeded the bounded operator window
and was stopped without a quality report. A prior source-tree run passed in
14,186 ms with the same planner/synthesis, citation, cache, and revision
checks. After raising the whole-run ceiling to 55 seconds while retaining the
under-one-minute fail-closed bound, the current source rerun passed in 11,866 ms
with planner and synthesis, valid citations, cache reuse, and post-update
invalidation. This source result does not prove packaged GUI behavior or
authorize a personal-index sync. No packaged-GUI evaluation is made.

The bounded source trial remains deliberately separate from this fixture gate. A one-document,
non-reconciling run completed for Personal Drive and Personal Gmail after a 180-second per-source
window, while their tighter 30-second operator probe expired during connector-to-embedding work.
The trial did not authorize recurring sync or reconcile indexed data.

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
