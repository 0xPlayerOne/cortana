# Release history

Cortana preserves published tags and corrects release metadata with a new patch
release instead of rewriting an existing tag.

The transitional `v0.1.2` release shipped the production installer and platform
archives while the Rust version update was being migrated from the legacy
single-package flow. The following patch release reconciles the Release Please
manifest, Rust crate, Python package, web application, and lockfile versions
under the automated manifest flow.

## Current release: v0.32.11

Download the Desktop app or a matching core archive from the
[latest GitHub release](https://github.com/0xPlayerOne/cortana/releases/latest). The protected
`v0.32.11` is the current protected source and published release. Release-assets workflow
`31928018360` completed all 18 archive, checksum, updater-signature, manifest, and credential-free
packaged-core gates. v0.32.10 and earlier remain historical evidence.

### Supported Desktop platforms

The v0.32.11 Desktop support policy is **macOS Apple Silicon (arm64), Linux x86_64, and Windows
x86_64**. The release intentionally does not publish an Intel macOS Desktop bundle, so Intel
macOS is unsupported rather than merely unverified. Rosetta execution and the macOS core archive
do not change that policy. Adding Intel support requires a matching app bundle, strict codesign,
updater signature, installer verification, and native Desktop acceptance evidence.

For a first installation, use the Desktop-first steps in the [README](../README.md#desktop-first-launch-recommended).
The app starts query-only: source authorization, validation, initial sync, local model setup, and
recurring ingestion are separate confirmation-gated actions. macOS Developer ID/notarization and
real browser, tray, native-dialog, and updater interactions remain host acceptance gates; passing
the release verifier does not claim those GUI behaviors.

To re-check the published release without touching the live index or starting a sync:

```bash
GH_REPO=0xPlayerOne/cortana CORTANA_REQUIRE_MINISIGN=1 \
  scripts/verify-desktop-release.sh v0.32.11
```

The current-release section is the operational source of truth. Entries below preserve historical
release and incident evidence and should be labeled historical when a newer patch is published.

The v0.32.11 source is the release boundary for the bounded large-PDF Drive parser and the
post-v0.31.12 hardening, bounded live-index
evaluation harness, and readiness-budget diagnostics described below. Future source-tree changes
must still use the protected staging and promotion flow, followed by the release verifier, before
being called downloadable-release behavior.

The current non-reconciling rollout evidence is also explicit: Work Drive completed a fresh
production-budget validation at 478 documents and 4,527,721 bytes with zero writes. Personal
Drive reached its 1,799-second connector deadline while processing a large PDF and failed closed;
it remains below the complete-validation gate. Recurring sync stays uninstalled until Personal
Drive and every other enabled source have fresh `complete=true` records at their configured
budgets.

The repository also includes `scripts/shared-agent-auth-drill.sh`, a disposable offline HTTP smoke
check for scoped principals, ACL isolation, metadata-only audit responses, and token rotation. It
uses synthetic data only and is not a substitute for the packaged GUI/MCP/manual acceptance gates.
The companion `scripts/shared-agent-mcp-drill.py` exercises the real shipped MCP stdio subprocess,
including workspace ACL filtering, file-backed token rotation, and revocation. Both drills are
offline synthetic evidence and never authorize a source or touch the live index.

## v0.32.12 release intent (pending protected release flow)

This patch publishes the Drive connector hardening promoted through the protected staging-to-main
flow: bounded PDF/DOCX extraction, folder and folder-shortcut filtering, metadata-only records for
unsupported binary items, and correct placement of the `--max-documents` cap. The release intent
changes no credentials, source authorization, indexed data, recurring-sync state, or optional
memory-provider state. Release assets and packaged-core verification remain pending until the
version-only Release Please PR is merged and its release workflow completes.

## v0.32.11 release intent (published and verified)

This patch promotes the current-release documentation boundary after the v0.32.10 package was
published. Release-assets workflow `31928018360` completed all 18 assets and the strict verifier;
it changes no credentials, source authorization, indexed data, recurring-sync state, or optional
memory-provider state.

## v0.32.10 release intent (published and verified)

The post-v0.32.9 source tree contains the bounded Google Drive PDF metadata preflight and the
corresponding current Personal Drive validation evidence. This release intent keeps those
production-safety changes represented by a downloadable patch release after the protected
staging-to-main promotion and Release Please version PR. It changes no credentials,
source authorization, indexed data, recurring-sync state, or optional memory-provider state.

The protected promotion and version-only PR merged successfully. Release-assets workflow
`31926397636` published all 18 assets and passed the strict cross-platform verifier, including
archive checksums, updater signatures, the manifest, and packaged-core verification.

## v0.32.9 release intent (published and verified)

This patch publishes the bounded large-PDF Drive parser after the v0.32.8 production-budget
validation timed out while reading a large document. PDFs larger than the bounded page window are
sampled with an explicit truncation marker instead of holding the connector indefinitely. Release
Please published the v0.32.9 tag, and release-assets workflow `31920097809` completed all 18 assets,
checksums, updater signatures, the manifest, and packaged-core evaluation. The release changes no
credentials, source authorization, indexed data, recurring-sync state, or optional memory-provider state.

## v0.32.8 release intent (published and verified)

This patch records the rollback-safe Hermes migration publication and the protected promotion that
reconciled the staging and main trees. Release Please published the v0.32.8 tag, and release-assets
workflow `31903165576` completed all 18 assets, checksums, updater signatures, the manifest, and
packaged-core evaluation. The migration stages outputs before publication and restores prior
files on failure; it changes no credentials, source authorization, indexed data, recurring-sync
state, or optional memory-provider state.

The v0.32.7 release intent below remains the preceding published evidence record.

## v0.32.6 release intent (published)

This metadata-only intent publishes the readiness-budget diagnostics that landed after v0.32.5.
Readiness failures now report the validated and required document, byte, and duration limits so
operators can correct an under-budget source without inspecting private validation-state files.
The intent changes no credentials, indexed data, source authorization, recurring-sync state, or
optional memory-provider state. The protected promotion carrying this tree included
`Release-As: 0.32.6`; Release Please opened and merged the version-only PR, and the tag is now
published. The release-assets workflow `31880502344` and strict 18-asset verifier completed
successfully, including all platform archives, checksums, updater signatures, the manifest, and
packaged-core offline evaluation.

## v0.32.5 release intent

The graph workspace/source focus fix and its evidence-selection regression coverage are now
promoted through the protected staging-to-main flow. This metadata-only release intent asked
Release Please to publish the patch so the fix is represented by a downloadable version;
it does not authorize source credentials, a corpus sync, reconciliation, or optional memory
providers. Its release-assets workflow and strict 18-asset verifier succeeded before this section
was promoted as the current release record.
The protected promotion commit carries the explicit `Release-As: 0.32.5` footer so the
main release caller preserves this intent after the staging history is flattened.

The protected promotion `#1301` and Release Please automation published v0.32.5 from the
v0.32.4 documentation and graph-focus promotion. Release-assets workflow `31872008773` completed all platform
jobs and the strict 18-asset verifier, including archive checksums, six updater signatures, the
updater manifest, and the credential-free packaged-core offline evaluator. These checks do not prove packaged GUI
behavior, operating-system signing, full-corpus source readiness, or optional memory provider
behavior.

The previous v0.31.15 package and workflow `31774425020` remain historical evidence.

The v0.32.0 package also includes the bounded, read-only live-index evaluation harness
(`scripts/evaluate-live-index.py` and `eval/live-manifest.example.json`). It measures retrieval
recall/MRR, citation validity, bounded provider-backed synthesis, fallback behavior, latency, and
repeated-query cache hits against an operator-approved corpus without syncing, reconciling,
changing the index, or printing query content. A private manifest and successful run are still
required before claiming the approved-corpus evaluation gate is closed.

## v0.32.2 source hardening and rollout evidence

The v0.32.2 release contains the embedding-supervisor recovery fix: after three consecutive
five-second health-probe failures, the supervisor stops and respawns the local embedding child
and waits for health before continuing. The fix keeps steady-state liveness checks lightweight
while preserving a real vector probe for startup and restart.

The 2026-08-15 operator rollout added production-budget validation evidence without enabling
recurring sync or reconciliation. Work Drive validated 478 documents and 4,527,663 bytes at the
2,000-document/128 MiB/900-second budget. Work Gmail validated 7,395 documents and 34,494,647
bytes at its 10,000-document/64 MiB/600-second budget. A Work Drive non-reconciling trial was
intentionally cancelled twice under the installed v0.32.1 binary after its embedding service
stalled; it made no deletions and did not authorize a full index. After v0.32.2 installation, a
foreground retry progressed through bounded unchanged batches before the local embedding health
probe timed out behind queued Qwen work; it was cancelled after roughly seven minutes, and the
service recovered afterward. It made no deletions and remains pending a longer bounded trial. The Personal Drive production validation then failed closed at its
899-second connector timeout; it produced no validation record and did not authorize a sync.
Special Gmail completed production-budget validation with 214 documents and 995,335 bytes and a
bounded 100-document-cap non-reconciling trial with 0 deletions. Personal Gmail completed
production-budget validation with 430 documents and 1,563,456 bytes and the same bounded
100-document-cap non-reconciling trial with 0 deletions. These capped prefixes remain short of
complete production trials.

On 2026-08-15 a second bounded Work Drive retry emitted all 478 connector records but failed closed
when the local embedding connection closed during ingestion. The run used `--no-reconcile`, made no
deletions, and the supervisor restarted the router; query-only readiness passed after recovery.
Because controlled ingestion commits completed prefixes, this is not a clean trial result. Work
Drive remains pending a successful bounded retry.

These records advance source readiness but do not close the recurring-sync gate: every enabled
source still needs a fresh complete production-budget validation and a successful bounded trial.
Discord and code/filesystem sources remain disabled by operator choice, Slack remains optional and
unconfigured, and Hindsight/Honcho remain disabled.

## v0.31.16 release-history recovery

The graph hierarchy, truthful status fallback, and Desktop-first project documentation were
validated on `staging` and promoted to `main` through the protected exact-tree flow. This
metadata-only marker restores those already-published changes to Release Please's conventional
commit history after the promotion was flattened; it does not change runtime behavior, authorize
sources, enable recurring sync, alter credentials, or trigger a memory provider. The v0.31.16
version PR, published assets, and strict 18-asset verifier have completed successfully.

Generated version pull requests are restricted to changelog and configured
version files, then merged automatically without running the code-change test
matrix. Topic pull requests target the protected `staging` branch and run the
fast validation tier before merge. A separate protected promotion PR carries
the validated `staging` tree to `main`, where final audit and release gates run.

## Staging-release invariant

The repository uses Code Foundry's `staging-release` workflow. Topic branches
start from `staging` and merge there with squash after the fast validation tier.
Code Foundry opens or maintains a separate `staging` → `main` promotion PR;
that PR rebases into the protected release branch after the final audit and
release review pass. Release Please version PRs also target `main` and rebase
through the same protected flow.

The release caller (`release.yml`) triggers only on pushes to `main` and
delegates the Release Please contract to the pinned Code Foundry runtime. The
staging promotion caller triggers from `staging` and creates the protected
promotion PR; it does not publish a release or bypass branch protection.
After a release, Code Foundry reconciles `staging` with the new `main` tree
through its protected reconciliation path.

The `uv.lock` project entry carries a Release Please version annotation and is
covered by the package-version regression test, keeping Python lock metadata
aligned with the shared release manifest after an automated release.

The merge methods are intentionally distinct: topic PRs squash into `staging`,
while both the staging promotion PR and Release Please PR rebase into `main`.
This keeps `main` linear and makes post-release staging reconciliation
deterministic.

## 0.19.0 release-history recovery

The Hindsight desktop settings, deterministic evaluation gate, and bounded outbox
telemetry landed together in the 2026-08-02 promotion. A metadata-only marker
commit restores those already-published capabilities to Release Please's
conventional-commit history after that promotion was merged as one squash commit;
it does not change runtime behavior or trigger a corpus sync.

## 0.23.0 release-history recovery

The agent integration guide, model-backed evaluation opt-in, nested filesystem
coverage, and numbered local setup path landed together in the 2026-08-03
promotion. This metadata-only marker restores those already-published staging
capabilities to Release Please's conventional-commit history after the
promotion was merged as one squash commit. It changes documentation only; it
does not alter runtime behavior, credentials, or trigger a corpus sync.

## 0.30.7 release-history recovery

The planner headroom fix and current provider-backed evaluation evidence landed
in the 2026-08-11 staging promotion. This metadata-only marker restores those
already-published staging capabilities to Release Please's conventional-commit
history after the promotion was flattened into a single tree commit. It changes
documentation only; it does not alter runtime behavior, credentials, or trigger
a corpus sync.

## Post-v0.31.11 production hardening

The source tree after the published `v0.31.11` tag includes production hardening
that is carried by the next patch release: provider and Desktop loopback
clients reject redirects, the required unit gate runs both Bun and Python tests,
retired model identifiers are guarded in shipped runtime paths, and packaged-core
offline evaluation is enforced by the release verifiers. This marker restores
those changes to Release Please's conventional-commit history after the protected
promotion flattened their source commits. It changes release metadata only; it
does not alter runtime behavior, credentials, or trigger a corpus sync.

The next release verification must include the packaged-core offline evaluator in
addition to archive, checksum, updater-signature, and manifest checks.

The v0.31.12 patch release carries this verification contract; it does not alter
runtime behavior, credentials, or indexed data.

The release signal is intentionally documentation-only so Release Please can
publish the verification contract without changing the runtime or indexed data.

This marker is the v0.31.12 release boundary for the protected promotion flow.

## v0.31.13 onboarding and auth hardening (historical release contents)

The protected promotion after `v0.31.12`, released as `v0.31.13`, carries the Desktop-first getting-started guide,
documentation synchronization rules, and atomic HTTP bearer-policy reload with fail-closed
remote-listener protection. This metadata-only marker restores those already-validated staging
capabilities to Release Please's conventional-commit history after exact-tree promotion flattened
their topic commits. It changes release metadata only; it does not authorize sources, enable
recurring sync, alter credentials, or change indexed data.

The v0.31.13 release verification retained the archive, checksum, updater-signature, manifest,
and packaged-core gates from v0.31.12. The HTTP reload behavior is covered by rotation, invalid-policy,
remote-listener, and metadata-only audit tests; source-tree MCP bearer sessions reread the file-backed
policy on each tool call and fail closed on malformed or revoked credentials.

The post-release source also serializes direct JSONL ingestion and source validation with the
global `sync.lock`, and requires a bearer principal for `/readyz` on remote listeners while keeping
`/healthz` public liveness. These changes are not retroactively claimed for the v0.31.12 artifact.

Bearer-policy reloads now prefer the private `0600` environment file for HTTP and file-backed MCP
principals, while connector and provider API-key lookups retain process-environment precedence.
Process-environment-only bearer clients remain startup-scoped and must reconnect after rotation.
The macOS package verifier also rejects malformed `CORTANA_REQUIRE_GATEKEEPER` values instead of
silently treating them as an optional check; only `0` or `1` is accepted.

The same source-tree lane now also:

- acquires the mutation lock before opening the store for mutating CLI commands, so startup
  migrations and fingerprint writes cannot race a concurrent sync;
- bounds direct JSONL imports to 2,000 documents, 128 MiB of content, 15 minutes, and 8 MiB per
  line, and bounds custom evaluation fixtures before deserialization;
- fences Hindsight/Honcho outbox acknowledgements and failures to the specific lease that claimed
  the row, preventing an expired worker from changing a newer worker's result; and
- serializes Desktop sidecar preparation and atomically renames completed sidecars into place.
- serializes Desktop settings and schedule writes through one per-config cross-process lock.

These are shipped safety contracts, not evidence that a large personal sync or optional memory
provider is enabled. The v0.32.0 package, signatures, and packaged-core gate are verified; the
manual Desktop, source-authorization, and optional-memory gates remain separate.

## Desktop release gates

The desktop pipeline follows a staged audit policy:

- **Staging PRs keep desktop feedback fast.** `desktop.yml` exposes the stable
  `Tauri 2 / Linux` aggregate for both `staging` and `main`, but its six
  expensive jobs are final-audit jobs and stay skipped on staging PRs. Code
  Foundry's fast staging tier remains the single quick validation path.
- **Main code PRs require the desktop aggregate.** Ordinary main-targeted code
  pull requests run six independent jobs: `gtk_provenance`, `gtk_iterator`,
  `security_audit` (pinned Rust dependency audit), `desktop_test`,
  `desktop_clippy`, and `release` (Linux release compilation). The stable
  `Tauri 2 / Linux` aggregate depends on all six and must pass before merge.
  Provenance, the iterator test, dependency auditing, desktop tests, and
  clippy run concurrently so no independent check waits behind another.
- **Staging-to-main promotion defers only release compilation.** The protected
  promotion PR keeps the five desktop checks above that validate the staged
  tree, while the expensive `release` job is reserved for an ordinary main
  code PR, an explicit workflow dispatch, or the release-assets workflow. This
  prevents compiling the same desktop release twice for one staged change.
- **Repository quality is owned by Code Foundry Validation / CI.** The
  `desktop_test` and `desktop_clippy` jobs do not rerun the root `type-check` or
  `build` scripts: Code Foundry Validation / CI already runs the Python, Rust,
  and web checks on the same PR SHA, so the desktop pipeline only
  runs desktop-specific fast checks.
- **Version-only release PRs are intentionally lightweight.** Release Please
  pull requests (`release-please--branches--main` head refs) skip all six long
  desktop jobs entirely at job level. The `Tauri 2 / Linux` aggregate still
  runs and treats skipped dependencies as acceptable, so the required check
  stays green without burning runner minutes on version bumps.
- **Manual workflow dispatch is the final audit path.** Dispatching
  `desktop.yml` on any branch reruns all six jobs unconditionally, independent
  of pull request state.
- **Audit tooling is warm-cached.** The `security_audit` job caches the exact
  `cargo-audit` 0.22.2 binary in
  `~/.cargo/bin` under a stable per-OS/arch key, so repeated final audits
  restore the pinned binary and skip `cargo install`; on a cache miss the
  install stays locked to 0.22.2, and the audit itself still fails on any
  vulnerability.

The aggregate fails on any dependency failure or cancellation rather than
silently skipping, so a real regression can never hide behind the release-please
fast path.

Desktop artifacts also carry the connector package source as a Tauri resource. The
application never embeds credentials or a machine-specific virtual environment; after an
explicit Readiness approval, native Rust uses the local `uv` executable to create the per-user
connector environment and install the bounded ingestion extra.

## Binary archive verification

Before uploading a binary archive, the release workflow runs
`scripts/verify-release.sh`. It checks the SHA-256 sidecar, rejects absolute or
path-traversal entries, requires the executable, web workspace, connector wheel,
example config, and Cortana skill, and executes the packaged binary's version
command — asserting the reported version equals the release tag encoded in the
archive name (`cortana-vX.Y.Z-<target>.tar.gz`). An archive whose name does not
embed a plain semver version, or whose packaged `bin/cortana` reports a
different version, fails the gate so a stale-checkout build or mislabeled
upload can never ship as a release. The final published-asset gate
(`scripts/verify-desktop-release.sh`) repeats the version and, for releases built
after this gate was introduced, offline-evaluation assertions on the downloaded
Linux core archive when running on Linux. When the host can execute the packaged
target, the current verifiers run `cortana --offline eval` with an isolated
temporary configuration and a hard 60-second timeout, requiring JSON `passed: true`.
The macOS package verifier applies the same check to the bundled core. These checks
prove the shipped core only; they do not launch the GUI or authorize sync. To verify
a downloaded archive locally:

```bash
./scripts/verify-release.sh \
  cortana-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz \
  cortana-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256
```
