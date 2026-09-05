#!/usr/bin/env python3
"""Verify sanitized, operator-owned packaged Desktop acceptance evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

MAX_EVIDENCE_BYTES = 1 * 1024 * 1024
CONTRACT_VERSION = "cortana.desktop-acceptance-private.v2"
ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "aarch64-apple-darwin": ("macOS", "arm64", "dmg"),
    "x86_64-unknown-linux-gnu": ("Linux", "x64", "AppImage"),
    "x86_64-pc-windows-msvc": ("Windows", "x64", "msi"),
}
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
CASE_CHECKS = {
    "clean-install-first-run": (
        "install",
        "launch",
        "core-sidecar",
        "optional-tooling",
        "approval-gated-setup",
        "progress-cancel-retry",
        "workspace-create-switch",
        "readiness",
        "query-only-default",
        "no-implicit-source-sync-model-memory-write",
        "package-version-agreement",
    ),
    "source-authorization-discovery": (
        "workspace-isolation",
        "provider-authorization",
        "provider-discovery",
        "source-scope",
        "read-only-validation",
        "no-implicit-sync",
        "credential-redaction",
        "denied-revoked-malformed-recovery",
    ),
    "services-tray-autostart": (
        "service-lifecycle",
        "tray-window-quit",
        "single-instance-autostart",
        "crash-stale-state-port-conflict",
        "sleep-wake-login-restart",
        "durable-status",
        "retrieval-preserved",
        "allowlisted-boundary",
        "no-implicit-sync",
        "mcp-http-lifecycle",
    ),
    "backup-restore-dialogs": (
        "native-file-directory-dialogs",
        "redacted-import-export",
        "verified-snapshot",
        "explicit-restore-confirmation",
        "recovery-copy",
        "cancelled-inaccessible-low-disk-path-attack",
        "corrupt-version-mismatch",
        "post-restore-search-memory-acl",
        "packaged-rollback",
    ),
    "updater-lifecycle": (
        "version-channel-release-notes",
        "approval-download-install",
        "progress-cancellation",
        "signature-archive-version-safety",
        "application-service-restart",
        "data-policy-preservation",
        "network-malformed-disk-failure",
        "incompatible-interrupted-restart",
        "upgrade",
        "rollback",
        "no-recurring-sync-or-synthesis",
    ),
    "recovery-failure-paths": (
        "corrupt-config",
        "missing-sidecar",
        "stale-service",
        "provider-outage",
        "low-disk",
        "permission-denial",
        "crash-restart",
        "upgrade-recovery",
        "actionable-remediation",
    ),
    "accessibility-screen-reader": (
        "keyboard-focus",
        "screen-reader",
        "contrast",
        "reduced-motion",
        "status-error-semantics",
        "zoom-responsive",
        "target-size",
        "error-recovery",
        "packaged-screen-reader",
    ),
    "resource-large-list": (
        "startup-idle-active-resource",
        "large-list-virtualization",
        "graph-bounds",
        "background-service-overhead",
        "slow-storage-low-memory",
        "cancellation-cleanup",
        "security-regressions",
    ),
    "uninstall-and-data-preservation": (
        "uninstall-record",
        "data-preservation",
        "credentials-index-memory-backups",
        "services-cleanup",
        "reinstall-recovery",
    ),
    "os-trust": (
        "package-signing",
        "gatekeeper-smartscreen-trust",
        "native-boundary",
        "supported-session-display",
        "platform-specific-credentials",
        "release-asset-identity",
    ),
}
CASE_KEYS = {"case_id", "method", "result", "evidence_ids"}
PLATFORM_KEYS = {
    "target",
    "platform",
    "architecture",
    "package_digest",
    "installer",
    "query_only_default",
    "no_implicit_side_effects",
    "cases",
    "metrics",
}
METRICS = {
    "startup_p95_ms",
    "idle_cpu_pct",
    "active_cpu_pct",
    "idle_rss_bytes",
    "active_rss_bytes",
    "large_list_p95_ms",
    "graph_p95_ms",
}
THRESHOLDS = {
    "max_startup_p95_ms",
    "max_idle_cpu_pct",
    "max_active_cpu_pct",
    "max_idle_rss_bytes",
    "max_active_rss_bytes",
    "max_large_list_p95_ms",
    "max_graph_p95_ms",
}
TOP_LEVEL_KEYS = {
    "case_checks",
    "contract_version",
    "approved",
    "governance",
    "release",
    "support_scope",
    "platforms",
    "thresholds",
}
SUPPORT_SCOPE_KEYS = {"supported_targets", "unsupported_targets"}
GOVERNANCE_KEYS = {
    "raw_data_location",
    "reviewer_ids",
    "secrets_allowed",
    "private_paths_allowed",
    "retention_days",
    "deletion_contact",
}
RELEASE_KEYS = {"version", "release_digest", "source"}
FORBIDDEN_KEYS = {
    "answer",
    "content",
    "credential",
    "credentials",
    "log_path",
    "path",
    "private_path",
    "query",
    "raw",
    "secret",
    "screenshot",
    "screenshot_path",
    "source_content",
    "token",
}
OPAQUE_IDENTIFIER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
PRIVATE_PATH = re.compile(
    r"(?:/Users/|/private/|/home/|/tmp/|/var/folders/|[A-Za-z]:\\|\\\\|file://)",
    re.IGNORECASE,
)
PROJECT_VERSION = re.compile(r"(?m)^version\s*=\s*[\"'](?P<version>\d+\.\d+\.\d+)[\"']\s*$")


class EvidenceError(ValueError):
    """Raised when external Desktop evidence is incomplete or unsafe."""


def current_project_version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = PROJECT_VERSION.search(cargo)
    if match is None:
        raise EvidenceError("current project release version is unavailable")
    return match.group("version")


def _object(value: Any, name: str) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError(f"{name} must be an object")
    return value


def _exact_keys(value: Mapping[str, Any], allowed: set[str], name: str) -> None:
    actual = set(value)
    if actual != allowed:
        missing = sorted(allowed - actual)
        extra = sorted(actual - allowed)
        raise EvidenceError(
            f"{name} fields are incomplete or unknown: missing={missing}, extra={extra}"
        )


def _opaque(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value or len(value) > 128:
        raise EvidenceError(f"{name} must be a bounded non-empty string")
    if not OPAQUE_IDENTIFIER.fullmatch(value):
        raise EvidenceError(f"{name} must be an opaque identifier")
    return value


def _strings(value: Any, name: str, *, non_empty: bool = True, maximum: int = 16) -> list[str]:
    if (
        not isinstance(value, list)
        or (non_empty and not value)
        or len(value) > maximum
        or any(not isinstance(item, str) for item in value)
    ):
        raise EvidenceError(f"{name} must be a bounded string array")
    return [_opaque(item, f"{name}[]") for item in value]


def _check_safe_strings(value: Any) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_KEYS:
                raise EvidenceError(f"raw evidence field {key!r} is not allowed")
            _check_safe_strings(child)
    elif isinstance(value, list):
        for child in value:
            _check_safe_strings(child)
    elif isinstance(value, str) and PRIVATE_PATH.search(value):
        raise EvidenceError("private or machine-specific paths are not allowed")


def _digest(value: Any, name: str, *, allow_placeholder: bool = False) -> str:
    if not isinstance(value, str) or not DIGEST.fullmatch(value):
        raise EvidenceError(f"{name} must be a SHA-256 digest")
    if not allow_placeholder and value == "sha256:" + "0" * 64:
        raise EvidenceError(f"{name} is still the template placeholder digest")
    return value


def _bounded_number(value: Any, name: str, *, minimum: float, maximum: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise EvidenceError(f"{name} must be numeric")
    if not minimum <= value <= maximum:
        raise EvidenceError(f"{name} is outside its allowed bound")
    return float(value)


def _bounded_integer(value: Any, name: str, *, minimum: int, maximum: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise EvidenceError(f"{name} must be an integer between {minimum} and {maximum}")
    return value


def _validate_case(case: Any) -> str:
    value = _object(case, "Desktop acceptance case")
    _exact_keys(value, CASE_KEYS, "Desktop acceptance case")
    case_id = _opaque(value.get("case_id"), "Desktop acceptance case id")
    method = value.get("method")
    if method not in {"manual-review", "manual-screen-reader-review"}:
        raise EvidenceError(f"{case_id} has an unsupported evidence method")
    if case_id == "accessibility-screen-reader" and method != "manual-screen-reader-review":
        raise EvidenceError("accessibility evidence requires a screen-reader review method")
    if value.get("result") != "passed":
        raise EvidenceError(f"{case_id} is not passed")
    _strings(value.get("evidence_ids"), f"{case_id}.evidence_ids")
    return case_id


def _validate_case_checks(value: Any) -> Mapping[str, list[str]]:
    case_checks = _object(value, "case_checks")
    if set(case_checks) != set(CASE_CHECKS):
        raise EvidenceError("case_checks must contain every required acceptance case")
    checked: dict[str, list[str]] = {}
    for case_id, required_checks in CASE_CHECKS.items():
        checks = _strings(case_checks[case_id], f"case_checks.{case_id}", maximum=64)
        if len(set(checks)) != len(checks):
            raise EvidenceError(f"case_checks.{case_id} must not contain duplicates")
        if set(checks) != set(required_checks):
            raise EvidenceError(f"case_checks.{case_id} is incomplete or unknown")
        checked[case_id] = checks
    return checked


def _validate_preflight_case(case: Any) -> str:
    value = _object(case, "Desktop acceptance case")
    _exact_keys(value, CASE_KEYS, "Desktop acceptance case")
    case_id = _opaque(value.get("case_id"), "Desktop acceptance case id")
    method = value.get("method")
    if method not in {"manual-review", "manual-screen-reader-review"}:
        raise EvidenceError(f"{case_id} has an unsupported evidence method")
    if case_id == "accessibility-screen-reader" and method != "manual-screen-reader-review":
        raise EvidenceError("accessibility evidence requires a screen-reader review method")
    if value.get("result") != "not-run":
        raise EvidenceError(f"preflight input must keep {case_id} not-run")
    _strings(value.get("evidence_ids"), f"{case_id}.evidence_ids")
    return case_id


def _validate_thresholds(thresholds: Mapping[str, Any]) -> None:
    if set(thresholds) != THRESHOLDS:
        raise EvidenceError("thresholds must contain exactly the Desktop resource thresholds")
    for name in ("max_startup_p95_ms", "max_large_list_p95_ms", "max_graph_p95_ms"):
        _bounded_integer(thresholds[name], f"thresholds.{name}", minimum=1, maximum=120_000)
    for name in ("max_idle_rss_bytes", "max_active_rss_bytes"):
        _bounded_integer(thresholds[name], f"thresholds.{name}", minimum=1, maximum=8 * 1024**3)
    for name in ("max_idle_cpu_pct", "max_active_cpu_pct"):
        _bounded_number(thresholds[name], f"thresholds.{name}", minimum=0, maximum=100)


def _validate_support_scope(scope: Any) -> Mapping[str, Any]:
    scope = _object(scope, "support_scope")
    _exact_keys(scope, SUPPORT_SCOPE_KEYS, "support_scope")
    supported = _strings(scope.get("supported_targets"), "support_scope.supported_targets")
    if set(supported) != set(TARGETS) or len(supported) != len(TARGETS):
        raise EvidenceError(
            "support_scope.supported_targets must list every supported target exactly once"
        )
    unsupported = _strings(
        scope.get("unsupported_targets"), "support_scope.unsupported_targets", maximum=64
    )
    if not unsupported:
        raise EvidenceError("support_scope.unsupported_targets must name unsupported targets")
    if len(set(unsupported)) != len(unsupported):
        raise EvidenceError("support_scope.unsupported_targets must not contain duplicates")
    overlap = set(unsupported) & set(TARGETS)
    if overlap:
        raise EvidenceError(
            f"support_scope marks supported targets as unsupported: {sorted(overlap)}"
        )
    return scope


def _validate_metrics(
    metrics: Mapping[str, Any], thresholds: Mapping[str, Any], target: str
) -> None:
    if set(metrics) != METRICS:
        raise EvidenceError(f"{target}.metrics must contain exactly the resource metrics")
    for name in ("startup_p95_ms", "idle_rss_bytes", "active_rss_bytes"):
        _bounded_integer(metrics[name], f"{target}.metrics.{name}", minimum=0, maximum=8 * 1024**3)
    for name in ("large_list_p95_ms", "graph_p95_ms"):
        _bounded_integer(metrics[name], f"{target}.metrics.{name}", minimum=0, maximum=120_000)
    for name in ("idle_cpu_pct", "active_cpu_pct"):
        _bounded_number(metrics[name], f"{target}.metrics.{name}", minimum=0, maximum=100)

    _validate_thresholds(thresholds)

    for metric, threshold in (
        ("startup_p95_ms", "max_startup_p95_ms"),
        ("idle_cpu_pct", "max_idle_cpu_pct"),
        ("active_cpu_pct", "max_active_cpu_pct"),
        ("idle_rss_bytes", "max_idle_rss_bytes"),
        ("active_rss_bytes", "max_active_rss_bytes"),
        ("large_list_p95_ms", "max_large_list_p95_ms"),
        ("graph_p95_ms", "max_graph_p95_ms"),
    ):
        if metrics[metric] > thresholds[threshold]:
            raise EvidenceError(f"{target}.metrics.{metric} exceeds its threshold")


def _validate_platform(platform: Any, thresholds: Mapping[str, Any]) -> str:
    value = _object(platform, "Desktop platform evidence")
    _exact_keys(value, PLATFORM_KEYS, "Desktop platform evidence")
    target = _opaque(value.get("target"), "Desktop target")
    if target not in TARGETS:
        raise EvidenceError(f"unsupported platform target: {target}")
    expected_platform, expected_architecture, expected_installer = TARGETS[target]
    if (
        value.get("platform") != expected_platform
        or value.get("architecture") != expected_architecture
    ):
        raise EvidenceError(f"{target} platform descriptor does not match its target")
    if value.get("installer") != expected_installer:
        raise EvidenceError(f"{target} installer does not match its supported package")
    _digest(value.get("package_digest"), f"{target}.package_digest")
    if value.get("query_only_default") is not True:
        raise EvidenceError(f"{target} did not preserve query-only first-run behavior")
    if value.get("no_implicit_side_effects") is not True:
        raise EvidenceError(f"{target} recorded an implicit privileged or data-moving action")

    cases = value.get("cases")
    if not isinstance(cases, list) or len(cases) != len(REQUIRED_CASES):
        raise EvidenceError(f"{target} must contain every required Desktop acceptance case")
    case_ids = {_validate_case(case) for case in cases}
    if case_ids != REQUIRED_CASES:
        raise EvidenceError(f"{target} acceptance cases are incomplete or duplicated")
    _validate_metrics(_object(value.get("metrics"), f"{target}.metrics"), thresholds, target)
    return target


def validate_evidence(
    value: Mapping[str, Any], *, expected_version: str | None = None
) -> dict[str, Any]:
    _check_safe_strings(value)
    _exact_keys(value, TOP_LEVEL_KEYS, "Desktop acceptance evidence")
    if value.get("contract_version") != CONTRACT_VERSION:
        raise EvidenceError("unsupported Desktop acceptance contract")
    if value.get("approved") is not True:
        raise EvidenceError("Desktop acceptance evidence is not approved")

    case_checks = _validate_case_checks(value.get("case_checks"))
    support_scope = _validate_support_scope(value.get("support_scope"))

    governance = _object(value.get("governance"), "governance")
    _exact_keys(governance, GOVERNANCE_KEYS, "governance")
    if governance.get("raw_data_location") != "external-encrypted-store":
        raise EvidenceError("raw Desktop evidence must remain in an external encrypted store")
    _strings(governance.get("reviewer_ids"), "governance.reviewer_ids")
    if governance.get("secrets_allowed") is not False:
        raise EvidenceError("secrets are not allowed in Desktop evidence")
    if governance.get("private_paths_allowed") is not False:
        raise EvidenceError("private paths are not allowed in Desktop evidence")
    _bounded_integer(
        governance.get("retention_days"), "governance.retention_days", minimum=1, maximum=3650
    )
    _opaque(governance.get("deletion_contact"), "governance.deletion_contact")

    release = _object(value.get("release"), "release")
    _exact_keys(release, RELEASE_KEYS, "release")
    if not isinstance(release.get("version"), str) or not SEMVER.fullmatch(release["version"]):
        raise EvidenceError("release.version must be plain semantic version text")
    expected = expected_version or current_project_version()
    if not SEMVER.fullmatch(expected) or release["version"] != expected:
        raise EvidenceError(f"release.version does not match current release {expected}")
    _digest(release.get("release_digest"), "release.release_digest")
    if release.get("source") != "published-release-assets":
        raise EvidenceError("release.source must be published-release-assets")

    thresholds = _object(value.get("thresholds"), "thresholds")
    if set(thresholds) != THRESHOLDS:
        raise EvidenceError("thresholds must contain exactly the Desktop resource thresholds")
    platforms = value.get("platforms")
    if not isinstance(platforms, list) or len(platforms) != len(TARGETS):
        raise EvidenceError("evidence must contain every supported platform")
    targets = [_validate_platform(platform, thresholds) for platform in platforms]
    if set(targets) != set(TARGETS):
        raise EvidenceError("evidence must contain each supported platform exactly once")

    return {
        "contract_version": CONTRACT_VERSION,
        "version": release["version"],
        "platform_count": len(platforms),
        "case_count_per_platform": len(REQUIRED_CASES),
        "targets": [target for target in TARGETS if target in targets],
        "reviewer_count": len(governance["reviewer_ids"]),
        "raw_data_location": governance["raw_data_location"],
        "supported_targets": list(support_scope["supported_targets"]),
        "unsupported_targets": list(support_scope["unsupported_targets"]),
        "case_checks": case_checks,
    }


def validate_preflight(
    value: Mapping[str, Any], *, expected_version: str | None = None
) -> dict[str, Any]:
    """Validate an unapproved, not-run record without promoting it."""
    _check_safe_strings(value)
    _exact_keys(value, TOP_LEVEL_KEYS, "Desktop acceptance evidence")
    if value.get("contract_version") != CONTRACT_VERSION:
        raise EvidenceError("unsupported Desktop acceptance contract")
    if value.get("approved") is not False:
        raise EvidenceError("preflight input must remain unapproved")

    case_checks = _validate_case_checks(value.get("case_checks"))
    support_scope = _validate_support_scope(value.get("support_scope"))

    governance = _object(value.get("governance"), "governance")
    _exact_keys(governance, GOVERNANCE_KEYS, "governance")
    if governance.get("raw_data_location") != "external-encrypted-store":
        raise EvidenceError("raw Desktop evidence must remain in an external encrypted store")
    _strings(governance.get("reviewer_ids"), "governance.reviewer_ids")
    if governance.get("secrets_allowed") is not False:
        raise EvidenceError("secrets are not allowed in Desktop evidence")
    if governance.get("private_paths_allowed") is not False:
        raise EvidenceError("private paths are not allowed in Desktop evidence")
    _bounded_integer(
        governance.get("retention_days"), "governance.retention_days", minimum=1, maximum=3650
    )
    _opaque(governance.get("deletion_contact"), "governance.deletion_contact")

    release = _object(value.get("release"), "release")
    _exact_keys(release, RELEASE_KEYS, "release")
    if not isinstance(release.get("version"), str) or not SEMVER.fullmatch(release["version"]):
        raise EvidenceError("release.version must be plain semantic version text")
    expected = expected_version or current_project_version()
    if not SEMVER.fullmatch(expected) or release["version"] != expected:
        raise EvidenceError(f"release.version does not match current release {expected}")
    _digest(release.get("release_digest"), "release.release_digest", allow_placeholder=True)
    if release.get("source") != "published-release-assets":
        raise EvidenceError("release.source must be published-release-assets")

    thresholds = _object(value.get("thresholds"), "thresholds")
    _validate_thresholds(thresholds)
    platforms = value.get("platforms")
    if not isinstance(platforms, list) or len(platforms) != len(TARGETS):
        raise EvidenceError("preflight input must contain every supported platform")

    targets: list[str] = []
    for platform in platforms:
        item = _object(platform, "Desktop platform evidence")
        _exact_keys(item, PLATFORM_KEYS, "Desktop platform evidence")
        target = _opaque(item.get("target"), "Desktop target")
        if target not in TARGETS:
            raise EvidenceError(f"unsupported platform target: {target}")
        expected_platform, expected_architecture, expected_installer = TARGETS[target]
        if (
            item.get("platform") != expected_platform
            or item.get("architecture") != expected_architecture
        ):
            raise EvidenceError(f"{target} platform descriptor does not match its target")
        if item.get("installer") != expected_installer:
            raise EvidenceError(f"{target} installer does not match its supported package")
        _digest(item.get("package_digest"), f"{target}.package_digest", allow_placeholder=True)
        if item.get("query_only_default") is not True:
            raise EvidenceError(f"{target} did not preserve query-only first-run behavior")
        if item.get("no_implicit_side_effects") is not True:
            raise EvidenceError(f"{target} recorded an implicit privileged or data-moving action")

        cases = item.get("cases")
        if not isinstance(cases, list) or len(cases) != len(REQUIRED_CASES):
            raise EvidenceError(f"{target} must contain every required Desktop acceptance case")
        case_ids = {_validate_preflight_case(case) for case in cases}
        if case_ids != REQUIRED_CASES:
            raise EvidenceError(f"{target} acceptance cases are incomplete or duplicated")
        metrics = _object(item.get("metrics"), f"{target}.metrics")
        if set(metrics) != METRICS or any(metric is not None for metric in metrics.values()):
            raise EvidenceError(f"{target}.metrics must contain exactly null resource values")
        targets.append(target)

    if set(targets) != set(TARGETS):
        raise EvidenceError("preflight input must contain each supported platform exactly once")
    return {
        "evaluation": "cortana-desktop-acceptance-private-v2",
        "preflight_passed": True,
        "promotable": False,
        "approved": False,
        "version": release["version"],
        "platform_count": len(platforms),
        "case_count_per_platform": len(REQUIRED_CASES),
        "targets": [target for target in TARGETS if target in targets],
        "reviewer_count": len(governance["reviewer_ids"]),
        "raw_data_location": governance["raw_data_location"],
        "supported_targets": list(support_scope["supported_targets"]),
        "unsupported_targets": list(support_scope["unsupported_targets"]),
        "case_checks": case_checks,
    }


def load_evidence(path: Path) -> tuple[dict[str, Any], str]:
    if not path.is_file():
        raise EvidenceError("evidence is not a regular file")
    raw = path.read_bytes()
    if len(raw) > MAX_EVIDENCE_BYTES:
        raise EvidenceError(f"evidence exceeds the {MAX_EVIDENCE_BYTES} byte limit")
    try:
        value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise EvidenceError("evidence is not valid UTF-8 JSON") from error
    if not isinstance(value, dict):
        raise EvidenceError("evidence must be a JSON object")
    return value, f"sha256:{hashlib.sha256(raw).hexdigest()}"


def verify(path: Path, *, expected_version: str | None = None) -> dict[str, Any]:
    evidence, digest = load_evidence(path)
    summary = validate_evidence(evidence, expected_version=expected_version)
    return {
        "evaluation": "cortana-desktop-acceptance-private-v2",
        "passed": True,
        "evidence_digest": digest,
        **summary,
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, help="sanitized Desktop acceptance evidence")
    parser.add_argument(
        "--preflight",
        action="store_true",
        help="validate the unapproved not-run template without promoting it",
    )
    parser.add_argument(
        "--version",
        dest="expected_version",
        help="expected release version; defaults to the current Cargo project version",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    try:
        if arguments.preflight:
            evidence, digest = load_evidence(arguments.evidence)
            report = {
                **validate_preflight(evidence, expected_version=arguments.expected_version),
                "evidence_digest": digest,
            }
        else:
            report = verify(arguments.evidence, expected_version=arguments.expected_version)
    except (OSError, EvidenceError) as error:
        raise SystemExit(f"invalid Desktop acceptance evidence: {error}") from error
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
