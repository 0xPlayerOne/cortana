import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_CASES = {
    "clean-install-first-run",
    "source-authorization-discovery",
    "services-tray-autostart",
    "backup-restore-dialogs",
    "updater-lifecycle",
    "recovery-failure-paths",
    "accessibility-screen-reader",
    "resource-large-list",
    "uninstall-and-data-preservation",
    "os-trust",
}
REQUIRED_CASE_CHECKS = {
    "clean-install-first-run",
    "source-authorization-discovery",
    "services-tray-autostart",
    "backup-restore-dialogs",
    "updater-lifecycle",
    "recovery-failure-paths",
    "accessibility-screen-reader",
    "resource-large-list",
    "uninstall-and-data-preservation",
    "os-trust",
}
SPEC = importlib.util.spec_from_file_location(
    "cortana_desktop_acceptance_evidence",
    ROOT / "scripts/verify-desktop-acceptance-evidence.py",
)
assert SPEC and SPEC.loader
desktop_evidence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(desktop_evidence)


def template() -> dict:
    return json.loads((ROOT / "eval/desktop-acceptance-private.example.json").read_text())


def approved_evidence() -> dict:
    evidence = template()
    evidence["approved"] = True
    digest = "sha256:" + "1" * 64
    evidence["release"]["release_digest"] = digest
    for platform in evidence["platforms"]:
        platform["package_digest"] = digest
        for case in platform["cases"]:
            case["result"] = "passed"
            case["evidence_ids"] = [f"opaque-{platform['target']}-{case['case_id']}"]
        platform["metrics"] = {
            "startup_p95_ms": 2_500,
            "idle_cpu_pct": 2.0,
            "active_cpu_pct": 35.0,
            "idle_rss_bytes": 64 * 1024 * 1024,
            "active_rss_bytes": 256 * 1024 * 1024,
            "large_list_p95_ms": 500,
            "graph_p95_ms": 700,
        }
    return evidence


def test_not_run_template_cannot_be_promoted() -> None:
    with pytest.raises(desktop_evidence.EvidenceError, match="not approved"):
        desktop_evidence.validate_evidence(template())


def test_not_run_template_passes_non_promoting_preflight() -> None:
    report = desktop_evidence.validate_preflight(template())

    assert report["preflight_passed"] is True
    assert report["promotable"] is False
    assert report["approved"] is False
    assert report["platform_count"] == 3
    assert report["case_count_per_platform"] == 10


def test_approved_platform_evidence_is_summarized_without_raw_records(tmp_path) -> None:
    path = tmp_path / "desktop-evidence.json"
    path.write_text(json.dumps(approved_evidence()), encoding="utf-8")

    report = desktop_evidence.verify(path)

    assert report["passed"] is True
    assert report["platform_count"] == 3
    assert report["case_count_per_platform"] == 10
    assert report["targets"] == [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ]
    assert len(report["evidence_digest"]) == 71
    assert "evidence_ids" not in json.dumps(report)
    assert "opaque-aarch64-apple-darwin" not in json.dumps(report)


def test_template_contains_the_complete_m12_case_contract() -> None:
    evidence = template()

    for platform in evidence["platforms"]:
        assert {case["case_id"] for case in platform["cases"]} == REQUIRED_CASES


def test_platform_evidence_requires_every_supported_target_and_case() -> None:
    evidence = approved_evidence()
    evidence["platforms"] = evidence["platforms"][:-1]

    with pytest.raises(desktop_evidence.EvidenceError, match="supported platform"):
        desktop_evidence.validate_evidence(evidence)


def test_platform_evidence_rejects_raw_paths_and_secret_fields() -> None:
    evidence = approved_evidence()
    evidence["platforms"][0]["cases"][0]["screenshot_path"] = "/Users/private/screenshot.png"

    with pytest.raises(desktop_evidence.EvidenceError, match="raw evidence field"):
        desktop_evidence.validate_evidence(evidence)


def test_platform_evidence_rejects_resource_threshold_breach() -> None:
    evidence = approved_evidence()
    evidence["platforms"][0]["metrics"]["active_rss_bytes"] = 2 * 1024 * 1024 * 1024 + 1

    with pytest.raises(desktop_evidence.EvidenceError, match="active_rss_bytes"):
        desktop_evidence.validate_evidence(evidence)


def test_platform_evidence_rejects_template_placeholder_digests() -> None:
    evidence = approved_evidence()
    evidence["release"]["release_digest"] = "sha256:" + "0" * 64

    with pytest.raises(desktop_evidence.EvidenceError, match="placeholder"):
        desktop_evidence.validate_evidence(evidence)


def test_platform_evidence_rejects_a_stale_release_version() -> None:
    evidence = approved_evidence()
    evidence["release"]["version"] = "0.56.2"

    with pytest.raises(desktop_evidence.EvidenceError, match="current release"):
        desktop_evidence.validate_evidence(evidence)


def test_platform_evidence_allows_an_explicit_historical_version_override(tmp_path) -> None:
    evidence = approved_evidence()
    evidence["release"]["version"] = "0.56.2"
    path = tmp_path / "historical-desktop-evidence.json"
    path.write_text(json.dumps(evidence), encoding="utf-8")

    report = desktop_evidence.verify(path, expected_version="0.56.2")

    assert report["version"] == "0.56.2"


def test_preflight_records_supported_and_unsupported_platform_scope() -> None:
    report = desktop_evidence.validate_preflight(template())

    assert report["supported_targets"] == [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ]
    assert report["unsupported_targets"] == [
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "aarch64-pc-windows-msvc",
    ]


def test_platform_evidence_rejects_an_implicit_platform_scope() -> None:
    evidence = approved_evidence()
    del evidence["support_scope"]

    with pytest.raises(desktop_evidence.EvidenceError, match="support_scope"):
        desktop_evidence.validate_evidence(evidence)


def test_desktop_evidence_has_a_complete_sanitized_case_matrix() -> None:
    evidence = template()

    assert set(evidence["case_checks"]) == REQUIRED_CASE_CHECKS
    assert all(evidence["case_checks"][case_id] for case_id in REQUIRED_CASE_CHECKS)


def test_desktop_evidence_rejects_an_incomplete_case_matrix() -> None:
    evidence = approved_evidence()
    evidence["case_checks"]["updater-lifecycle"] = []

    with pytest.raises(desktop_evidence.EvidenceError, match="case_checks"):
        desktop_evidence.validate_evidence(evidence)
