import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_RELATIONSHIP_CASES = {
    "explicit-links",
    "thread-series",
    "temporal-adjacency",
    "entity-mentions",
    "backlink-navigation",
    "semantic-neighbors",
    "contradiction-review",
    "supersession-review",
    "code-dependencies",
}
REQUIRED_USER_TASKS = {
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
SPEC = importlib.util.spec_from_file_location(
    "cortana_relationship_quality_evidence",
    ROOT / "scripts/verify-relationship-evidence.py",
)
assert SPEC and SPEC.loader
relationship = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(relationship)


def template() -> dict:
    return json.loads((ROOT / "eval/relationship-quality-private.example.json").read_text())


def approved_evidence() -> dict:
    evidence = template()
    evidence["approved"] = True
    for case in evidence["relationship_cases"]:
        case["result"] = "passed"
        case["metrics"] = {
            "edge_precision": 1.0,
            "edge_coverage": 1.0,
            "provenance_completeness": 1.0,
            "invalidation_correctness": 1.0,
            "false_inference_rate": 0.0,
            "acl_leaks": 0,
            "stale_edge_failures": 0,
            "deleted_record_failures": 0,
            "low_confidence_disclosure_failures": 0,
            "large_neighborhood_failures": 0,
            "latency_p95_ms": 250,
            "peak_rss_bytes": 64 * 1024 * 1024,
        }
    for task in evidence["user_tasks"]:
        task.update(
            {
                "result": "passed",
                "task_success_rate": 1.0,
                "control_steps": 4,
                "treatment_steps": 3,
                "retrieval_lift": 0.25,
            }
        )
    evidence["metrics"] = {
        "edge_precision": 1.0,
        "edge_coverage": 1.0,
        "provenance_completeness": 1.0,
        "invalidation_correctness": 1.0,
        "task_completion_rate": 1.0,
        "navigation_step_reduction": 0.25,
        "retrieval_lift": 0.25,
        "false_inference_rate": 0.0,
        "acl_leaks": 0,
        "stale_edge_failures": 0,
        "deleted_record_failures": 0,
        "low_confidence_disclosure_failures": 0,
        "large_neighborhood_failures": 0,
        "latency_p95_ms": 250,
        "peak_rss_bytes": 64 * 1024 * 1024,
    }
    return evidence


def test_not_run_template_cannot_be_promoted() -> None:
    with pytest.raises(relationship.EvidenceError, match="not approved"):
        relationship.validate_evidence(template())


def test_not_run_template_passes_non_promoting_preflight() -> None:
    report = relationship.validate_preflight(template())

    assert report["preflight_passed"] is True
    assert report["promotable"] is False
    assert report["approved"] is False
    assert report["case_count"] == 9
    assert report["task_count"] == 4


def test_approved_relationship_evidence_is_summarized_without_raw_cases(tmp_path) -> None:
    path = tmp_path / "evidence.json"
    path.write_text(json.dumps(approved_evidence()), encoding="utf-8")

    report = relationship.verify(path)

    assert report["passed"] is True
    assert report["case_count"] == 9
    assert report["task_count"] == 4
    assert report["graph_activation_authorized"] is False
    assert len(report["evidence_digest"]) == 71
    assert "explicit-links" not in json.dumps(report)
    assert "opaque-approved-corpus-revision" in json.dumps(report)


def test_relationship_evidence_rejects_a_stale_release_version() -> None:
    evidence = approved_evidence()
    evidence["release_version"] = "0.56.2"

    with pytest.raises(relationship.EvidenceError, match="release version"):
        relationship.validate_evidence(evidence, expected_version="0.56.3")


def test_relationship_evidence_rejects_the_pre_release_binding_contract() -> None:
    evidence = approved_evidence()
    evidence["contract_version"] = "cortana.relationship-quality-private.v1"

    with pytest.raises(relationship.EvidenceError, match="unsupported"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_supports_an_explicit_historical_version_override(tmp_path) -> None:
    evidence = approved_evidence()
    evidence["release_version"] = "0.39.0"
    path = tmp_path / "historical-evidence.json"
    path.write_text(json.dumps(evidence), encoding="utf-8")

    report = relationship.verify(path, expected_version="0.39.0")

    assert report["passed"] is True
    assert report["version"] == "0.39.0"


def test_template_contains_the_complete_relationship_and_task_contract() -> None:
    evidence = template()

    assert {case["id"] for case in evidence["relationship_cases"]} == REQUIRED_RELATIONSHIP_CASES
    assert {task["id"] for task in evidence["user_tasks"]} == REQUIRED_USER_TASKS


def test_relationship_evidence_rejects_raw_query_fields() -> None:
    evidence = approved_evidence()
    evidence["user_tasks"][0]["query"] = "private query must not be recorded"

    with pytest.raises(relationship.EvidenceError, match="raw evidence field"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_rejects_unknown_free_form_fields() -> None:
    evidence = approved_evidence()
    evidence["user_tasks"][0]["reviewer_notes"] = "private content must not be recorded"

    with pytest.raises(relationship.EvidenceError, match="unknown"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_rejects_graph_core_dependency() -> None:
    evidence = approved_evidence()
    evidence["release_policy"]["graph_required_for_search"] = True

    with pytest.raises(relationship.EvidenceError, match="cannot be required"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_requires_per_case_quality_metrics() -> None:
    evidence = approved_evidence()
    del evidence["relationship_cases"][0]["metrics"]

    with pytest.raises(relationship.EvidenceError, match="metrics"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_rejects_a_per_case_quality_failure() -> None:
    evidence = approved_evidence()
    evidence["relationship_cases"][0]["metrics"]["edge_precision"] = 0.5

    with pytest.raises(relationship.EvidenceError, match="edge_precision"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_requires_each_user_task_to_meet_quality_thresholds() -> None:
    evidence = approved_evidence()
    evidence["user_tasks"][0]["task_success_rate"] = 0.5

    with pytest.raises(relationship.EvidenceError, match="task_success_rate"):
        relationship.validate_evidence(evidence)

    evidence = approved_evidence()
    evidence["user_tasks"][0]["retrieval_lift"] = -0.1

    with pytest.raises(relationship.EvidenceError, match="retrieval_lift"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_covers_every_released_edge_kind() -> None:
    evidence = approved_evidence()
    evidence["relationship_cases"][0]["edge_kinds"].remove("contains")

    with pytest.raises(relationship.EvidenceError, match="edge kinds"):
        relationship.validate_evidence(evidence)


def test_relationship_evidence_rejects_ambiguous_edge_policy_sets() -> None:
    evidence = approved_evidence()
    evidence["release_policy"]["optional_edge_kinds"].append("contains")

    with pytest.raises(relationship.EvidenceError, match="both enabled and optional"):
        relationship.validate_evidence(evidence)
