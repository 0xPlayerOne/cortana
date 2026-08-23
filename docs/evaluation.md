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

The current protected source and published tag are `v0.34.31`, promoted through the protected
staging → main flow and Release Please automation. Release-assets workflow `32642844805` published
the cross-platform package lanes; the 18-asset checksums, updater signatures, manifest, and
packaged-core verifier passed. The release does not change credentials, source authorization,
indexed data, recurring-sync policy, or native-memory behavior. The audited host now runs v0.34.31.
Query-only readiness,
`doctor`, deterministic `eval`, the strict release verifier, packaged-core verification, skill parity,
and the disposable native-memory/shared-agent/control-plane drills all passed. Recurring sync remains
uninstalled. A fresh v0.34.31 `eval --model` run also passed planner/synthesis execution, citation
validation, cache reuse, and revision invalidation in 13,834 ms without provider fallback; this is
synthetic fixture evidence only.

An explicit `readiness --allow-sync-service` check remains intentionally closed because the enabled
source set is not fully production-validated. The previous v0.34.13 pass used 25 documents, 5 MiB,
and 60 seconds per source: nine of 13 enabled non-code profiles returned `complete=true`, while
`personal-gmail` timed out after 59 seconds and the three Special Google profiles failed closed with
Google OAuth `invalid_grant`. After installing v0.34.15, a bounded Personal Gmail retry at the same
25-document/5 MiB scope completed within a 120-second cap, bringing the current bounded total to ten
complete profiles. These records make no index or reconciliation writes and do not meet configured
production budgets. Personal Drive's bounded probe passed after reauthorization, but its previous
full-budget run stalled on a large PDF and was stopped after 147 documents. Recurring sync remains
uninstalled, and no reconciliation or large sync has been requested.

The v0.34.20 Personal Drive follow-up confirms the distinction: validation passed at the
25-document/5 MiB/60-second smoke bound (234,160 bytes), while the separate non-reconciling trial
reached the 60-second safety bound and failed closed as `budget_exceeded`, with no deletions. This
is fresh failure/recovery evidence for the connector boundary, not a production-budget validation;
the source remains ineligible for reconciliation or recurring sync.

The full current-release bounded sweep on 2026-08-22 reinforces that gate: ten enabled non-code
profiles validated completely, while the three special Google profiles failed closed with
`invalid_grant`. Non-reconciling trials passed for Apple Notes, Gmail, Calendar, and Buzz with zero
deletions; Work Drive and Personal Drive both reached the 60-second safety bound and recorded
`budget_exceeded`. This is source-operational evidence only and remains below every configured
production budget.

An approved-corpus, read-only v0.34.20 evaluation against the local query API also ran with
synthesis explicitly enabled in an isolated temporary server. Scoped retrieval passed with
recall@k 1.0, MRR 1.0, hybrid retrieval, and no ACL leakage. The provider-backed answer request
timed out at the 25-second request bound on both the initial and repeated call, reported provider
fallback, and did not use synthesis or the answer cache. The overall provider-backed gate therefore
failed closed while the deterministic extractive default remains healthy; this is current live
operational evidence, not a quality pass or authorization to enable provider synthesis by default.

The older production-budget results below are retained as historical evidence, not current
authorization. They document prior successful prefixes and failure/recovery behavior, but the
on-disk validation record and its configuration fingerprint are the authoritative gate.
The earlier Work Drive non-reconciling trial was cancelled under the installed v0.32.1 binary after
the embedding service stalled; it made no deletions. After installing v0.32.2, a new foreground
`sync --source work-drive --no-reconcile --require-validation` attempt progressed through bounded
unchanged batches, then the local embedding health probe timed out while queued embedding work was
still completing; the operator cancelled it after roughly seven minutes. That failed attempt is
historical: the service recovered afterward, and a subsequent v0.32.4 bounded retry completed 100
unchanged documents with no deletions. The source still lacks a complete 478-document production-
budget trial. The earlier Personal Drive production-budget validation failed closed after its
899-second connector timeout. On 2026-08-15 a bounded
25-document/5 MiB/60-second validation succeeded (25 documents, 167,848 bytes), followed by a
non-reconciling trial with `changed=1`, `unchanged=24`, and `deleted=0`. This proves the bounded
connector path only; it remains below the configured production budget and does not authorize
recurring sync. Special Gmail completed production-budget validation with 214 documents and
995,335 bytes, followed by a 100-document-cap non-reconciling trial with `deleted=0`. Personal
Gmail completed production-budget validation with 430 documents and 1,563,456 bytes, followed by
the same 100-document-cap non-reconciling trial with `deleted=0`. These capped trials remain below
complete production trials, so the recurring gate remains
correctly closed until every enabled source has fresh complete validation and a successful bounded
trial.

On 2026-08-15 a second bounded Work Drive retry emitted all 478 connector records, then failed
closed when the local embedding connection closed during ingestion. It was non-reconciling and made
no deletions; controlled ingestion may retain the completed prefix. The embedding supervisor
restarted the router, and query-only readiness passed afterward. A subsequent v0.32.4 run with a
100-document/16 MiB/300-second bound completed `changed=0`, `unchanged=100`, and `deleted=0`.
This closes the bounded retry observation but not the complete 478-document production trial or
the recurring-sync gate.
The v0.34.28 release gate verifies the packaged core offline without credentials. The fresh
provider-backed `cortana eval --model` run against the installed v0.34.28 binary passed retrieval
recall, MRR, case pass rate, citation validity, planner and synthesis execution, cache reuse, and
revision invalidation in 15,279 ms under the 55,000 ms bound, with no provider fallback. This is
synthetic fixture evidence only: it does not query the personal index, prove packaged GUI behavior,
or authorize source synchronization. The approved-corpus provider-backed evaluation remains an open
operator gate; the evaluator remains opt-in and extractive mode remains the production default.

The fresh bounded approved-index evaluation on the installed v0.34.20 query API is recorded
separately from the fixture gate. It preserved workspace/source isolation, citation validity,
cache reuse, and an extractive answer with recall@k 1.0, but the expected source ranked fourth
(MRR 0.25), below the production threshold. An isolated synthesis-enabled attempt also timed out
at the 30-second request ceiling, so no provider-backed personal answer was accepted. Do not
enable production synthesis or treat this run as full-corpus quality evidence until a repeatable
approved-corpus evaluation meets the retrieval, citation, synthesis, latency, and fallback gates.

A subsequent private, read-only v0.34.20 evaluation against the approved `work` / `work-notes`
scope passed the retrieval-only thresholds: recall@k 1.0, MRR 1.0, retrieval pass rate 1.0,
zero forbidden-source leaks, hybrid retrieval without degradation, repeated-query cache hit rate
1.0, and a 158 ms maximum request latency. The manifest and query remain untracked and the report
contains only bounded metrics and source IDs. This strengthens current retrieval and folder-scope
evidence, but it is not provider-backed synthesis evidence and does not authorize recurring sync.

The audited host now runs `/Users/amf/.local/bin/cortana` v0.34.28. The embedding and HTTP services
are running; readiness confirms a fresh verified backup while the backup scheduler and recurring sync
remain uninstalled. The isolated `/healthz` and `/readyz` probes,
`doctor`, and `readiness --max-backup-age-hours 48` passed after the upgrade. This is local
installation evidence, not proof of native GUI, browser OAuth, updater, or operating-system trust
behavior.

On 2026-08-22, the installed v0.34.22 binary passed the disposable native-memory lifecycle drill
(dedupe, recall, expiry, export, and forget), the scoped HTTP authorization drill (query/status/admin
scope separation, ACL filtering, metadata-only audit, token rotation, and revocation), and the real
MCP stdio drill (10 tools, workspace ACL filtering, and token rotation). The same binary passed the
offline Desktop control-plane drill (bounded ingest, hybrid search/context, metadata-only audit,
verified backup, restore, SQLite verification, and post-restore search). These are synthetic,
temporary-directory control-plane evidence and do not replace native packaged-GUI acceptance or
live approved-corpus evaluation.

The preceding v0.32.12 fixture run from 2026-08-16 remains historical evidence: it passed
retrieval recall, MRR, case pass rate, citation validity, planner and synthesis execution, cache
reuse, and revision invalidation in 14,437 ms with no provider fallback. It does not query the
personal index, prove packaged GUI behavior, or authorize source synchronization.

Historical v0.32.6 evidence: the then-installed core passed `eval --model` on 2026-08-15 in
18,178 ms with planner and
synthesis enabled, valid citations, cache reuse, revision invalidation, and no provider fallback.
This remains provider-backed fixture evidence: it does not query the personal index or prove
packaged GUI behavior. Query-only readiness passed against the installed index with database
integrity, embedding/index generation, ACL, provider, API, and backup-freshness checks; the
separate `--allow-sync-service` gate fails closed because `personal-drive` has only a bounded
25-document/5 MiB validation after its production-budget connector timeout, against the configured
2,000-document, 128 MiB budget, so the recurring sync service remains uninstalled.

After that evaluation, a source-scoped live pass rechecked all enabled non-code sources with
validation-required, non-reconciling 25-document/5 MiB/60-second caps. Apple Notes, calendars,
Drive, Gmail, and Buzz all completed without deletions; Special Calendar naturally returned zero
records. The index ended at 12,123 documents/42,638 chunks and query-only readiness passed. This
is operational ingestion evidence, not a full-corpus quality benchmark. That historical pass
used smaller bounds than the configured budgets; the current host status now reports 10 of 13
enabled sources complete within the refreshed bounded limits, while the three Special Google
sources require reauthorization and Personal Drive still lacks a production-budget validation.
The pass did not enable Discord, code, Slack, or synthesis.

On 2026-08-22, the installed v0.34.22 binary revalidated all three configured Apple Notes folder
scopes with the bounded source-smoke command: `work-notes`, `personal-notes`, and `special-notes`
all passed validation at the 25-document/5 MiB/60-second bound. The companion non-reconciling
trial measurements were captured immediately before the metadata-only v0.34.20 release against
the same application content: `work-notes` returned 25 documents (118,540 bytes), `personal-notes`
returned 25 (89,645 bytes), and `special-notes` returned 8 (14,046 bytes), with zero deletions.
Together these results confirm workspace folder routing and the current connector path, but remain
below the configured production budgets and do not authorize recurring sync.

The v0.34.28 source retains the post-v0.31.6 Apple Notes executable hardening and
Buzz source-directory/log-size guards. The published archive evaluation above is
not packaged-GUI evidence.

The model fixture remains synthetic and does not authorize sources or recurring sync.

The current v0.34.28 source tree also serializes Desktop settings and service-schedule writes through a
shared per-config lock, held across validation, backups, atomic replacement, and audit writing.
This protects concurrent Desktop windows/processes in the v0.34.28 source; it does
not authorize source ingestion or recurring sync.

The v0.34.28 source adds bounded embedding-supervisor recovery: steady-state health checks
avoid queueing vector requests, while startup and restart still require a real vector probe. The
cancelled v0.32.2 Work Drive trial is an operational throughput observation, not a failed
retrieval-quality result; a longer bounded retry is required before advancing that source gate.

### Prior approved-index retrieval evidence (2026-08-22 UTC; v0.34.11 API)

The run timestamp is normalized to 2026-08-22 UTC (the local wall clock was the evening of
2026-08-21). A private, one-case manifest was run against the then-installed v0.34.11 query API using
an approved work Apple Notes scope. The read-only harness passed hybrid retrieval and the
extractive answer path with recall@k 1.0, MRR 1.0, retrieval and answer pass rates 1.0, citation
validity 1.0, zero retrieval or provider fallback, a repeated-query cache-hit rate of 1.0, and a
326 ms maximum request latency. The runtime correctly reported `synthesis_used = false`; this is
current live retrieval/extractive-answer evidence, not provider-backed synthesis evidence. The
report contained only bounded metrics and source IDs; the manifest and query text were not
committed. It does not authorize sync, prove full-corpus source validation, or close the separate
provider-backed synthesis, shared-agent, or packaged-GUI gates.

### Historical approved-index retrieval evidence (2026-08-14)

A private, one-case manifest was run against the local query API using the approved `work` /
`work-gmail` index scope. The harness passed with hybrid retrieval, recall@k 1.0, MRR 1.0,
retrieval pass rate 1.0, zero retrieval degradation, zero forbidden-source leaks, a repeated
query cache-hit rate of 1.0, and a 1,750 ms maximum request latency. The report contained only
bounded metrics and source IDs; the manifest and query text were not committed. This is
read-only live-index retrieval evidence. It does not authorize sync, prove full-corpus source
validation, or close the separate provider-backed answer/synthesis, shared-agent, or packaged-GUI
gates.

A separate temporary synthesis-enabled API attempt against the same approved scope failed closed at
the 45,000 ms request ceiling: no synthesized answer or citations were returned, no forbidden
source IDs leaked, and the provider-unavailable fallback flag remained false. This is a bounded
provider-latency failure record, not provider-backed quality evidence. Production synthesis remains
disabled and extractive mode remains the safe default until a repeatable cited-answer run passes.

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
Answer cases enforce the requested source scope against returned evidence as well as expected and
forbidden source IDs, so a cited answer from another connector cannot pass the evaluation.
It never calls ingestion, reconciliation, service management, backup/restore, or a source
connector. Reports contain only bounded metrics and source IDs; they do not echo queries, answers,
tokens, or provider error bodies.

Copy `eval/live-manifest.example.json` to a private, untracked file and replace its placeholders
with queries and expected IDs from an explicitly approved representative corpus. Run it against a
running query API with the smallest useful manifest:

Answer cases may also include a bounded `required_answer_terms` list. Terms are matched
case-insensitively against the provider answer, while reports expose only checked and missing
counts so approved answer text and evaluation terms are not echoed into artifacts.

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
recurring sync or implicit memory writes.

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
release evidence; use **Current release boundary** above for the v0.34.28 source and package
state.

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

The bounded source trial remains deliberately separate from this fixture gate. Personal Drive first
failed its 2,000-document/128 MiB/900-second production-budget validation after the connector
timed out at 899 seconds. A follow-up 25-document/5 MiB/60-second validation and non-reconciling
trial completed with `changed=1`, `unchanged=24`, and `deleted=0`; this bounded prefix does not
authorize recurring sync or reconcile the full indexed corpus. Earlier one-document trials for
Personal Drive and Personal Gmail used a 180-second per-source window while their tighter
30-second operator probes expired during connector-to-embedding work; those historical trials also
did not authorize recurring sync or reconcile indexed data.

On 2026-08-16 a separate Personal Drive validation-only probe completed 100 documents at
390,182 bytes and a 300-second cap with zero index writes. It remains under the configured
production budget and is recorded as connector evidence only; the recurring-sync gate stays
closed until a complete production-budget validation succeeds.

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
