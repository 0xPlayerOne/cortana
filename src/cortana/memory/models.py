from __future__ import annotations

import json
import re as _re
from collections.abc import Iterable
from dataclasses import dataclass, field
from hashlib import sha256
from typing import Any

_TAG_VALUE_RE = _re.compile(r"^[A-Za-z0-9._-]{1,64}$")
_SOURCE_ID_RE = _re.compile(r"^[^\s]+$")


class MemoryError(ValueError):
    """Base error for memory adapter data validation."""


class MemoryArgumentError(MemoryError):
    """Configuration or parameter validation failed."""


def _normalize_tag(value: str) -> str:
    normalized = value.strip().lower()
    if not normalized:
        raise MemoryError("memory tags cannot be empty")
    if not _TAG_VALUE_RE.fullmatch(normalized):
        raise MemoryError(f"memory tag `{value}` is malformed")
    return normalized


def _normalize_source_id(value: str) -> str:
    if not value:
        raise MemoryArgumentError("memory source_id cannot be empty")
    normalized = value.strip()
    if not normalized:
        raise MemoryArgumentError("memory source_id cannot be empty")
    if not _SOURCE_ID_RE.fullmatch(normalized):
        raise MemoryArgumentError(f"memory source_id `{value}` is malformed")
    return normalized


def workspace_acl_tags(project: str, acl: Iterable[str]) -> list[str]:
    """Map workspace/project and ACL labels to strict, canonical Hindsight tags."""

    mapped: list[str] = [f"workspace:{_normalize_tag(project)}"]
    for item in acl:
        mapped.append(f"acl:{_normalize_tag(item)}")
    return sorted(mapped)


def stable_document_id(project: str, source: str, source_id: str) -> str:
    """Deterministically derive a stable external document identifier."""

    digest_input = "|".join(
        [_normalize_tag(project), _normalize_tag(source), _normalize_source_id(source_id)]
    )
    return sha256(digest_input.encode("utf-8")).hexdigest()


@dataclass(frozen=True)
class MemoryDocument:
    """Canonical representation of a derived memory candidate."""

    project: str
    source: str
    source_id: str
    title: str
    content: str
    context: str | None = None
    metadata: dict[str, Any] = field(default_factory=dict)
    acl: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        self.validate()

    def validate(self) -> None:
        project = _normalize_tag(self.project)
        source = _normalize_tag(self.source)
        source_id = _normalize_source_id(self.source_id)
        if not self.title.strip():
            raise MemoryError("memory document title cannot be empty")
        if not self.content.strip():
            raise MemoryError("memory document content cannot be empty")
        workspace_acl_tags(project, self.acl)
        object.__setattr__(self, "project", project)
        object.__setattr__(self, "source", source)
        object.__setattr__(self, "source_id", source_id)

    @property
    def document_id(self) -> str:
        return stable_document_id(self.project, self.source, self.source_id)

    @property
    def tags(self) -> list[str]:
        return workspace_acl_tags(self.project, self.acl)

    def retention_payload(self) -> dict[str, Any]:
        """Payload fields required by derived-memory retain APIs."""

        payload: dict[str, Any] = {
            "document_id": self.document_id,
            "content": self.content,
            "metadata": self.metadata,
            "tags": self.tags,
        }
        if self.context is not None and self.context.strip():
            payload["context"] = self.context.strip()
        return payload


@dataclass(frozen=True)
class DocumentEnvelope:
    """Queue envelope around a canonical document with operation metadata."""

    document: MemoryDocument
    operation: MemoryOperation
    max_attempts: int = 8


class MemoryOperation:
    RETAIN = "retain"
    DELETE = "delete"


def serialize_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))
