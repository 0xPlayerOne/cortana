//! Config assertions for the Code Foundry v0.34.0 adoption.
//!
//! These tests pin the repository-level Code Foundry configuration so CI can
//! detect drift between `.github/code-foundry.yml`, the generated workflows,
//! and the Cargo layout they are based on. They deliberately assert the safe
//! single-pass fallback for Rust CodeQL: the repository has no Cargo
//! workspace (the root package is the only workspace member and
//! `apps/desktop/src-tauri` is a standalone package with its own lockfile),
//! so no shardable package list exists to shard on.

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

/// Assert the single-package Cargo layout that justifies the safe fallback.
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

/// No shardable workspace packages exist, so CodeQL stays on the documented
/// single-pass fallback: one "all" shard, two CodeQL threads, one concurrent job.
#[test]
fn rust_codeql_uses_safe_single_pass_fallback() {
    assert_eq!(config_value("codeql_rust_shards"), "'[\"all\"]'");
    assert_eq!(config_value("codeql_rust_threads"), "2");
    assert_eq!(config_value("codeql_rust_max_parallel"), "1");

    let caller = read(".github/workflows/validation.yml");
    assert!(
        caller.contains("rust-shards: '[\"all\"]'"),
        "validation caller must forward the single-shard fallback:\n{caller}"
    );
    assert!(caller.contains("rust-threads: '2'"), "{caller}");
    assert!(caller.contains("rust-max-parallel: 1"), "{caller}");
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

/// The desktop Tauri workflow gates the expensive Linux release compilation:
/// it runs for promotion PRs to main and manual audits, but
/// never for staging PRs, ordinary feature PRs, or Release Please version PRs.
/// The fast desktop checks above it stay unconditional.
#[test]
fn desktop_linux_release_compile_is_gated() {
    let desktop = read(".github/workflows/desktop.yml");
    let step_start = desktop
        .find("- name: Compile release desktop")
        .expect("desktop workflow must keep the compile step");
    let step = &desktop[step_start..];

    for required in [
        "github.event_name == 'workflow_dispatch'",
        "(github.event_name == 'pull_request' &&",
        "github.event.pull_request.base.ref == 'main' &&",
        "!startsWith(github.event.pull_request.head.ref, 'release-please--branches--main')",
    ] {
        assert!(
            step.contains(required),
            "compile gate must include `{required}`"
        );
    }
    assert!(
        !step.contains("github.event_name == 'push' && github.ref == 'refs/heads/main'"),
        "release compilation must not rerun on main push after the promotion PR"
    );
    // Explicit manual final audit path must exist.
    assert!(
        desktop.contains("workflow_dispatch:"),
        "desktop workflow must expose workflow_dispatch for manual final audits"
    );
    // Fast checks must precede the gate unchanged.
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
