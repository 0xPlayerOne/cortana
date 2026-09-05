#!/usr/bin/env python3
"""Verify sanitized, operator-owned relationship-quality evidence."""

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
CONTRACT_VERSION = "cortana.relationship-quality-private.v3"
ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
REQUIRED_CATEGORIES = {
    "explicit",
    "thread",
    "temporal",
    "entity",
    "backlink",
    "semantic-neighbor",
    "contradiction",
    "supersession",
    "code-dependency",
}
REQUIRED_METRICS = {
    "edge_precision",
    "edge_coverage",
    "provenance_completeness",
    "invalidation_correctness",
    "task_completion_rate",
    "navigation_step_reduction",
    "retrieval_lift",
    "false_inference_rate",
    "acl_leaks",
    "stale_edge_failures",
    "deleted_record_failures",
    "low_confidence_disclosure_failures",
    "large_neighborhood_failures",
    "latency_p95_ms",
    "peak_rss_bytes",
}
REQUIRED_THRESHOLDS = {
    "min_edge_precision",
    "min_edge_coverage",
    "min_provenance_completeness",
    "min_invalidation_correctness",
    "min_task_completion_rate",
    "min_navigation_step_reduction",
    "min_retrieval_lift",
    "max_false_inference_rate",
    "max_acl_leaks",
    "max_stale_edge_failures",
    "max_deleted_record_failures",
    "max_low_confidence_disclosure_failures",
    "max_large_neighborhood_failures",
    "max_latency_p95_ms",
    "max_peak_rss_bytes",
}
REQUIRED_TASKS = {
    "graph-assisted-navigation",
    "relationship-explanation",
    "graph-assisted-retrieval",
    "graph-assisted-memory-review",
}
REQUIRED_CASE_METRICS = {
    "edge_precision",
    "edge_coverage",
    "provenance_completeness",
    "invalidation_correctness",
    "false_inference_rate",
    "acl_leaks",
    "stale_edge_failures",
    "deleted_record_failures",
    "low_confidence_disclosure_failures",
    "large_neighborhood_failures",
    "latency_p95_ms",
    "peak_rss_bytes",
}
FORBIDDEN_KEYS = {
    "answer",
    "content",
    "private_path",
    "query",
    "raw",
    "source_content",
    "token",
}
OPAQUE_IDENTIFIER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
TOP_LEVEL_KEYS = {
    "approved",
    "contract_version",
    "governance",
    "graph_activation_authorized",
    "metrics",
    "raw_data_location",
    "release_version",
    "relationship_cases",
    "release_policy",
    "thresholds",
    "user_tasks",
}
GOVERNANCE_KEYS = {"corpus_revision", "deletion_contact", "reviewer_ids", "secrets_allowed"}
CASE_KEYS = {"category", "edge_kinds", "id", "metrics", "result"}
TASK_KEYS = {
    "control",
    "control_steps",
    "id",
    "result",
    "retrieval_lift",
    "task_success_rate",
    "treatment",
    "treatment_steps",
}
POLICY_KEYS = {
    "enabled_by_default",
    "graph_required_for_exact_document",
    "graph_required_for_search",
    "optional_edge_kinds",
}


class EvidenceError(ValueError):
    """Raised when external relationship evidence is incomplete or unsafe."""


def current_project_version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r"(?m)^version\s*=\s*[\"'](?P<version>\d+\.\d+\.\d+)[\"']\s*$", cargo)
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


def _string(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value or len(value) > 128:
        raise EvidenceError(f"{name} must be a bounded non-empty string")
    if not OPAQUE_IDENTIFIER.fullmatch(value):
        raise EvidenceError(f"{name} must be an opaque identifier")
    return value


def _strings(value: Any, name: str, *, non_empty: bool = True) -> list[str]:
    if not isinstance(value, list) or (non_empty and not value):
        raise EvidenceError(f"{name} must be a non-empty string array")
    return [_string(item, f"{name}[]") for item in value]


def _check_keys(value: Any) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_KEYS:
                raise EvidenceError(f"raw evidence field {key!r} is not allowed")
            _check_keys(child)
    elif isinstance(value, list):
        for child in value:
            _check_keys(child)


def _bounded_number(value: Any, name: str, *, minimum: float, maximum: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise EvidenceError(f"{name} must be numeric")
    if not minimum <= value <= maximum:
        raise EvidenceError(f"{name} is outside its allowed bound")
    return float(value)


def _validate_case(case: Any) -> str:
    value = _object(case, "relationship case")
    _exact_keys(value, CASE_KEYS, "relationship case")
    _string(value.get("id"), "relationship case id")
    category = _string(value.get("category"), "relationship case category")
    _strings(value.get("edge_kinds"), "relationship case edge_kinds")
    if value.get("result") != "passed":
        raise EvidenceError("every relationship case must be passed")
    return category


def _validate_preflight_case(case: Any) -> str:
    value = _object(case, "relationship case")
    _exact_keys(value, CASE_KEYS, "relationship case")
    _string(value.get("id"), "relationship case id")
    category = _string(value.get("category"), "relationship case category")
    _strings(value.get("edge_kinds"), "relationship case edge_kinds")
    if value.get("result") != "not-run":
        raise EvidenceError("preflight input must keep every relationship case not-run")
    return category


def _validate_case_metrics(case: Any, thresholds: Mapping[str, Any], *, preflight: bool) -> None:
    value = _object(case, "relationship case")
    case_id = _string(value.get("id"), "relationship case id")
    metrics = _object(value.get("metrics"), f"{case_id}.metrics")
    if set(metrics) != REQUIRED_CASE_METRICS:
        raise EvidenceError(f"{case_id} case metrics are incomplete or unknown")
    if preflight:
        if any(metric is not None for metric in metrics.values()):
            raise EvidenceError(f"{case_id} preflight case metrics must be null")
        return

    for name in (
        "edge_precision",
        "edge_coverage",
        "provenance_completeness",
        "invalidation_correctness",
    ):
        _bounded_number(metrics[name], f"{case_id}.metrics.{name}", minimum=0, maximum=1)
        threshold = thresholds[f"min_{name}"]
        if metrics[name] < threshold:
            raise EvidenceError(f"{case_id}.metrics.{name} is below its threshold")

    _bounded_number(
        metrics["false_inference_rate"],
        f"{case_id}.metrics.false_inference_rate",
        minimum=0,
        maximum=1,
    )
    if metrics["false_inference_rate"] > thresholds["max_false_inference_rate"]:
        raise EvidenceError(f"{case_id}.metrics.false_inference_rate exceeds its threshold")

    for metric, threshold in (
        ("acl_leaks", "max_acl_leaks"),
        ("stale_edge_failures", "max_stale_edge_failures"),
        ("deleted_record_failures", "max_deleted_record_failures"),
        ("low_confidence_disclosure_failures", "max_low_confidence_disclosure_failures"),
        ("large_neighborhood_failures", "max_large_neighborhood_failures"),
    ):
        metric_value = metrics[metric]
        threshold_value = thresholds[threshold]
        if (
            isinstance(metric_value, bool)
            or not isinstance(metric_value, int)
            or metric_value < 0
            or metric_value > 1_000_000
            or isinstance(threshold_value, bool)
            or not isinstance(threshold_value, int)
            or threshold_value < 0
            or metric_value > threshold_value
        ):
            raise EvidenceError(f"{case_id}.metrics.{metric} exceeds its threshold or is invalid")

    latency = metrics["latency_p95_ms"]
    rss = metrics["peak_rss_bytes"]
    if (
        isinstance(latency, bool)
        or not isinstance(latency, int)
        or latency < 0
        or latency > 60_000
        or latency > thresholds["max_latency_p95_ms"]
        or isinstance(rss, bool)
        or not isinstance(rss, int)
        or rss < 0
        or rss > 2 * 1024 * 1024 * 1024
        or rss > thresholds["max_peak_rss_bytes"]
    ):
        raise EvidenceError(f"{case_id}.metrics resource values exceed their thresholds")


def _validate_task(task: Any, thresholds: Mapping[str, Any]) -> str:
    value = _object(task, "user task")
    _exact_keys(value, TASK_KEYS, "user task")
    task_id = _string(value.get("id"), "user task id")
    _string(value.get("control"), "user task control")
    _string(value.get("treatment"), "user task treatment")
    if value.get("result") != "passed":
        raise EvidenceError("every user task must be passed")
    _bounded_number(
        value.get("task_success_rate"), f"{task_id}.task_success_rate", minimum=0, maximum=1
    )
    if value["task_success_rate"] < thresholds["min_task_completion_rate"]:
        raise EvidenceError(f"{task_id}.task_success_rate is below its threshold")
    for field in ("control_steps", "treatment_steps"):
        steps = value.get(field)
        if isinstance(steps, bool) or not isinstance(steps, int) or steps < 0 or steps > 10_000:
            raise EvidenceError(f"{task_id}.{field} must be a bounded non-negative integer")
    _bounded_number(value.get("retrieval_lift"), f"{task_id}.retrieval_lift", minimum=-1, maximum=1)
    if value["retrieval_lift"] < thresholds["min_retrieval_lift"]:
        raise EvidenceError(f"{task_id}.retrieval_lift is below its threshold")
    return task_id


def _validate_preflight_task(task: Any) -> str:
    value = _object(task, "user task")
    _exact_keys(value, TASK_KEYS, "user task")
    task_id = _string(value.get("id"), "user task id")
    _string(value.get("control"), "user task control")
    _string(value.get("treatment"), "user task treatment")
    if value.get("result") != "not-run":
        raise EvidenceError("preflight input must keep every user task not-run")
    if any(value.get(field) is not None for field in ("task_success_rate", "retrieval_lift")):
        raise EvidenceError(f"{task_id} preflight metrics must be null")
    if any(value.get(field) is not None for field in ("control_steps", "treatment_steps")):
        raise EvidenceError(f"{task_id} preflight step counts must be null")
    return task_id


def _validate_metadata(
    value: Mapping[str, Any], *, expected_version: str | None = None
) -> tuple[str, Mapping[str, Any]]:
    _check_keys(value)
    _exact_keys(value, TOP_LEVEL_KEYS, "evidence")
    if value.get("contract_version") != CONTRACT_VERSION:
        raise EvidenceError("unsupported relationship-quality contract")
    release_version = value.get("release_version")
    if not isinstance(release_version, str) or not SEMVER.fullmatch(release_version):
        raise EvidenceError("release_version must be plain semantic version text")
    expected = expected_version or current_project_version()
    if not SEMVER.fullmatch(expected) or release_version != expected:
        raise EvidenceError(f"release version does not match current release {expected}")
    if value.get("raw_data_location") != "external-encrypted-store":
        raise EvidenceError("raw relationship data must remain external and encrypted")
    if value.get("graph_activation_authorized") is not False:
        raise EvidenceError("relationship evidence cannot authorize graph activation")

    governance = _object(value.get("governance"), "governance")
    _exact_keys(governance, GOVERNANCE_KEYS, "governance")
    reviewers = _strings(governance.get("reviewer_ids"), "governance.reviewer_ids")
    if len(reviewers) > 16:
        raise EvidenceError("governance.reviewer_ids exceeds the reviewer bound")
    _string(governance.get("corpus_revision"), "governance.corpus_revision")
    _string(governance.get("deletion_contact"), "governance.deletion_contact")
    if governance.get("secrets_allowed") is not False:
        raise EvidenceError("secrets are not allowed in relationship evidence")
    return release_version, governance


def _validate_case_and_task_sets(
    value: Mapping[str, Any], thresholds: Mapping[str, Any], *, preflight: bool
) -> tuple[list[Any], list[Any]]:
    cases = value.get("relationship_cases")
    if not isinstance(cases, list) or len(cases) != len(REQUIRED_CATEGORIES):
        raise EvidenceError("relationship evidence must contain all required categories")
    case_validator = _validate_preflight_case if preflight else _validate_case
    categories = {case_validator(case) for case in cases}
    if categories != REQUIRED_CATEGORIES:
        raise EvidenceError("relationship evidence categories are incomplete or duplicated")
    for case in cases:
        _validate_case_metrics(case, thresholds, preflight=preflight)

    tasks = value.get("user_tasks")
    if not isinstance(tasks, list) or len(tasks) != len(REQUIRED_TASKS):
        raise EvidenceError("relationship evidence must contain all required user tasks")
    task_validator = (
        _validate_preflight_task if preflight else lambda task: _validate_task(task, thresholds)
    )
    task_ids = {task_validator(task) for task in tasks}
    if task_ids != REQUIRED_TASKS:
        raise EvidenceError("relationship evidence user tasks are incomplete or duplicated")
    return cases, tasks


def _validate_policy(policy: Any) -> Mapping[str, Any]:
    policy = _object(policy, "release_policy")
    _exact_keys(policy, POLICY_KEYS, "release_policy")
    if (
        policy.get("graph_required_for_search") is not False
        or policy.get("graph_required_for_exact_document") is not False
    ):
        raise EvidenceError("graph cannot be required for search or exact document access")
    enabled = _strings(policy.get("enabled_by_default"), "release_policy.enabled_by_default")
    optional = _strings(policy.get("optional_edge_kinds"), "release_policy.optional_edge_kinds")
    if len(set(enabled)) != len(enabled):
        raise EvidenceError("release_policy.enabled_by_default must not contain duplicates")
    if len(set(optional)) != len(optional):
        raise EvidenceError("release_policy.optional_edge_kinds must not contain duplicates")
    if set(enabled) & set(optional):
        raise EvidenceError("an edge kind cannot be both enabled and optional")
    return policy


def _validate_edge_kind_coverage(cases: Sequence[Any], policy: Mapping[str, Any]) -> None:
    observed = {
        edge_kind
        for case in cases
        for edge_kind in _object(case, "relationship case")["edge_kinds"]
    }
    released = set(policy["enabled_by_default"]) | set(policy["optional_edge_kinds"])
    if observed != released:
        missing = sorted(released - observed)
        extra = sorted(observed - released)
        raise EvidenceError(
            f"relationship evidence edge kinds are incomplete: missing={missing}, extra={extra}"
        )


def _validate_threshold_shape(thresholds: Any) -> Mapping[str, Any]:
    thresholds = _object(thresholds, "thresholds")
    if set(thresholds) != REQUIRED_THRESHOLDS:
        raise EvidenceError("thresholds must contain exactly the relationship-quality thresholds")
    metric_values: dict[str, int | float] = {
        "edge_precision": thresholds["min_edge_precision"],
        "edge_coverage": thresholds["min_edge_coverage"],
        "provenance_completeness": thresholds["min_provenance_completeness"],
        "invalidation_correctness": thresholds["min_invalidation_correctness"],
        "task_completion_rate": thresholds["min_task_completion_rate"],
        "navigation_step_reduction": thresholds["min_navigation_step_reduction"],
        "retrieval_lift": thresholds["min_retrieval_lift"],
        "false_inference_rate": thresholds["max_false_inference_rate"],
        "acl_leaks": thresholds["max_acl_leaks"],
        "stale_edge_failures": thresholds["max_stale_edge_failures"],
        "deleted_record_failures": thresholds["max_deleted_record_failures"],
        "low_confidence_disclosure_failures": thresholds["max_low_confidence_disclosure_failures"],
        "large_neighborhood_failures": thresholds["max_large_neighborhood_failures"],
        "latency_p95_ms": 0,
        "peak_rss_bytes": 0,
    }
    _validate_metrics(metric_values, thresholds)
    return thresholds


def _validate_metrics(metrics: Mapping[str, Any], thresholds: Mapping[str, Any]) -> None:
    if set(metrics) != REQUIRED_METRICS:
        raise EvidenceError("metrics must contain exactly the relationship-quality metrics")
    if set(thresholds) != REQUIRED_THRESHOLDS:
        raise EvidenceError("thresholds must contain exactly the relationship-quality thresholds")

    for name in (
        "edge_precision",
        "edge_coverage",
        "provenance_completeness",
        "invalidation_correctness",
        "task_completion_rate",
    ):
        _bounded_number(metrics[name], f"metrics.{name}", minimum=0, maximum=1)
        _bounded_number(thresholds[f"min_{name}"], f"thresholds.min_{name}", minimum=0, maximum=1)
        if metrics[name] < thresholds[f"min_{name}"]:
            raise EvidenceError(f"metrics.{name} is below its threshold")

    _bounded_number(
        metrics["navigation_step_reduction"],
        "metrics.navigation_step_reduction",
        minimum=0,
        maximum=1,
    )
    _bounded_number(
        thresholds["min_navigation_step_reduction"],
        "thresholds.min_navigation_step_reduction",
        minimum=0,
        maximum=1,
    )
    if metrics["navigation_step_reduction"] < thresholds["min_navigation_step_reduction"]:
        raise EvidenceError("navigation step reduction is below its threshold")

    _bounded_number(metrics["retrieval_lift"], "metrics.retrieval_lift", minimum=-1, maximum=1)
    _bounded_number(
        thresholds["min_retrieval_lift"], "thresholds.min_retrieval_lift", minimum=-1, maximum=1
    )
    if metrics["retrieval_lift"] < thresholds["min_retrieval_lift"]:
        raise EvidenceError("retrieval lift is below its threshold")

    for name in (
        "false_inference_rate",
        "max_false_inference_rate",
    ):
        source = metrics if name in metrics else thresholds
        _bounded_number(
            source[name],
            f"{'metrics' if source is metrics else 'thresholds'}.{name}",
            minimum=0,
            maximum=1,
        )
    if metrics["false_inference_rate"] > thresholds["max_false_inference_rate"]:
        raise EvidenceError("false inference rate exceeds its threshold")

    for metric, threshold in (
        ("acl_leaks", "max_acl_leaks"),
        ("stale_edge_failures", "max_stale_edge_failures"),
        ("deleted_record_failures", "max_deleted_record_failures"),
        ("low_confidence_disclosure_failures", "max_low_confidence_disclosure_failures"),
        ("large_neighborhood_failures", "max_large_neighborhood_failures"),
    ):
        metric_value = metrics[metric]
        threshold_value = thresholds[threshold]
        if (
            isinstance(metric_value, bool)
            or not isinstance(metric_value, int)
            or metric_value < 0
            or metric_value > 1_000_000
            or isinstance(threshold_value, bool)
            or not isinstance(threshold_value, int)
            or threshold_value < 0
            or metric_value > threshold_value
        ):
            raise EvidenceError(f"{metric} exceeds its threshold or is invalid")

    latency = metrics["latency_p95_ms"]
    max_latency = thresholds["max_latency_p95_ms"]
    rss = metrics["peak_rss_bytes"]
    max_rss = thresholds["max_peak_rss_bytes"]
    if (
        isinstance(latency, bool)
        or not isinstance(latency, int)
        or latency < 0
        or latency > 60_000
        or isinstance(max_latency, bool)
        or not isinstance(max_latency, int)
        or not 0 < max_latency <= 60_000
        or latency > max_latency
        or isinstance(rss, bool)
        or not isinstance(rss, int)
        or rss < 0
        or rss > 2 * 1024 * 1024 * 1024
        or isinstance(max_rss, bool)
        or not isinstance(max_rss, int)
        or not 0 < max_rss <= 2 * 1024 * 1024 * 1024
        or rss > max_rss
    ):
        raise EvidenceError("resource metrics exceed their thresholds or are invalid")


def validate_evidence(
    value: Mapping[str, Any], *, expected_version: str | None = None
) -> dict[str, Any]:
    release_version, governance = _validate_metadata(value, expected_version=expected_version)
    if value.get("approved") is not True:
        raise EvidenceError("relationship-quality evidence is not approved")
    thresholds = _validate_threshold_shape(value.get("thresholds"))
    cases, tasks = _validate_case_and_task_sets(value, thresholds, preflight=False)

    _validate_metrics(_object(value.get("metrics"), "metrics"), thresholds)
    policy = _validate_policy(value.get("release_policy"))
    _validate_edge_kind_coverage(cases, policy)
    reviewers = governance["reviewer_ids"]
    return {
        "contract_version": CONTRACT_VERSION,
        "version": release_version,
        "case_count": len(cases),
        "task_count": len(tasks),
        "reviewer_count": len(reviewers),
        "corpus_revision": governance["corpus_revision"],
        "graph_activation_authorized": False,
    }


def validate_preflight(
    value: Mapping[str, Any], *, expected_version: str | None = None
) -> dict[str, Any]:
    """Validate an unapproved, not-run record without promoting it."""
    release_version, governance = _validate_metadata(value, expected_version=expected_version)
    if value.get("approved") is not False:
        raise EvidenceError("preflight input must remain unapproved")
    thresholds = _validate_threshold_shape(value.get("thresholds"))
    cases, tasks = _validate_case_and_task_sets(value, thresholds, preflight=True)
    metrics = _object(value.get("metrics"), "metrics")
    if set(metrics) != REQUIRED_METRICS or any(metric is not None for metric in metrics.values()):
        raise EvidenceError(
            "preflight metrics must contain exactly null relationship-quality values"
        )
    policy = _validate_policy(value.get("release_policy"))
    _validate_edge_kind_coverage(cases, policy)
    reviewers = governance["reviewer_ids"]
    return {
        "evaluation": "cortana-relationship-quality-private-v3",
        "preflight_passed": True,
        "promotable": False,
        "approved": False,
        "version": release_version,
        "case_count": len(cases),
        "task_count": len(tasks),
        "reviewer_count": len(reviewers),
        "corpus_revision": governance["corpus_revision"],
        "graph_activation_authorized": False,
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
        "evaluation": "cortana-relationship-quality-private-v3",
        "passed": True,
        "evidence_digest": digest,
        **summary,
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, help="sanitized relationship evidence")
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
        raise SystemExit(f"invalid relationship-quality evidence: {error}") from error
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
