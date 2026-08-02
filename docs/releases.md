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

## 0.19.0 release-history recovery

The Hindsight desktop settings, deterministic evaluation gate, and bounded outbox
telemetry landed together in the 2026-08-02 promotion. A metadata-only marker
commit restores those already-published capabilities to Release Please's
conventional-commit history after that promotion was merged as one squash commit;
it does not change runtime behavior or trigger a corpus sync.

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

### macOS publisher trust

The release workflow keeps platform signing opt-in. Without Apple credentials,
macOS artifacts use the existing ad-hoc identity (`-`); the Tauri updater
signature still protects update payloads, but Gatekeeper will not treat the app
as a notarized Developer ID application. To enable trusted distribution, add
these repository secrets together:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID `.p12` certificate;
- `APPLE_CERTIFICATE_PASSWORD`: the `.p12` export password;
- `APPLE_SIGNING_IDENTITY`: the exact Developer ID Application identity;
- `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`: the Apple account and
  notarization credentials.

The workflow passes them only to the macOS Tauri release job. Never commit
certificate material or credentials to the repository. If the secrets are
absent, the release remains usable for local testing and updater verification,
but should not be advertised as notarized macOS distribution.

Desktop artifacts also carry the connector package source as a Tauri resource. The
application never embeds credentials or a machine-specific virtual environment; after an
explicit Readiness approval, native Rust uses the local `uv` executable to create the per-user
connector environment and install the bounded ingestion extra.

## Binary archive verification

Before uploading a binary archive, the release workflow runs
`scripts/verify-release.sh`. It checks the SHA-256 sidecar, rejects absolute or
path-traversal entries, requires the executable, web workspace, connector wheel,
example config, and Cortana skill, and executes the packaged binary's version
command. To verify a downloaded archive locally:

```bash
./scripts/verify-release.sh \
  cortana-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz \
  cortana-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256
```
