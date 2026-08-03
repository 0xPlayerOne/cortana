import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1]))

from scripts.benchmark_query import percentile, summarize  # noqa: E402


def test_percentile_uses_nearest_rank_without_interpolation() -> None:
    assert percentile([7, 1, 4, 9], 0.5) == 4
    assert percentile([7, 1, 4, 9], 0.95) == 9


def test_summary_marks_failed_or_missing_iterations() -> None:
    summary = summarize(
        [{"passed": True, "latency_ms": 10}, {"passed": False, "latency_ms": 20}],
        iterations=3,
        concurrency=2,
    )
    assert summary["isolated"] is True
    assert summary["passed"] is False
    assert summary["latency_ms"] == {"min": 10, "p50": 10, "p95": 20, "max": 20}
