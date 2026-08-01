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

## Desktop release gates

The desktop pipeline follows a staged audit policy:

- **Staging stays fast.** `desktop.yml` triggers only on pull requests
  targeting `main` and on manual dispatch, so staging integration never waits
  on the desktop build.
- **Main code PRs require the desktop aggregate.** Ordinary main-targeted pull
  requests run the `audit` (provenance), `quality` (desktop tests + clippy),
  and `release` (Linux release compilation) jobs; the stable `Tauri 2 / Linux`
  aggregate check must pass before merge.
- **Web quality is owned by Code Foundry Validation / CI.** The `quality` job
  does not rerun `bun run typecheck` or `bun run build`: Code Foundry
  Validation / CI already runs both on the same main-targeting PR SHA, so the
  desktop pipeline only repeats desktop-specific fast checks.
- **Version-only release PRs are intentionally lightweight.** Release Please
  pull requests (`release-please--branches--main` head refs) skip the three
  long desktop jobs entirely at job level. The `Tauri 2 / Linux` aggregate
  still runs and treats skipped dependencies as acceptable, so the required
  check stays green without burning runner minutes on version bumps.
- **Manual workflow dispatch is the final audit path.** Dispatching
  `desktop.yml` on any branch reruns all three jobs unconditionally,
  independent of pull request state.
- **Audit tooling is warm-cached.** The `audit` job caches the exact
  `cargo-audit` 0.22.2 binary in `~/.cargo/bin` under a stable per-OS/arch
  key, so repeated final audits restore the pinned binary and skip
  `cargo install`; on a cache miss the install stays locked to 0.22.2, and
  the audit itself still fails on any vulnerability.

The aggregate fails on any dependency failure or cancellation rather than
silently skipping, so a real regression can never hide behind the release-please
fast path.
