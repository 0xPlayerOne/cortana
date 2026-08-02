"""Deterministic, offline evaluation for optional derived memory.

The benchmark deliberately accepts captured ranked document IDs rather than
calling a provider. This keeps evaluation reproducible, prevents personal
data from entering CI, and lets the canonical Cortana path remain complete if
Hindsight is disabled or unavailable.
"""

from __future__ import annotations

import json
import math
from collections.abc import Sequence
from dataclasses import asdict, dataclass

BENCHMARK_VERSION = "v1"
DEFAULT_TOP_K = 5
DEFAULT_MIN_RECALL_GAIN = 0.1
DEFAULT_MIN_MRR_GAIN = 0.1


class EvaluationError(ValueError):
    """Raised when a benchmark fixture or gate is malformed."""


def _text(value: str, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise EvaluationError(f"{label} must be a non-empty string")
    return value.strip()


def _ids(values: Sequence[str], label: str) -> tuple[str, ...]:
    normalized: list[str] = []
    seen: set[str] = set()
    for value in values:
        item = _text(value, label)
        if item in seen:
            raise EvaluationError(f"{label} contains duplicate document ID `{item}`")
        seen.add(item)
        normalized.append(item)
    return tuple(normalized)


@dataclass(frozen=True, slots=True)
class BenchmarkCase:
    """One query with relevance labels and two captured result rankings."""

    case_id: str
    query: str
    relevant_document_ids: tuple[str, ...]
    canonical_results: tuple[str, ...]
    combined_results: tuple[str, ...]

    def __post_init__(self) -> None:
        object.__setattr__(self, "case_id", _text(self.case_id, "case_id"))
        object.__setattr__(self, "query", _text(self.query, "query"))
        relevant = _ids(self.relevant_document_ids, "relevant_document_ids")
        canonical = _ids(self.canonical_results, "canonical_results")
        combined = _ids(self.combined_results, "combined_results")
        if not relevant:
            raise EvaluationError("relevant_document_ids cannot be empty")
        object.__setattr__(self, "relevant_document_ids", relevant)
        object.__setattr__(self, "canonical_results", canonical)
        object.__setattr__(self, "combined_results", combined)


@dataclass(frozen=True, slots=True)
class CaseMetrics:
    """Metrics for one ranking at the configured cutoff."""

    case_id: str
    hit_at_k: bool
    recall_at_k: float
    reciprocal_rank: float


@dataclass(frozen=True, slots=True)
class RankingMetrics:
    """Aggregate ranking quality across all benchmark cases."""

    case_count: int
    hits_at_k: int
    recall_at_k: float
    mean_reciprocal_rank: float
    cases: tuple[CaseMetrics, ...]


@dataclass(frozen=True, slots=True)
class ComparativeEvaluation:
    """Cortana-only versus combined-memory results and the enablement gate."""

    version: str
    top_k: int
    minimum_recall_gain: float
    minimum_mrr_gain: float
    canonical: RankingMetrics
    combined: RankingMetrics
    recall_gain: float
    mrr_gain: float
    material_gain: bool

    def as_dict(self) -> dict[str, object]:
        """Return stable JSON-compatible output for audit artifacts."""

        return asdict(self)

    def as_json(self) -> str:
        """Serialize the report without nondeterministic whitespace or keys."""

        return json.dumps(self.as_dict(), sort_keys=True, separators=(",", ":"))


def _validate_threshold(value: float, label: str) -> float:
    if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise EvaluationError(f"{label} must be a finite non-negative number")
    return float(value)


def _score(
    cases: Sequence[BenchmarkCase],
    rankings: Sequence[Sequence[str]],
    top_k: int,
) -> RankingMetrics:
    metrics: list[CaseMetrics] = []
    for case, ranking in zip(cases, rankings, strict=True):
        relevant = set(case.relevant_document_ids)
        top = tuple(ranking[:top_k])
        hits = [index for index, document_id in enumerate(top) if document_id in relevant]
        hit_at_k = bool(hits)
        recall = len({document_id for document_id in top if document_id in relevant}) / len(
            relevant
        )
        reciprocal_rank = 1.0 / (hits[0] + 1) if hits else 0.0
        metrics.append(
            CaseMetrics(
                case_id=case.case_id,
                hit_at_k=hit_at_k,
                recall_at_k=recall,
                reciprocal_rank=reciprocal_rank,
            )
        )

    count = len(metrics)
    if count == 0:
        raise EvaluationError("benchmark cases cannot be empty")
    return RankingMetrics(
        case_count=count,
        hits_at_k=sum(metric.hit_at_k for metric in metrics),
        recall_at_k=sum(metric.recall_at_k for metric in metrics) / count,
        mean_reciprocal_rank=sum(metric.reciprocal_rank for metric in metrics) / count,
        cases=tuple(metrics),
    )


def evaluate_comparative(
    cases: Sequence[BenchmarkCase],
    *,
    top_k: int = DEFAULT_TOP_K,
    minimum_recall_gain: float = DEFAULT_MIN_RECALL_GAIN,
    minimum_mrr_gain: float = DEFAULT_MIN_MRR_GAIN,
    version: str = BENCHMARK_VERSION,
) -> ComparativeEvaluation:
    """Evaluate captured rankings and decide whether derived memory is useful.

    The gate requires a material gain in both recall@k and mean reciprocal
    rank. A report never enables Hindsight itself; callers must explicitly
    review and opt in after this deterministic check passes.
    """

    if not isinstance(top_k, int) or isinstance(top_k, bool) or top_k <= 0:
        raise EvaluationError("top_k must be a positive integer")
    normalized_cases = tuple(cases)
    if not normalized_cases:
        raise EvaluationError("benchmark cases cannot be empty")
    case_ids = [case.case_id for case in normalized_cases]
    if len(set(case_ids)) != len(case_ids):
        raise EvaluationError("benchmark case IDs must be unique")
    normalized_version = _text(version, "version")
    recall_threshold = _validate_threshold(minimum_recall_gain, "minimum_recall_gain")
    mrr_threshold = _validate_threshold(minimum_mrr_gain, "minimum_mrr_gain")

    canonical = _score(
        normalized_cases,
        [case.canonical_results for case in normalized_cases],
        top_k,
    )
    combined = _score(
        normalized_cases,
        [case.combined_results for case in normalized_cases],
        top_k,
    )
    recall_gain = combined.recall_at_k - canonical.recall_at_k
    mrr_gain = combined.mean_reciprocal_rank - canonical.mean_reciprocal_rank
    return ComparativeEvaluation(
        version=normalized_version,
        top_k=top_k,
        minimum_recall_gain=recall_threshold,
        minimum_mrr_gain=mrr_threshold,
        canonical=canonical,
        combined=combined,
        recall_gain=recall_gain,
        mrr_gain=mrr_gain,
        material_gain=recall_gain >= recall_threshold and mrr_gain >= mrr_threshold,
    )


def default_benchmark_cases() -> tuple[BenchmarkCase, ...]:
    """Return the small versioned fixture used by the offline evaluation CLI."""

    return (
        BenchmarkCase(
            case_id="architecture-context",
            query="where is the desktop trust boundary documented",
            relevant_document_ids=("desktop-architecture", "security-model"),
            canonical_results=("desktop-architecture", "release-notes"),
            combined_results=("desktop-architecture", "security-model", "release-notes"),
        ),
        BenchmarkCase(
            case_id="recovery-procedure",
            query="how do I recover the index after a failed restore",
            relevant_document_ids=("backup-recovery",),
            canonical_results=("release-notes", "operations-overview"),
            combined_results=("backup-recovery", "operations-overview"),
        ),
        BenchmarkCase(
            case_id="embedding-cache",
            query="what controls query embedding cache reuse",
            relevant_document_ids=("embedding-cache",),
            canonical_results=("embedding-cache", "query-api"),
            combined_results=("embedding-cache", "query-api"),
        ),
        BenchmarkCase(
            case_id="source-authorization",
            query="which source authorization step is required before sync",
            relevant_document_ids=("source-authorization",),
            canonical_results=("operations-overview", "source-authorization"),
            combined_results=("source-authorization", "operations-overview"),
        ),
    )


def run_default_benchmark() -> ComparativeEvaluation:
    """Run the deterministic fixture without contacting Cortana or Hindsight."""

    return evaluate_comparative(default_benchmark_cases())


def main() -> None:
    """Print a stable JSON report for CI or a local production-readiness audit."""

    print(run_default_benchmark().as_json())


if __name__ == "__main__":
    main()


__all__ = [
    "BENCHMARK_VERSION",
    "BenchmarkCase",
    "CaseMetrics",
    "ComparativeEvaluation",
    "DEFAULT_MIN_MRR_GAIN",
    "DEFAULT_MIN_RECALL_GAIN",
    "DEFAULT_TOP_K",
    "EvaluationError",
    "RankingMetrics",
    "default_benchmark_cases",
    "evaluate_comparative",
    "run_default_benchmark",
]
