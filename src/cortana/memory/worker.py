from __future__ import annotations

from .models import MemoryArgumentError, MemoryDocument, MemoryOperation
from .outbox import Outbox, OutboxEntry
from .provider import MemoryProvider, ProviderError


class MemorySyncWorker:
    """Drain the outbox and sync memory operations.

    Workers process only due rows and retry transient failures.
    """

    def __init__(
        self,
        *,
        outbox: Outbox,
        provider: MemoryProvider,
        worker_id: str = "worker",
    ) -> None:
        self._outbox = outbox
        self._provider = provider
        self._worker_id = worker_id

    def run(self, *, limit: int = 64, lease_seconds: float = 60.0) -> int:
        processed = 0
        while True:
            entries = self._outbox.claim_due(
                limit=limit, lease_seconds=lease_seconds, worker=self._worker_id
            )
            if not entries:
                break
            for entry in entries:
                self._apply(entry)
                processed += 1
        return processed

    def _apply(self, entry: OutboxEntry) -> None:
        try:
            if entry.operation == MemoryOperation.RETAIN:
                document = MemoryDocument(
                    project=entry.project,
                    source=entry.source,
                    source_id=entry.source_id,
                    title=entry.title,
                    content=entry.content,
                    context=entry.context if isinstance(entry.context, str) else None,
                    metadata=entry.metadata,
                    acl=tuple(
                        tag.removeprefix("acl:") for tag in entry.tags if tag.startswith("acl:")
                    ),
                )
                self._provider.retain(document)
            elif entry.operation == MemoryOperation.DELETE:
                self._provider.delete(entry.document_id)
            else:
                raise ValueError(f"unsupported operation {entry.operation}")
            if entry.leased_by is not None:
                self._outbox.acknowledge(entry.id, lease_owner=entry.leased_by)
        except ProviderError as error:
            if entry.leased_by is not None:
                self._outbox.mark_failed(
                    entry.id,
                    lease_owner=entry.leased_by,
                    error=str(error),
                    retriable=error.retriable,
                )
        except MemoryArgumentError as error:
            if entry.leased_by is not None:
                self._outbox.mark_failed(
                    entry.id, lease_owner=entry.leased_by, error=str(error), retriable=False
                )
        except Exception as error:
            if entry.leased_by is not None:
                self._outbox.mark_failed(
                    entry.id,
                    lease_owner=entry.leased_by,
                    error=f"{type(error).__name__}: {error}",
                    retriable=False,
                )
