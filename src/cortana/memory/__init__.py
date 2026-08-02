"""Optional derived-memory sidecar integration primitives.

This package intentionally remains opt-in and disconnected from the core ingestion
pipeline until the Hindsight evaluation gates are passed.
"""

from .hindsight import HindsightConfig, HindsightHttpProvider
from .models import (
    DocumentEnvelope,
    MemoryArgumentError,
    MemoryDocument,
    MemoryError,
    MemoryOperation,
    stable_document_id,
    workspace_acl_tags,
)
from .outbox import Outbox, OutboxEntry
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
    "MemoryProvider",
    "ProviderError",
    "HindsightConfig",
    "HindsightHttpProvider",
    "MemorySyncWorker",
    "stable_document_id",
    "workspace_acl_tags",
]
