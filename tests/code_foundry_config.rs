//! Config assertions for the Code Foundry v0.34.2 adoption.
//!
//! These tests pin the repository-level Code Foundry configuration so CI can
//! detect drift between `.github/code-foundry.yml`, the generated workflows,
//! and the Cargo layout they are based on. Rust CodeQL is sharded across the
//! standalone Cargo manifests (root package, desktop Tauri app, vendored
//! glib) with a bounded parallelism cap.

use std::fs;
use std::path::{Path, PathBuf};

/// Runtime tag every generated workflow and config line must pin.
const RUNTIME_REF: &str = "v0.34.2";

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
    assert_eq!(config_value("runtime_ref"), RUNTIME_REF);
    assert_eq!(
        config_value("runtime_repository"),
        "0xPlayerOne/code-foundry"
    );
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
            "legacy generated caller {legacy}.yml must be removed by the v0.34.2 sync"
        );
    }
    let caller = read(".github/workflows/validation.yml");
    assert!(
        caller.contains("uses: 0xPlayerOne/code-foundry/.github/workflows/validation.yml@")
            && caller.contains("@v0.34.2"),
        "validation caller must reference the v0.34.2 orchestrator:\n{caller}"
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
            .filter(|line| line.trim().starts_with("ref:") && line.contains(RUNTIME_REF))
            .count(),
        1,
        "mode checkout must pin {RUNTIME_REF}"
    );
    assert!(
        caller.contains(&format!("runtime-ref: {RUNTIME_REF}")),
        "orchestrator input must pin {RUNTIME_REF}"
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
        "security_audit",
        "quality",
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

/// The desktop workflow splits the former single sequential audit path into
/// four independent parallel jobs (provenance, Rust dependency audit,
/// web+desktop quality, release compilation) plus a fast aggregate job that keeps the stable
/// "Tauri 2 / Linux" required-check name. The workflow stays scoped to
/// main-targeted promotion PRs and manual dispatch, skipping release-please
/// version PRs at job level; final-audit steps keep the same gate, and the
/// aggregate always runs after needs, treating skipped jobs as acceptable
/// and failing only on failure or cancellation.
#[test]
fn desktop_linux_release_compile_is_gated() {
    let desktop = read(".github/workflows/desktop.yml");

    // Workflow topology assertions.
    assert!(desktop.contains("pull_request:"));
    assert!(desktop.contains("branches: [main]"));
    assert!(!desktop.contains("\n  push:"));
    assert!(desktop.contains("workflow_dispatch:"));

    let final_audit_gate = [
        "github.event_name == 'workflow_dispatch'",
        "(github.event_name == 'pull_request' &&",
        "github.event.pull_request.base.ref == 'main' &&",
        "!startsWith(github.event.pull_request.head.ref, 'release-please--branches--main')",
    ];

    // The four parallel jobs: independent names, runners, timeouts, and a
    // job-level release-please guard so version-only PRs never start them.
    let parallel_jobs = [
        ("gtk_provenance", "GTK Provenance + Release Test"),
        ("security_audit", "Security Audit (cargo-audit)"),
        ("quality", "Web + Desktop Quality"),
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
    // every parallel job. It always runs after needs (`!cancelled()`), fails
    // only on dependency failure or cancellation, and treats skipped
    // dependencies (release-please version PRs) as acceptable.
    let aggregate = job_block(&desktop, "aggregate");
    assert!(
        aggregate.contains("name: Tauri 2 / Linux"),
        "aggregate job must keep the stable `Tauri 2 / Linux` required-check name:\n{aggregate}"
    );
    assert!(
        aggregate.contains("needs: [gtk_provenance, security_audit, quality, release]"),
        "aggregate job must depend on all four parallel jobs:\n{aggregate}"
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
        "needs.gtk_provenance.result == 'failure'",
        "needs.gtk_provenance.result == 'cancelled'",
        "needs.security_audit.result == 'failure'",
        "needs.security_audit.result == 'cancelled'",
        "needs.quality.result == 'failure'",
        "needs.quality.result == 'cancelled'",
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
    for job_id in ["gtk_provenance", "security_audit", "quality", "release"] {
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
    ] {
        assert!(
            desktop.contains(fast_check),
            "fast desktop check `{fast_check}` must be retained"
        );
    }

    // Web typecheck/build is owned by Code Foundry Validation / CI, which
    // already runs `bun run typecheck` and `bun run build` on the same
    // main-targeting PR SHA. The desktop quality job must not duplicate it:
    // after the shared setup steps it runs exactly the two desktop-specific
    // fast checks (tests + clippy) and no other steps.
    let quality = job_block(&desktop, "quality");
    assert!(
        !quality.contains("- name: Check web"),
        "quality job must not duplicate the Code Foundry web typecheck/build step:\n{quality}"
    );
    let cache_start = quality
        .find("- name: Cache Rust build artifacts")
        .expect("quality job must keep the rust cache step");
    let tail_start = quality[cache_start + 1..]
        .find("\n      - name: ")
        .map_or(quality.len(), |next| cache_start + 1 + next);
    let tail = &quality[tail_start..];
    assert_eq!(
        tail.matches("- name: ").count(),
        2,
        "quality job must run exactly the two desktop fast checks after setup:\n{quality}"
    );
    assert!(
        tail.contains("- name: Test desktop") && tail.contains("run: bun run desktop:test"),
        "quality job must keep the desktop test step:\n{quality}"
    );
    assert!(
        tail.contains("- name: Lint desktop")
            && tail.contains("run: bun run --cwd apps/desktop clippy"),
        "quality job must keep the desktop clippy step:\n{quality}"
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
    assert!(
        cache_block.contains("uses: actions/cache@v4"),
        "cargo-audit cache must use actions/cache@v4:\n{cache_block}"
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
