from __future__ import annotations

import json

import pytest

from cortana.memory.evaluation import (
    BenchmarkCase,
    EvaluationError,
    default_benchmark_cases,
    evaluate_comparative,
    run_default_benchmark,
)


def test_default_benchmark_is_deterministic_and_requires_material_gain() -> None:
    first = run_default_benchmark()
    second = run_default_benchmark()

    assert first.version == "v1"
    assert first.material_gain is True
    assert first.recall_gain >= first.minimum_recall_gain
    assert first.mrr_gain >= first.minimum_mrr_gain
    assert first.as_json() == second.as_json()
    payload = json.loads(first.as_json())
    assert payload["canonical"]["case_count"] == len(default_benchmark_cases())
    assert payload["combined"]["case_count"] == len(default_benchmark_cases())


def test_gate_fails_when_combined_results_do_not_improve() -> None:
    case = BenchmarkCase(
        case_id="unchanged",
        query="same result",
        relevant_document_ids=("doc-1",),
        canonical_results=("doc-1",),
        combined_results=("doc-1",),
    )

    report = evaluate_comparative((case,), minimum_recall_gain=0.01, minimum_mrr_gain=0.01)

    assert report.material_gain is False
    assert report.recall_gain == 0.0
    assert report.mrr_gain == 0.0


def test_metrics_are_cut_off_at_top_k_and_preserve_case_order() -> None:
    cases = (
        BenchmarkCase(
            case_id="first",
            query="first query",
            relevant_document_ids=("target",),
            canonical_results=("noise", "target"),
            combined_results=("target", "noise"),
        ),
        BenchmarkCase(
            case_id="second",
            query="second query",
            relevant_document_ids=("other",),
            canonical_results=("noise",),
            combined_results=("other",),
        ),
    )

    report = evaluate_comparative(cases, top_k=1, minimum_recall_gain=0.0, minimum_mrr_gain=0.0)

    assert [case.case_id for case in report.canonical.cases] == ["first", "second"]
    assert report.canonical.recall_at_k == 0.0
    assert report.combined.recall_at_k == 1.0
    assert report.combined.mean_reciprocal_rank == 1.0


def test_malformed_fixture_is_rejected() -> None:
    with pytest.raises(EvaluationError, match="duplicate"):
        BenchmarkCase(
            case_id="duplicate-relevant",
            query="query",
            relevant_document_ids=("doc", "doc"),
            canonical_results=("doc",),
            combined_results=("doc",),
        )


def test_invalid_gate_arguments_are_rejected() -> None:
    case = BenchmarkCase(
        case_id="valid",
        query="query",
        relevant_document_ids=("doc",),
        canonical_results=("doc",),
        combined_results=("doc",),
    )

    with pytest.raises(EvaluationError, match="top_k"):
        evaluate_comparative((case,), top_k=0)
    with pytest.raises(EvaluationError, match="minimum_mrr_gain"):
        evaluate_comparative((case,), minimum_mrr_gain=-0.1)
    with pytest.raises(EvaluationError, match="benchmark cases"):
        evaluate_comparative(())
