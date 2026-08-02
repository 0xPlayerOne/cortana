from __future__ import annotations

from typing import Protocol

from .models import MemoryDocument


class ProviderError(Exception):
    """Raised when a memory provider request fails."""

    def __init__(self, message: str, *, retriable: bool = False) -> None:
        super().__init__(message)
        self.retriable = retriable


class MemoryProvider(Protocol):
    """Small abstraction for optional derived-memory providers."""

    def retain(self, document: MemoryDocument) -> None: ...

    def delete(self, document_id: str) -> None: ...

    @property
    def configured(self) -> bool: ...


__all__ = ["MemoryProvider", "ProviderError"]
