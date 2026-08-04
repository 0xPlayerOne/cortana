# Release history

Cortana preserves published tags and corrects release metadata with a new patch
release instead of rewriting an existing tag.

The transitional `v0.1.2` release shipped the production installer and platform
archives while the Rust version update was being migrated from the legacy
single-package flow. The following patch release reconciles the Release Please
manifest, Rust crate, Python package, web application, and lockfile versions
under the automated manifest flow.

Generated version pull requests are restricted to changelog and configured
version files, then merged automatically without running the code-change test
matrix. Topic and staging-to-main promotion pull requests still run the complete
CI, test, security, and CodeQL gates.

## Promotion-before-release invariant

A release must never run while staging still contains source code that has not
been promoted to main. Releasing over unpromoted work publishes version
metadata that does not describe the promoted code, and the subsequent
`Release / Reconcile` step fails when it tries to reconcile the new release
metadata onto a staging branch that carries unpromoted commits.

The main release caller (`release.yml`) enforces this invariant with a read-only
preflight job that runs before the reusable Release job. It fetches
`origin/main` and `origin/staging` and classifies the direct tree diff:

- **Ready** when the two trees are identical, or when the only differing paths
  are approved Release Please metadata: `.release-please-manifest.json`,
  `CHANGELOG.md`, the configured version files, and lockfiles
  (`release-please-config.json` extra-files plus the runtime default release
  files).
- **Not ready** when the diff contains any other path — staging carries source
  code that has not been promoted to main.

An unready preflight never fails the workflow: it exposes `ready=false`, emits a
`::notice` annotation and a job summary, and the Release job is skipped, so
Release Please cannot create or merge a version PR over unpromoted work. The
preflight job is strictly read-only — it consumes no secrets, holds only
`contents: read`, and only fetches; it never pushes or runs a source sync.
Promote staging to main first, then rerun the workflow.

This invariant was added after Release Please v0.24.1 merged while staging
still contained unpromoted source code, failing `Release / Reconcile`.

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

## Desktop release gates

The desktop pipeline follows a staged audit policy:

- **Staging stays fast.** `desktop.yml` triggers only on pull requests
  targeting `main` and on manual dispatch, so staging integration never waits
  on the desktop build.
- **Main code PRs require the desktop aggregate.** Ordinary main-targeted pull
  requests run six independent jobs: `gtk_provenance`, `gtk_iterator`,
  `security_audit` (pinned Rust dependency audit), `desktop_test`,
  `desktop_clippy`, and `release` (Linux release compilation). The stable
  `Tauri 2 / Linux` aggregate depends on all six and must pass before merge.
  Provenance, the iterator test, dependency auditing, desktop tests, and
  clippy run concurrently so no independent check waits behind another.
- **Web quality is owned by Code Foundry Validation / CI.** The `desktop_test`
  and `desktop_clippy` jobs do not rerun `bun run typecheck` or `bun run build`:
  Code Foundry Validation / CI already runs both on the same main-targeting PR
  SHA, so the desktop pipeline only runs desktop-specific fast checks.
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
(`scripts/verify-desktop-release.sh`) repeats the same assertion on the
downloaded Linux core archive. To verify a downloaded archive locally:

```bash
./scripts/verify-release.sh \
  cortana-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz \
  cortana-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256
```
