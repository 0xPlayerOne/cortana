# Evaluation

Cortana evaluation turns product, retrieval, memory, source, security, Desktop, and release claims into repeatable evidence. This document defines durable methods and evidence rules; it does not track the current pass/fail state of milestones.

Current evaluation work belongs in [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues). Version-specific evidence belongs in [Release history](releases.md) or the owning issue.

## Principles

- Evaluate the exact contract being claimed.
- Keep deterministic CI separate from private approved-corpus and packaged-app evidence.
- Pin revisions, provider identity, configuration, and evaluation contract.
- Never commit personal source content, private queries, credentials, tokens, or absolute paths.
- Treat ACL leakage, invalid citations, unsafe deletion, and unbounded work as hard failures.
- Record degradation and fallback explicitly.
- Compare changes against a pinned baseline rather than relying only on aggregate averages.
- Distinguish product regression, corpus change, manifest change, provider variance, and environment variance.

## Evaluation lanes

### Deterministic core

Run against a temporary SQLite database with synthetic fixtures and a deterministic embedder.

Cover:

- source/project scope;
- ACL denial;
- semantic and lexical candidate behavior;
- reciprocal-rank fusion and stable ordering;
- exact identifiers, phrases, paraphrases, distractors, stopwords, and stale results;
- canonical-source deduplication;
- neighboring context;
- token budgets and omission accounting;
- citations;
- cache reuse and revision invalidation;
- embedding fallback;
- provider-independent extractive answers;
- latency and resource bounds.

This lane may run in CI and must never open the configured personal index or contact a live source.
The repository-owned `bun run test:eval` command executes the deterministic Rust evaluator against
the pinned synthetic fixture; it is part of `test:unit` so every validation run exercises the M5
quality gate without contacting a provider or source.

The same command also runs the provider-free knowledge-graph gate from
`eval/knowledge-graph-v1.json`. The evaluator creates 25 workspaces, 100 sources, and 2,500
documents and chunks in a disposable store, then exercises the real router and graph projection.
It fails on ACL disclosure, missing canonical containment, invalid provenance, stale relationships
after mutation, disabled-edge exposure, or a breached versioned latency, response-size, index-size,
CPU, wall-clock, or memory threshold. Run only that integration contract with:

```bash
cortana eval --knowledge
```

The report records the exact fixture digest, platform, applied thresholds, p50/p95 latency,
response bytes, store/index bytes, peak RSS, CPU and wall time, and sanitized failure reasons. The
fixture is synthetic release-regression evidence; it is not an approved private corpus, a live
connector trial, an interactive packaged-GUI review, or an operating-system trust result.

The synthetic relationship metrics are correctness and safety evidence for the provider-free graph
projection, not proof that graph-assisted workflows improve a governed corpus. Prepare the separate
approved relationship lane with
[`eval/relationship-quality-private.example.json`](../eval/relationship-quality-private.example.json).
It covers explicit links, threads, temporal relations, entities, backlinks, semantic neighbors,
contradictions, supersession, and code dependencies, plus document-only/search-only control tasks
for navigation, explanation, retrieval, and memory review.
The raw cases remain external; only sanitized case outcomes, task success, navigation-step
reduction, retrieval lift, provenance, invalidation, ACL, false-inference, latency, and memory
metrics may be recorded.

The relationship record must include bounded quality, safety, and resource metrics for each
relationship case, not only an aggregate. Every control-versus-treatment task must also meet the
configured task-success and retrieval-lift thresholds. Its edge-kind union must exactly match the
enabled and optional release-policy edge kinds, and those policy sets must be duplicate-free and
disjoint, so an aggregate result cannot hide an unmeasured or ambiguously classified released edge
type.

Verify a completed external evidence record without loading its raw corpus:

```bash
uv run python scripts/verify-relationship-evidence.py /path/to/approved-relationship-evidence.json
```

Before collecting private results, validate the checked-in contract or an operator-owned copy
without approving or promoting it:

```bash
uv run python scripts/verify-relationship-evidence.py --preflight eval/relationship-quality-private.example.json
```

Preflight requires every case and task to remain `not-run`, all measured values to remain null, and
the release policy to keep graph-assisted behavior optional. It emits `preflight_passed: true` with
`promotable: false`; it is preparation evidence, not a relationship-quality result.

The verifier requires all nine relationship categories and all four control-versus-treatment task
records, checks the zero-disclosure and resource thresholds, and emits only bounded aggregate
evidence. Each record includes the release version it evaluated, and the verifier binds it to the
current Cargo project release by default. Use `--version` only when deliberately verifying an
explicitly historical record. The checked-in `not-run` template intentionally fails verification.
A passing report cannot authorize graph activation, enable optional edge derivations, or make the
graph a requirement for search or exact document access.

### Approved-corpus retrieval

Run read-only against an explicitly approved local index and private manifest.

The manifest defines:

- case identifier;
- workspace and source scope;
- expected evidence identifiers;
- forbidden source or workspace identifiers;
- query class;
- acceptable evidence set;
- latency and budget limits;
- whether memory may participate;
- whether the case is retrieval-only, extractive-answer, or synthesis-enabled.

The baseline harness accepts three read-only case classes: `retrieval_cases` call `/v1/search`,
`context_cases` call `/v1/context` and verify bounded inclusion/omission metrics, and
`answer_cases` call `/v1/answer` and validate citations against returned evidence. Context cases
must provide a `max_tokens` value between 256 and 64,000. A case may also declare
`forbidden_projects` and `forbidden_sources`; any returned matching scope is a hard failure.
Each successful context case is repeated once within the operator budget. The report records only
boolean `digest_reused` and `content_unchanged` outcomes, so unchanged pinned inputs can be
measured without retaining query or context content.
Evidence responses do not expose project labels in every API version, so scope checks are
enforced when labels are present and the request's authenticated ACL remains authoritative.

Raw queries and source content remain outside the repository. Reports contain only bounded metrics and non-secret identifiers.

The checked-in `eval/live-manifest.example.json` is a schema template only. Replace its
placeholders in an operator-controlled local or encrypted manifest; do not commit real queries,
source IDs, or corpus content.

Before opening the approved index, validate the external manifest without contacting any service:

```bash
uv run python scripts/evaluate-live-index.py --validate-only /path/to/approved-manifest.json
```

Manifest version 2 binds the manifest to `release_version`. By default the validator requires that
version to match the current Cargo release; use `--version X.Y.Z` only when intentionally replaying
an explicitly historical record. The required corpus provenance block rejects a future `approved_at`
and, when supplied, an `expires_at` that is no longer active. The preflight report contains only
case counts, bounded resource thresholds, release/contract/scope provenance, and non-secret corpus
provenance. It records `index_contacted: false`. Run the live evaluator only after the operator has
approved the manifest and read-only index:

```bash
uv run python scripts/evaluate-live-index.py /path/to/approved-manifest.json
```

### Provider-backed answers

Evaluate optional planners, rerankers, and synthesizers against synthetic and approved-corpus cases.

Measure:

- planner usefulness;
- evidence recall after expansion;
- answer correctness;
- paragraph citation completeness;
- unknown or unauthorized citations;
- latency and timeout;
- provider cost or usage where available;
- cache reuse;
- fallback;
- malformed output;
- provider outage;
- privacy disclosure and opt-in behavior.

Every accepted factual paragraph must cite returned authorized evidence. Provider failure must return an explicit bounded fallback without blocking core retrieval.
The model-backed report emits an activation record containing the sanitized provider authority,
exact model, API/retrieval contract versions, pinned corpus revision, and a digest of the bounded
report. `activated` is true only when the opt-in gate passes; provider paths, credentials, and raw
queries are never recorded.

### Source operations

Use synthetic connectors and separately approved live source trials.

Cover:

- authorization and discovery boundaries;
- validation with zero writes;
- document, byte, time, response, spool, and concurrency budgets;
- cursor behavior;
- transient retry;
- cancellation;
- completed-prefix retention;
- configuration fingerprints;
- complete versus partial snapshots;
- reconciliation dry-run and deletion safety;
- source isolation;
- embedding reuse;
- interactive query availability during ingestion;
- recurring-policy prerequisites.

A partial, failed, cancelled, timed-out, sampled, capped, or non-reconciling operation must never gain deletion authority.

### Native memory

Cover explicit memory first:

- retain/remember;
- idempotent retry;
- recall;
- expiry;
- supersession;
- redaction/forget;
- export;
- backup and restore;
- memory revision and cache invalidation;
- ACL and workspace isolation.

For memory intelligence, separately evaluate:

- candidate precision and recall;
- duplicate suppression;
- reinforcement and contradiction classification;
- supersession correctness;
- approval load;
- retention accuracy;
- reflection grounding;
- derived-representation provenance and invalidation;
- sensitive-data handling;
- provider failure.

Automatic retention remains independently disableable and cannot be activated without reproducible quality and safety evidence.

### Shared-agent authorization

Use disposable synthetic indexes and principals.

Cover:

- local owner versus bearer principal;
- query, status, memory, and admin scopes;
- ACL intersection;
- evidence and memory isolation;
- cache isolation;
- token rotation and revocation;
- HTTP reload;
- MCP file-backed rotation;
- metadata-only audit;
- remote-listener safeguards;
- legacy/public-row quarantine.

Zero unauthorized or cross-workspace disclosure is required.

### Agent HQ and Control Plane integration

Use disposable integration environments.

Cover:

- workspace-to-project mapping;
- least-privilege principal resolution;
- RuntimeNode/local-service transport;
- request and response bounds;
- ContextBundle version, digest, scope, revisions, and degradation validation;
- ContextPackage incorporation;
- immutable ExecutionPlan pinning;
- replay;
- node offline and reconnect;
- revoked credentials;
- stale revisions;
- failure policy;
- separate ProjectState and Cortana memory effects.

No service may read another service's database or raw credentials.

### Packaged Desktop

Run real packages on every supported operating system and architecture claim.

Cover:

- clean install and first run;
- tooling approval;
- workspaces;
- provider setup;
- authorization;
- validation and trial preparation;
- services, tray, background, and autostart;
- native dialogs;
- backup and restore;
- updater and restart;
- accessibility;
- security;
- large-corpus behavior;
- recovery;
- uninstall;
- operating-system trust.

Headless tests and static package verification do not substitute for this lane.

The checked-in [`eval/desktop-acceptance-private.example.json`](../eval/desktop-acceptance-private.example.json)
is a sanitized schema template for recording the manual lane outside repository history. A separate
copy must contain one approved record for each supported target and all ten cases covering clean
install, source authorization, services/tray, backup/restore, updater, recovery, accessibility,
large-list resources, uninstall, and operating-system trust. Raw screenshots, logs, queries, private
paths, and credentials remain in the external encrypted evidence store. Verify the sanitized record
without loading those artifacts:

```bash
uv run python scripts/verify-desktop-acceptance-evidence.py /path/to/approved-desktop-evidence.json
```

Before collecting platform results, validate the three-target contract without approving or
promoting it:

```bash
uv run python scripts/verify-desktop-acceptance-evidence.py --preflight eval/desktop-acceptance-private.example.json
```

Preflight requires every supported target and all ten acceptance cases to remain `not-run`, permits
the template placeholder digests, and emits `preflight_passed: true` with `promotable: false`. It
does not provide packaged, manual, screen-reader, or operating-system evidence.

The verifier also checks query-only first-run behavior, absence of implicit side effects, package
identity, and per-platform startup, CPU, memory, large-list, and graph timing thresholds. The
checked-in `not-run` template intentionally fails verification. By default it binds the record to
the current Cargo project release; use `--version` only when deliberately verifying an explicitly
historical package record.

The sanitized record also names the complete supported target set and explicitly lists unsupported
targets. This prevents a three-platform result from being read as an unbounded claim about other
operating-system or architecture combinations.
Its top-level case matrix enumerates the required subchecks behind each of the ten platform cases;
each platform's evidence IDs must cover that same matrix before the record can be approved.

The repository's `bun run test:knowledge-accessibility` gate is a deterministic renderer check for
the knowledge/document-browser and graph surfaces. It uses the provider-free demo fixture and
headless Chromium to exercise keyboard navigation, virtualized document opening, canonical content
and provenance, graph selection, accessible names, WCAG 2.2 AA axe rules, reduced motion, 200% zoom,
responsive layouts, and browser-console cleanliness. CI builds `apps/web/dist` first and runs the
same command with `CORTANA_KNOWLEDGE_SERVER=preview`, so its uploaded JSON report and screenshots
cover the production Vite renderer rather than the development server. Local iteration may omit
that variable to use Vite development mode. The report also records provider-free demo-fixture
resource samples with p50 and p95 navigation/document/graph timings, peak request and response bytes,
DOM and visible-node counts, and Chromium heap observations against explicit browser ceilings. The
production-preview run includes a paginated 2,500-document fixture and verifies that document
rendering stays bounded at no more than 100 visible list rows and 200 graph nodes. These measurements
gate renderer regressions and document the observation method, but do not replace approved
large-corpus, supported-platform package launch, or manual VoiceOver, NVDA, or equivalent review.
The acceptance runner waits for two browser animation frames after each viewport change so media-query
assertions and screenshots observe the settled layout instead of stale geometry during rapid responsive
resizes.

The published-package acceptance workflow also extracts the exact release bundle's
`share/cortana/web` directory and runs the standard document/graph acceptance against that immutable
renderer through a loopback static server. The current `v0.56.9` bundle predates the provider-free
large-corpus fixture, so the workflow sets `CORTANA_KNOWLEDGE_RUN_LARGE=false` for that release and
records the release-compatible document/graph screenshots and resource measurements. The current
source/preview lane sets the large fixture explicitly; a future published bundle that contains it
may set `CORTANA_KNOWLEDGE_RUN_LARGE=true`. That lane still checks keyboard operation, responsive layout,
axe, provenance, graph controls, console cleanliness, screenshots, and resource observations; a
failure is written as `report.json` and remains a failure. It does not convert the current source
renderer report into evidence for an older immutable package.

When collecting local current-source package evidence, set
`CORTANA_ACCEPTANCE_PROVENANCE=prospective-source`; the renderer and Desktop acceptance helpers then
label their reports as prospective source evidence. The published workflow explicitly sets
`CORTANA_KNOWLEDGE_INSTALLATION_TYPE=published-package-renderer` for its immutable bundle, and the
aggregate release matrix accepts only the published installation types, so a prospective report
cannot accidentally satisfy an immutable-release matrix.

The headless packaged-core lane also downloads `latest.json` and binds the target-specific updater
entry to the requested release version, HTTPS URL, non-empty signature, and expected application
archive. Cryptographic signature verification remains part of the release-assets verifier; this
package lane prevents a target from passing with detached or stale updater metadata.

The aggregate Desktop matrix keeps each operating-system artifact in a separate directory because
renderer reports intentionally use the same relative `knowledge-accessibility/report.json` path.
It also requires the renderer report revision to equal the requested immutable release tag and
requires exactly the knowledge, document, and graph axe surfaces. It also verifies the exact six
document and six graph viewport records plus every referenced screenshot as a non-empty file within
the report directory, together with the complete latency p50/p95, request/response, DOM, visible-node,
heap, sample-count, and threshold records. A working-tree renderer report, incomplete viewport
matrix, mismatched screenshot manifest, missing measurement, missing image, or path traversal
therefore cannot be combined with published package records to produce a release pass.
The renderer fixture must be one of the approved provider-free fixtures; a large-corpus fixture must
also contain its bounded-rendering case and both large-corpus screenshots.

On macOS, the workflow requires a native lifecycle report for the packaged app's status item,
close-to-tray behavior, and tray reopen. That supplemental report is surfaced in the matrix but does
not replace the required renderer, source-authorization, service, dialog, updater, recovery,
accessibility, or OS-trust lanes.

For Unix release archives, the same workflow runs
`scripts/desktop-install-acceptance.mjs` against the extracted root `install.sh` in a disposable
prefix. On Windows it runs the same harness against the published MSI with silent, non-restarting
install and uninstall, and verifies the installed desktop executable, core sidecar, web assets, and
offline core evaluation. These lanes record the installed binary/version, explicitly installed
connector where applicable, query-only defaults, absence of source or service-schedule side effects,
and bounded cleanup with an explicit removed-state result. They do not exercise services, OAuth, native dialogs, updater lifecycle,
recovery UI, interactive first-run UI, accessibility, or operating-system trust; they also record
that installer dependency installation may use package-manager network access while provider-backed
work is not requested.

### Release trust

Verify:

- source tag;
- core binary;
- Desktop bundle;
- connector package;
- web assets;
- manifests;
- checksums;
- updater signatures;
- nested binary signatures;
- provenance/SBOM where required;
- installed version;
- notarization or platform trust;
- upgrade and rollback.

Package integrity and OS trust are separate claims.

## Metrics

### Retrieval and answer quality

- recall@k;
- mean reciprocal rank;
- source-level diversity;
- citation validity;
- citation completeness;
- answer pass rate;
- forbidden-source leak count;
- insufficient-evidence accuracy;
- fallback correctness;
- duplicate-source crowding.

The approved-corpus report records deterministic latency p50/p95/p99, source diversity, duplicate
source crowding, lexical fallback rate, answer cache reuse, citation failures, forbidden-scope
leaks, and context token inclusion/omission and budget compliance. These are diagnostic baseline
measurements; later retrieval changes compare against the same corpus and manifest revisions.

### Memory quality

- candidate precision/recall;
- classification accuracy;
- duplicate suppression;
- contradiction detection;
- supersession correctness;
- retention and expiry accuracy;
- recall quality;
- reflection grounding;
- derived-representation invalidation;
- unauthorized-memory count.

Run the checked-in, provider-free native-memory gate with:

```bash
cortana eval --memory
```

The versioned `eval/memory-intelligence-fixtures.json` suite uses disposable stores and synthetic
preferences, decisions, contradictions, stale working state, sensitive-data rejection, and
cross-workspace distractors. It reports observed per-case candidate/model/policy/corpus failure
domains with stable sanitized reason codes; nested policy failures retain their policy attribution
instead of being relabeled as the case's expected domain. Failed capability comparisons carry the
same attribution and contribute to the aggregate failure-domain counts. Untagged store, SQLite, or
evaluation-pipeline errors fail closed as sanitized corpus/infrastructure failures. The report also includes
explicit-only baselines, each proposed capability's comparison, latency, provider requests,
estimated provider cost, and measured disposable-store count and bytes. Candidate precision and
recall come from the labeled safe/sensitive/cross-workspace proposal confusion matrix; provider
requests and approval load are counted from the operations actually executed. Every gating fixture
must contain the complete ordered canonical case registry with its fixed category and failure
domain. A custom fixture may tighten thresholds but cannot omit cases or weaken quality, privacy,
latency, cost, or resource bounds. Synthetic success can enable release claims for manual candidate
review, deterministic classification, approval-gated consolidation, reflection, and derived
inspection; it cannot authorize automatic retention.

The synthetic contents cover a durable preference, semantic decision contradiction and
supersession, procedural consolidation, repeated episodic experience deduplication, stale working
state, sensitive rejection, and a cross-workspace distractor.

Each automatic capability comparison runs in its own disposable store, first writes an explicit
baseline memory, then proves the capability works without deleting that baseline or performing an
unapproved canonical write. Its approval work, provider use, store bytes, and latency are included
in the aggregate gates alongside the canonical cases. The reproducible report digest covers fixture identity, deterministic
case outcomes, quality/safety metrics, baseline state, and comparisons. Observational wall-clock
latency and store-byte measurements remain in the report and gates but are deliberately excluded
from the digest, so identical outcomes have the same evidence identity across machines.

The checked-in `eval/memory-intelligence-private.example.json` is a non-runnable governance
template for the separate approved private lane. Raw private cases stay outside repository history.
An authorized reviewer fills the opaque corpus revision, case results, approval load, latency,
CPU/RSS, and cost fields after running against a disposable private store. Even a passing private
report is evidence only: automatic retention remains disabled until a separate reviewed runtime
policy change is approved and reproducibly linked to both report digests.

Verify a completed external evidence record without loading its raw corpus:

```bash
cortana eval --memory-private-evidence /path/to/approved-private-evidence.json
```

The verifier requires all eight canonical private case categories, zero exposure, unsupported
claims, and automatic retentions, complete bounded quality/resource metrics, non-secret governance
metadata, and external encrypted raw-data storage. It emits only counts, the opaque corpus revision,
and a stable evidence digest. Private evidence is capped at 60 CPU seconds, 2 GiB peak RSS, 5 seconds
p95 latency, 16 provider requests, and $1 estimated provider cost. The checked-in `not-run` template
intentionally fails verification.

### Performance and economics

- p50/p95/p99 latency where relevant;
- startup time;
- CPU and memory;
- response bytes;
- index size;
- embedding time;
- source throughput;
- cache hit rate;
- unchanged-content reuse;
- provider request avoidance;
- context reduction;
- estimated returned tokens;
- cancellation cleanup.

### Reliability and safety

- ACL leak count;
- invalid accepted citation count;
- unbounded-operation count;
- unauthorized deletion count;
- backup verification;
- restore correctness;
- retry idempotency;
- source-isolation failures;
- crash/restart recovery;
- updater rejection correctness.

## Thresholds

Thresholds belong in versioned evaluation configuration or the owning issue, not prose that drifts from implementation. Every report records:

- evaluation contract version;
- source tree or release;
- corpus/manifest revision;
- corpus and memory revisions where applicable;
- embedding fingerprint;
- retrieval contract;
- provider endpoint class and model identifier without secrets;
- platform and architecture;
- applied thresholds;
- pass/fail reasons.

Safety thresholds such as ACL leaks, unauthorized deletion, and accepted invalid citations are zero unless an ADR explicitly changes the product contract.

## Product-claim evidence matrix

| Claim                             | Deterministic gate                                                            | Private/manual gate                                            | Hard failure                                                                           |
| --------------------------------- | ----------------------------------------------------------------------------- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Canonical entities and migrations | Rust store/model contract tests and migration fixtures                        | Backup/restore drill for the supported release                 | Lost canonical field or unrecoverable migration                                        |
| ContextBundle pinning             | Digest, revision, scope-isolation, and compatibility tests                    | Agent replay against an approved manifest                      | Accepted stale, mismatched, degraded, or unauthorized bundle                           |
| Connector safety                  | JSONL certification, budgets, cancellation, and reconciliation tests          | Bounded source trial for each account/source                   | Deletion after partial/stale/failed run                                                |
| Memory lifecycle                  | Remember/recall/expiry/forget/supersession/ACL tests                          | Approved-corpus memory quality and review-load evidence        | Unauthorized recall/write or ungrounded automatic write                                |
| Knowledge graph relationships     | Synthetic edge correctness, provenance, ACL, invalidation, and resource tests | Approved relationship cases and control-vs-graph task evidence | False/inferred edge disclosure, stale edge, ACL leak, or graph-required core retrieval |
| Public API compatibility          | HTTP/MCP/CLI schema snapshots and envelope tests                              | Disposable client/provider conformance run                     | Credential/path disclosure or incompatible unannounced change                          |
| Desktop trust                     | Headless native tests and package verifier                                    | Real install, updater, OS trust, and accessibility acceptance  | Package/runtime mismatch or unsafe native privilege                                    |

Every M2 contract names its deterministic fixture and its separate private/manual gate. A green CI
run is not evidence that a live source, private corpus, packaged GUI, or operating-system trust gate
has passed.

## Private manifest governance

Private manifests must be:

- stored locally or in an approved encrypted location;
- accessible only to authorized reviewers;
- excluded from repository history;
- redactable and deletable;
- versioned independently from product code;
- free of reusable credentials;
- reported through non-secret case and evidence identifiers.

### Governance contract

The checked-in `eval/live-manifest.example.json` is a transport-safe template,
not a usable personal manifest. An operator-owned manifest must include a
`governance` object with `contract_version: cortana.approved-corpus.v1` and
the following controls:

- `scope.workspaces`, `scope.sources`, and `scope.forbidden_sources` are
  opaque identifiers. A case may only name an allowed workspace/source, and
  `scope.memory` explicitly says whether native memory participates.
- `coverage` records the minimum number of cases for each representative
  workspace/source pair. Start with notes, Drive, Gmail, Calendar, Buzz, and
  memory when those connectors are enabled; add later connectors as separate
  coverage entries rather than silently treating an untested source as
  covered.
- Every case has a stable `id`, expected and forbidden evidence identifiers,
  and one explicit mode: `retrieval-only`, `extractive-answer`, or
  `provider-synthesis`. Answer cases may additionally set bounded
  `answer_criteria.required_terms`, `min_citations`, and `allow_abstain`.
- `resource_bounds` pins request/total latency, response bytes, case count,
  and an operator memory ceiling. The evaluator clamps its runtime limits to
  these values; a threshold cannot grant an unbounded run.
- `storage` is `local` or `encrypted-local`; credentials must remain
  external. `reviewer_access` names authorized reviewer identifiers and
  requires explicit approval; the corpus reviewer must be one of those
  identifiers. Reviewer IDs are not emitted in reports.
- `lifecycle` records retention days, operator/reviewer-confirmed deletion,
  controlled redaction, and stop/revoke incident handling. These are required
  governance decisions, not prose-only recommendations.
- `provider_synthesis_enabled` is an explicit opt-in. A synthesis case is
  rejected during validation unless that flag is true; extractive and
  retrieval-only cases remain independently measurable.

The validator also requires `operator_controlled`, `raw_data_external`,
`credentials_external`, and `private_paths_external` to be true. It rejects
filesystem paths in governance identifiers, duplicate case IDs, out-of-scope
cases, incomplete coverage, unsafe lifecycle values, and resource bounds over
the evaluator safety caps. This is a contract check only: it never opens a
source connector, writes to an index, mutates memory, or verifies corpus
content.

An approved live manifest must carry a non-secret `corpus` block with an operator-chosen `id`,
`revision`, `sha256:` digest, storage class (`local` or `encrypted-local`), approval window, and
reviewer identifier. The bounded live evaluator hashes the manifest file and emits only the manifest
digest plus the corpus identifiers/revision/digest in its report. Approval timestamps, reviewer
labels, raw queries, source content, private paths, and credentials never leave the local run.

The evaluator records the non-secret governance contract version and a digest
of the normalized workspace/source scope. A changed corpus digest, manifest
digest, or governance scope digest is a provenance change, not evidence of a
product regression, and must be reviewed independently.

A corpus or manifest change must not be misreported as a code regression.

Issue #2046 consumes this manifest/provenance contract but does not close corpus governance in
#2045. The final approved-corpus gate remains blocked until an authorized operator supplies a
governed, read-only manifest and index under the controls defined by #2045.

## Reports

Machine-readable reports should include bounded:

- case identifiers;
- metrics;
- revisions and contract versions;
- provider and environment identifiers;
- pass/fail status;
- degradation and fallback reasons;
- timing and resource measurements.

Reports must exclude raw source content, memory content, private queries, tokens, credential paths, and unnecessary absolute paths.

## Activation rules

- Core retrieval remains usable without a query model.
- Extractive mode remains independently available.
- Provider-backed synthesis requires reproducible approved-corpus evidence and explicit opt-in.
- Recurring synchronization requires source-readiness and bounded-trial evidence.
- Automatic memory formation requires candidate, classification, policy, provenance, ACL, and review evidence.
- A platform support claim requires packaged acceptance and OS trust evidence.
- Hosted, synchronized, or shared modes require their own security, tenancy, reliability, and deletion evaluation.

## Planning boundary

This document defines evaluation methods. [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues) own the cases currently pending, their owners, sequence, blockers, results, and activation decisions.
