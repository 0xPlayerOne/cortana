"""Optional derived-memory sidecar integration primitives.

This package intentionally remains opt-in and disconnected from the core ingestion
pipeline until the derived-memory evaluation gates are passed.
"""

from .cli import entrypoint as memory_sync_entrypoint
from .evaluation import (
    BENCHMARK_VERSION,
    BenchmarkCase,
    CaseMetrics,
    ComparativeEvaluation,
    EvaluationError,
    RankingMetrics,
    default_benchmark_cases,
    evaluate_comparative,
    run_default_benchmark,
)
from .hindsight import HindsightConfig, HindsightHttpProvider
from .honcho import HonchoConfig, HonchoHttpProvider
from .models import (
    DocumentEnvelope,
    MemoryArgumentError,
    MemoryDocument,
    MemoryError,
    MemoryOperation,
    stable_document_id,
    workspace_acl_tags,
)
from .outbox import Outbox, OutboxEntry, OutboxError
from .provider import MemoryProvider, ProviderError
from .worker import MemorySyncWorker

__all__ = [
    "DocumentEnvelope",
    "MemoryArgumentError",
    "MemoryDocument",
    "MemoryError",
    "MemoryOperation",
    "Outbox",
    "OutboxEntry",
    "OutboxError",
    "MemoryProvider",
    "ProviderError",
    "HindsightConfig",
    "HindsightHttpProvider",
    "HonchoConfig",
    "HonchoHttpProvider",
    "BENCHMARK_VERSION",
    "BenchmarkCase",
    "CaseMetrics",
    "ComparativeEvaluation",
    "EvaluationError",
    "RankingMetrics",
    "default_benchmark_cases",
    "evaluate_comparative",
    "run_default_benchmark",
    "MemorySyncWorker",
    "memory_sync_entrypoint",
    "stable_document_id",
    "workspace_acl_tags",
]
