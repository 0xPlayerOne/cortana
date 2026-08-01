//! Config assertions for the Code Foundry v0.34.0 adoption.
//!
//! These tests pin the repository-level Code Foundry configuration so CI can
//! detect drift between `.github/code-foundry.yml`, the generated workflows,
//! and the Cargo layout they are based on. Rust CodeQL is sharded across the
//! standalone Cargo manifests (root package, desktop Tauri app, vendored
//! glib) with a bounded parallelism cap.

use std::fs;
use std::path::{Path, PathBuf};

/// Runtime tag every generated workflow and config line must pin.
const RUNTIME_REF: &str = "v0.34.0";

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
            "legacy generated caller {legacy}.yml must be removed by the v0.34.0 sync"
        );
    }
    let caller = read(".github/workflows/validation.yml");
    assert!(
        caller.contains("uses: 0xPlayerOne/code-foundry/.github/workflows/validation.yml@")
            && caller.contains("@v0.34.0"),
        "validation caller must reference the v0.34.0 orchestrator:\n{caller}"
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
    for other in ["audit", "quality", "release", "aggregate"] {
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

/// The desktop workflow splits the former single sequential job into three
/// independent parallel jobs (audit/provenance, web+desktop quality, release
/// compilation) plus a fast aggregate job that keeps the stable
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

    // The three parallel jobs: independent names, runners, timeouts, and a
    // job-level release-please guard so version-only PRs never start them.
    let parallel_jobs = [
        ("audit", "Audit / Provenance"),
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
        aggregate.contains("needs: [audit, quality, release]"),
        "aggregate job must depend on all three parallel jobs:\n{aggregate}"
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
        "needs.audit.result == 'failure'",
        "needs.audit.result == 'cancelled'",
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

    // Final-audit steps keep the release-please exclusion and main-only gate.
    for step in [
        "Install cargo-audit",
        "Test patched GTK iterator in release mode",
        "Audit desktop Rust dependencies",
        "Compile release desktop",
    ] {
        let start = desktop
            .find(&format!("- name: {step}"))
            .unwrap_or_else(|| panic!("desktop workflow must keep the `{step}` step"));
        let step_block = &desktop[start
            ..desktop[start + 1..]
                .find("\n      - name: ")
                .map_or(desktop.len(), |next| start + 1 + next)];

        for required in &final_audit_gate {
            assert!(
                step_block.contains(required),
                "`{step}` must use the final-audit gate with `{required}`"
            );
        }
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
        "- name: Check web",
        "- name: Test desktop",
        "- name: Lint desktop",
        "- name: Verify patched GTK dependency provenance",
    ] {
        assert!(
            desktop.contains(fast_check),
            "fast desktop check `{fast_check}` must be retained"
        );
    }
}
