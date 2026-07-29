from __future__ import annotations

import dataclasses
import datetime as dt
import json
from collections.abc import Iterable
from typing import Any, TextIO


@dataclasses.dataclass(frozen=True)
class Document:
    source: str
    source_id: str
    title: str
    content: str
    uri: str | None = None
    updated_at: dt.datetime = dataclasses.field(default_factory=lambda: dt.datetime.now(dt.UTC))
    project: str = "default"
    acl: tuple[str, ...] = ()
    metadata: dict[str, Any] = dataclasses.field(default_factory=dict)

    def as_json(self) -> str:
        payload = dataclasses.asdict(self)
        payload["updated_at"] = self.updated_at.astimezone(dt.UTC).isoformat()
        return json.dumps(payload, ensure_ascii=False, separators=(",", ":"))


def emit(documents: Iterable[Document], output: TextIO) -> int:
    count = 0
    for document in documents:
        if not document.content.strip():
            continue
        output.write(document.as_json())
        output.write("\n")
        count += 1
    return count
