//! Config assertions for the Code Foundry runtime pin adoption.
//!
//! These tests pin the repository-level Code Foundry configuration so CI can
//! detect drift between `.github/code-foundry.yml`, the generated workflows,
//! and the Cargo layout they are based on. Rust CodeQL is sharded across the
//! standalone Cargo manifests (root package, desktop Tauri app, vendored
//! glib) with a bounded parallelism cap.

use std::fs;
use std::path::{Path, PathBuf};

/// Runtime tag every generated workflow and config line must pin. Read from
/// the adopted config so a runtime bump does not require editing this test.
fn runtime_ref() -> String {
    config_value("runtime_ref")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    let full = repo_root().join(path);
    fs::read_to_string(&full)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", full.display()))
}

/// Assert the standalone-manifest Cargo layout the shard list is based on.
#[test]
fn cargo_layout_has_no_workspace_to_shard() {
    let root_cargo = read("Cargo.toml");
    assert!(
        !root_cargo.lines().any(|line| line.trim() == "[workspace]"),
        "root Cargo.toml unexpectedly declares a [workspace]; re-evaluate Rust CodeQL sharding"
    );
    let desktop_cargo = read("apps/desktop/src-tauri/Cargo.toml");
    assert!(
        !desktop_cargo
            .lines()
            .any(|line| line.trim() == "[workspace]"),
        "desktop Cargo.toml unexpectedly declares a [workspace]; re-evaluate Rust CodeQL sharding"
    );
    // Single workspace member at the repository root, mirroring `cargo metadata --no-deps`.
    assert!(
        root_cargo.contains("name = \"cortana\""),
        "root Cargo.toml package name changed; update this assertion"
    );
}

fn config_value(key: &str) -> String {
    read(".github/code-foundry.yml")
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
        .unwrap_or_else(|| panic!("missing `{key}` in .github/code-foundry.yml"))
}

/// The generated config pins the adopted runtime everywhere.
#[test]
fn config_pins_runtime_ref() {
    assert_eq!(config_value("runtime_ref"), runtime_ref());
    assert_eq!(
        config_value("runtime_repository"),
        "0xPlayerOne/code-foundry"
    );
}

#[test]
fn release_merge_policy_matches_runtime_contract() {
    assert_eq!(config_value("merge_strategy"), "rebase");
    assert_eq!(config_value("release_merge_strategy"), "rebase");
}

/// Rust CodeQL shards across the three standalone Cargo manifests: the root
/// package, the desktop Tauri app, and the vendored glib. Two CodeQL threads
/// per shard and three shards in parallel cut wall-clock time while capping
/// total runner cost.
#[test]
fn rust_codeql_shards_standalone_manifests() {
    assert_eq!(
        config_value("codeql_rust_shards"),
        "'[\"src\",\"apps/desktop/src-tauri\",\"third_party/glib-0.18.5\"]'"
    );
    assert_eq!(config_value("codeql_rust_threads"), "2");
    assert_eq!(config_value("codeql_rust_max_parallel"), "3");

    let caller = read(".github/workflows/validation.yml");
    assert!(
        caller.contains(
            "rust-shards: '[\"src\",\"apps/desktop/src-tauri\",\"third_party/glib-0.18.5\"]'"
        ),
        "validation caller must forward the shard list:\n{caller}"
    );
    assert!(caller.contains("rust-threads: '2'"), "{caller}");
    assert!(caller.contains("rust-max-parallel: 3"), "{caller}");
}

/// The tiered validation caller is the single canonical validation entry
/// point: no legacy ci/test/security/codeql callers remain, so there can be
/// no duplicate validation suites.
#[test]
fn single_canonical_validation_caller() {
    for legacy in ["ci", "test", "security", "codeql"] {
        assert!(
            !Path::new(&repo_root())
                .join(format!(".github/workflows/{legacy}.yml"))
                .exists(),
            "legacy generated caller {legacy}.yml must be removed by the configured sync"
        );
    }
    let caller = read(".github/workflows/validation.yml");
    assert!(
        caller.contains(&format!(
            "uses: 0xPlayerOne/code-foundry/.github/workflows/validation.yml@{}",
            runtime_ref()
        )),
        "validation caller must reference the configured orchestrator:\n{caller}"
    );
}

/// Every runtime reference in the generated caller pins the adopted tag and
/// the caller never triggers on push, so no push+pull_request duplicate runs.
#[test]
fn validation_caller_pins_runtime_and_has_no_push_trigger() {
    let caller = read(".github/workflows/validation.yml");
    // Mode-job checkout ref and orchestrator input must both be the pinned tag.
    assert_eq!(
        caller
            .lines()
            .filter(|line| line.trim().starts_with("ref:") && line.contains(&runtime_ref()))
            .count(),
        1,
        "mode checkout must pin {}",
        runtime_ref()
    );
    assert!(
        caller.contains(&format!("runtime-ref: {}", runtime_ref())),
        "orchestrator input must pin {}",
        runtime_ref()
    );
    assert!(
        !caller.lines().any(|line| line.trim() == "push:"),
        "validation caller must not trigger on push; push+pull_request duplicates are forbidden:\n{caller}"
    );
    for event in ["pull_request:", "schedule:", "workflow_dispatch:"] {
        assert!(
            caller.lines().any(|line| line.trim() == event),
            "validation caller must keep the {event} trigger"
        );
    }
}

/// Extract a top-level job block, from its `  <job_id>:` line up to the next
/// job id (or end of file). Job-level keys share the two-space indent, so the
/// scan stops only at known job ids.
fn job_block<'a>(workflow: &'a str, job_id: &str) -> &'a str {
    let start = workflow
        .find(&format!("\n  {job_id}:"))
        .unwrap_or_else(|| panic!("desktop workflow must keep the `{job_id}` job"));
    let tail = &workflow[start + 1..];
    let mut end = workflow.len();
    for other in [
        "gtk_provenance",
        "gtk_iterator",
        "security_audit",
        "desktop_test",
        "desktop_clippy",
        "release",
        "aggregate",
    ] {
        if other != job_id {
            if let Some(pos) = tail.find(&format!("\n  {other}:")) {
                end = end.min(start + 1 + pos);
            }
        }
    }
    &workflow[start..end]
}

/// Extract the job header (up to its first step), where job-level keys live.
fn job_header(block: &str) -> &str {
    &block[..block.find("    steps:").unwrap_or(block.len())]
}

/// The desktop workflow keeps independent parallel jobs for GTK provenance,
/// GTK iterator (release mode), Rust dependency auditing, desktop tests,
/// desktop clippy, and Linux release compilation plus a fast aggregate job that
/// keeps the stable "Tauri 2 / Linux" required-check name. The workflow stays
/// scoped to main-targeted promotion PRs and manual dispatch, skipping
/// release-please version PRs at job level; final-audit steps keep the same
/// gate, and the aggregate always runs after needs, treating skipped jobs as
/// acceptable and failing only on failure or cancellation.
#[test]
fn desktop_linux_release_compile_is_gated() {
    let desktop = read(".github/workflows/desktop.yml");

    // Workflow topology assertions.
    assert!(desktop.contains("pull_request:"));
    assert!(desktop.contains("branches: [main, staging]"));
    assert!(!desktop.contains("\n  push:"));
    assert!(desktop.contains("workflow_dispatch:"));

    let final_audit_gate = [
        "github.event_name == 'workflow_dispatch'",
        "(github.event_name == 'pull_request' &&",
        "github.event.pull_request.base.ref == 'main' &&",
        "!startsWith(github.event.pull_request.head.ref, 'release-please--branches--main')",
    ];

    // The six parallel jobs: independent names, runners, timeouts, and
    // a job-level release-please guard so version-only PRs never start them.
    let parallel_jobs = [
        ("gtk_provenance", "GTK Provenance"),
        ("gtk_iterator", "GTK iterator (release)"),
        ("security_audit", "Security Audit (cargo-audit)"),
        ("desktop_test", "Desktop Tests"),
        ("desktop_clippy", "Desktop Clippy"),
        ("release", "Release Compilation"),
    ];
    for (job_id, job_name) in parallel_jobs {
        let block = job_block(&desktop, job_id);
        assert!(
            block.contains(&format!("name: {job_name}")),
            "`{job_id}` job must be named `{job_name}`:\n{block}"
        );
        assert!(
            block.contains("runs-on: ubuntu-24.04"),
            "`{job_id}` job must run on ubuntu-24.04:\n{block}"
        );
        assert!(
            block.contains("timeout-minutes:"),
            "`{job_id}` job must define a timeout:\n{block}"
        );
        let header = job_header(block);
        for required in &final_audit_gate {
            assert!(
                header.contains(required),
                "`{job_id}` job must apply the release-please guard at job level with `{required}`"
            );
        }
    }

    // The fast aggregate keeps the stable required-check name and fans out to
    // the detector plus every parallel job. It always runs after needs
    // (`!cancelled()`), fails only on dependency failure or cancellation, and
    // treats skipped dependencies (release-please version PRs) as acceptable.
    let aggregate = job_block(&desktop, "aggregate");
    assert!(
        aggregate.contains("name: Tauri 2 / Linux"),
        "aggregate job must keep the stable `Tauri 2 / Linux` required-check name:\n{aggregate}"
    );
    assert!(
        aggregate.contains("needs:")
            && aggregate.contains(
                "[changes, gtk_provenance, gtk_iterator, security_audit, desktop_test, desktop_clippy, release]"
            ),
        "aggregate job must depend on the detector and all six parallel jobs:\n{aggregate}"
    );
    assert!(
        aggregate.contains("if: ${{ !cancelled() }}"),
        "aggregate job must always run after needs, even when dependencies are skipped:\n{aggregate}"
    );
    assert!(
        aggregate.contains("timeout-minutes:"),
        "aggregate job must define a timeout:\n{aggregate}"
    );
    for token in [
        "needs.changes.result == 'failure'",
        "needs.changes.result == 'cancelled'",
        "needs.gtk_provenance.result == 'failure'",
        "needs.gtk_provenance.result == 'cancelled'",
        "needs.gtk_iterator.result == 'failure'",
        "needs.gtk_iterator.result == 'cancelled'",
        "needs.security_audit.result == 'failure'",
        "needs.security_audit.result == 'cancelled'",
        "needs.desktop_test.result == 'failure'",
        "needs.desktop_test.result == 'cancelled'",
        "needs.desktop_clippy.result == 'failure'",
        "needs.desktop_clippy.result == 'cancelled'",
        "needs.release.result == 'failure'",
        "needs.release.result == 'cancelled'",
    ] {
        assert!(
            aggregate.contains(token),
            "aggregate fail step must check `{token}`:\n{aggregate}"
        );
    }
    assert!(
        !aggregate.contains("!= 'success'"),
        "aggregate must treat skipped dependencies as acceptable, not fail on them:\n{aggregate}"
    );

    // Final-audit jobs keep the release-please exclusion and main-only gate;
    // individual steps no longer repeat the same condition after the split.
    for job_id in [
        "gtk_provenance",
        "gtk_iterator",
        "security_audit",
        "desktop_test",
        "desktop_clippy",
        "release",
    ] {
        let header = job_header(job_block(&desktop, job_id));
        for required in &final_audit_gate {
            assert!(
                header.contains(required),
                "`{job_id}` must apply the final-audit gate at job level with `{required}`"
            );
        }
    }
    for step in [
        "Verify patched GTK dependency provenance",
        "Test patched GTK iterator in release mode",
        "Install cargo-audit",
        "Audit desktop Rust dependencies",
        "Compile release desktop",
    ] {
        assert!(
            desktop.contains(&format!("- name: {step}")),
            "desktop workflow must keep the `{step}` step"
        );
    }

    // Verify rust cache action and lockfile-driven key and target paths.
    assert!(
        desktop.contains("- name: Cache Rust build artifacts"),
        "desktop workflow should cache rust artifacts before rust checks"
    );
    assert!(
        desktop.contains("hashFiles('apps/desktop/src-tauri/Cargo.lock')")
            && desktop.contains("hashFiles('third_party/glib-0.18.5/Cargo.toml')"),
        "rust cache should be lockfile-derived for desktop and glib inputs"
    );
    assert!(desktop.contains("apps/desktop/src-tauri/target"));
    assert!(desktop.contains("third_party/glib-0.18.5/target"));

    // Fast checks must remain present.
    for fast_check in [
        "- name: Test desktop",
        "- name: Lint desktop",
        "- name: Verify patched GTK dependency provenance",
        "- name: Test patched GTK iterator in release mode",
    ] {
        assert!(
            desktop.contains(fast_check),
            "fast desktop check `{fast_check}` must be retained"
        );
    }

    // The root type-check/build tasks are owned by Code Foundry Validation / CI,
    // which runs the repository's Python, Rust, and web checks on the same
    // main-targeting PR SHA. The desktop tests and clippy jobs must each keep
    // only their own desktop-specific check after setup.
    let desktop_test = job_block(&desktop, "desktop_test");
    assert!(
        !desktop_test.contains("- name: Check web"),
        "desktop_test job must not duplicate the Code Foundry web typecheck/build step:\n{desktop_test}"
    );
    let test_cache_start = desktop_test
        .find("- name: Cache Rust build artifacts")
        .expect("desktop_test job must keep the rust cache step");
    let test_tail_start = desktop_test[test_cache_start + 1..]
        .find("\n      - name: ")
        .map_or(desktop_test.len(), |next| test_cache_start + 1 + next);
    let test_tail = &desktop_test[test_tail_start..];
    assert_eq!(
        test_tail.matches("- name: ").count(),
        1,
        "desktop_test job must run only the desktop test check after setup:\n{desktop_test}"
    );
    assert!(
        test_tail.contains("- name: Test desktop")
            && test_tail.contains("run: bun run desktop:test"),
        "desktop_test job must keep the desktop test step:\n{desktop_test}"
    );

    let desktop_clippy = job_block(&desktop, "desktop_clippy");
    let clippy_cache_start = desktop_clippy
        .find("- name: Cache Rust build artifacts")
        .expect("desktop_clippy job must keep the rust cache step");
    let clippy_tail_start = desktop_clippy[clippy_cache_start + 1..]
        .find("\n      - name: ")
        .map_or(desktop_clippy.len(), |next| clippy_cache_start + 1 + next);
    let clippy_tail = &desktop_clippy[clippy_tail_start..];
    assert_eq!(
        clippy_tail.matches("- name: ").count(),
        1,
        "desktop_clippy job must run only the desktop clippy check after setup:\n{desktop_clippy}"
    );
    assert!(
        clippy_tail.contains("- name: Lint desktop")
            && clippy_tail.contains("run: bun run --cwd apps/desktop clippy"),
        "desktop_clippy job must keep the desktop clippy step:\n{desktop_clippy}"
    );
}

/// The dependency-audit job warm-caches the exact cargo-audit 0.22.2 binary with the
/// actions cache instead of recompiling it on every final audit. The cache
/// path holds only the pinned binary, the key is stable and versioned by
/// runner OS/arch plus the pinned version (never a lockfile hash), and the
/// install step keeps the final-audit gate while skipping on an exact cache
/// hit.
#[test]
fn desktop_audit_caches_cargo_audit_binary() {
    let desktop = read(".github/workflows/desktop.yml");

    // The cache step must restore the binary before the install check runs.
    let cache_start = desktop
        .find("- name: Cache cargo-audit binary")
        .unwrap_or_else(|| panic!("desktop workflow must warm-cache the cargo-audit binary"));
    let install_start = desktop
        .find("- name: Install cargo-audit")
        .unwrap_or_else(|| panic!("desktop workflow must keep the `Install cargo-audit` step"));
    assert!(
        cache_start < install_start,
        "cargo-audit cache step must run before the install step"
    );

    // The cache exists and holds only the exact pinned binary.
    let cache_block = &desktop[cache_start
        ..desktop[cache_start + 1..]
            .find("\n      - name: ")
            .map_or(desktop.len(), |next| cache_start + 1 + next)];
    let cache_major = cache_block.lines().find_map(|line| {
        let line = line.trim();
        let version = line
            .strip_prefix("uses: actions/cache@v")
            .and_then(|value| value.split_whitespace().next())
            .or_else(|| {
                line.split_once("# v")
                    .map(|(_, value)| value.split_whitespace().next().unwrap_or_default())
            })?;
        version.parse::<u32>().ok()
    });
    assert!(
        cache_major.is_some_and(|major| major >= 4),
        "cargo-audit cache must use a supported actions/cache major (v4+):\n{cache_block}"
    );
    assert!(
        cache_block.contains("id: cache-cargo-audit"),
        "cargo-audit cache step must expose an id for the cache-hit guard:\n{cache_block}"
    );
    assert!(
        cache_block.contains("path: ~/.cargo/bin/cargo-audit"),
        "cargo-audit cache must hold exactly the binary in ~/.cargo/bin:\n{cache_block}"
    );

    // The key is stable and versioned by runner OS/arch plus the pinned
    // version; a lockfile-derived key would miss on every run and defeat
    // the warm cache.
    assert!(
        cache_block.contains("${{ runner.os }}-${{ runner.arch }}-cargo-audit-0.22.2"),
        "cargo-audit cache key must pin runner OS/arch and version 0.22.2:\n{cache_block}"
    );
    assert!(
        !cache_block.contains("hashFiles"),
        "cargo-audit cache key must be stable, not lockfile-derived:\n{cache_block}"
    );

    // The cache-hit guard skips installation while the job-level final-audit
    // gate and the pinned install command stay intact.
    let install_block = &desktop[install_start
        ..desktop[install_start + 1..]
            .find("\n      - name: ")
            .map_or(desktop.len(), |next| install_start + 1 + next)];
    assert!(
        install_block.contains("steps.cache-cargo-audit.outputs.cache-hit != 'true'"),
        "install must be skipped on an exact cargo-audit cache hit:\n{install_block}"
    );
    for required in [
        "github.event_name == 'workflow_dispatch'",
        "(github.event_name == 'pull_request' &&",
        "github.event.pull_request.base.ref == 'main' &&",
        "!startsWith(github.event.pull_request.head.ref, 'release-please--branches--main')",
    ] {
        assert!(
            job_header(job_block(&desktop, "security_audit")).contains(required),
            "security_audit job must keep the final-audit gate with `{required}`"
        );
    }
    assert!(
        !install_block.contains("github.event_name == 'workflow_dispatch'"),
        "cargo-audit install should rely on its job-level final-audit gate:\n{install_block}"
    );
    assert!(
        install_block.contains("run: cargo install cargo-audit --version 0.22.2 --locked"),
        "install must stay locked to cargo-audit 0.22.2:\n{install_block}"
    );
}

#[test]
fn staging_promotion_caller_matches_staging_release_topology() {
    let release_pr = read(".github/workflows/release-pr.yml");
    assert!(
        release_pr.contains("branches: [staging]"),
        "staging promotion caller must trigger only on staging pushes:\n{release_pr}"
    );
    assert!(
        release_pr.contains(&format!(
            "uses: 0xPlayerOne/code-foundry/.github/workflows/release-pr.yml@{}",
            runtime_ref()
        )),
        "staging promotion caller must pin the configured reusable workflow:\n{release_pr}"
    );
    for secret in ["CODE_FOUNDRY_TOKEN", "RELEASE_PLEASE_TOKEN"] {
        assert!(
            release_pr.contains(&format!("{secret}: ${{{{ secrets.{secret} }}}}")),
            "staging promotion caller must pass `{secret}`:\n{release_pr}"
        );
    }
    assert!(
        release_pr.contains("pull-requests: write"),
        "staging promotion caller must be allowed to create/update its PR:\n{release_pr}"
    );
}

/// The direct-topology main release caller targets main only and delegates
/// the entire release (Release Please manifest handling, guarded auto-merge,
/// release creation) to the pinned reusable workflow. No staging promotion
/// preflight exists: there is no staging branch to gate against.
#[test]
fn release_caller_targets_main_without_staging_preflight() {
    let release = read(".github/workflows/release.yml");

    // The caller triggers on pushes to main and nothing else.
    assert!(
        release.contains("on:") && release.contains("branches: [main]"),
        "direct release caller must trigger on push to main:\n{release}"
    );
    assert!(
        !release.contains("staging"),
        "direct release caller must not reference staging:\n{release}"
    );

    // No staging promotion preflight job exists in the direct topology.
    assert!(
        !release.contains("\n  preflight:"),
        "direct release caller must not declare a staging preflight job:\n{release}"
    );
    assert!(
        !release.contains("git fetch --no-tags origin main staging"),
        "direct release caller must not fetch staging:\n{release}"
    );

    // The release job pins the reusable workflow and its inputs.
    let release_job = job_block(&release, "release");
    assert!(
        release_job.contains(&format!(
            "uses: 0xPlayerOne/code-foundry/.github/workflows/release.yml@{}",
            runtime_ref()
        )),
        "release job must keep the pinned reusable workflow:\n{release_job}"
    );
    for input in [
        "runner: ubuntu-slim",
        "runtime-repository: 0xPlayerOne/code-foundry",
        &format!("runtime-ref: {}", runtime_ref()),
    ] {
        assert!(
            release_job.contains(input),
            "release job must keep input `{input}`:\n{release_job}"
        );
    }
    for secret in [
        "CODE_FOUNDRY_TOKEN",
        "RELEASE_PLEASE_TOKEN",
        "STAGING_DEPLOY_KEY",
        "NPM_TOKEN",
    ] {
        assert!(
            release_job.contains(&format!("{secret}: ${{{{ secrets.{secret} }}}}")),
            "release job must keep passing `{secret}`:\n{release_job}"
        );
    }
    for permission in [
        "actions: write",
        "contents: write",
        "issues: write",
        "pull-requests: write",
        "id-token: write",
    ] {
        assert!(
            release.contains(permission),
            "release caller must keep the `{permission}` permission"
        );
    }
}

/// The Release Please contract is enforced by the pinned reusable release
/// workflow, not by a caller-level preflight. The direct caller keeps the
/// stable concurrency group and never adds staging promotion gating.
#[test]
fn release_preflight_metadata_contract_stays_in_runtime() {
    let release = read(".github/workflows/release.yml");
    assert!(
        !release.contains("RELEASE_FILES:"),
        "direct release caller must not duplicate the Release Please metadata contract:\n{release}"
    );
    assert!(
        release.contains("uses: 0xPlayerOne/code-foundry/.github/workflows/release.yml@"),
        "release caller must delegate the release contract to the pinned runtime:\n{release}"
    );
}
