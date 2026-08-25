#!/usr/bin/env python3
"""Compare bounded generic and structure-aware chunk statistics for fixtures.

This is intentionally offline and content-only. It is a review hook for an
approved fixture, not an ingestion command and never writes to Cortana's index.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections.abc import Iterable
from pathlib import Path

TARGET = 1_600
OVERLAP = 200


def bounded(content: str) -> list[str]:
    result: list[str] = []
    start = 0
    while start < len(content):
        end = min(len(content), start + TARGET)
        if end < len(content):
            boundary = content.rfind("\n\n", start + TARGET // 2, end)
            if boundary >= 0:
                end = boundary + 2
        chunk = content[start:end].strip()
        if chunk:
            result.append(chunk)
        if end == len(content):
            break
        start = max(end - OVERLAP, start + 1)
    return result


def strategy(record: dict[str, object]) -> str:
    metadata = record.get("metadata")
    metadata = metadata if isinstance(metadata, dict) else {}
    kind = " ".join(
        str(metadata.get(key, "")).lower()
        for key in ("mime_type", "mimeType", "content_type", "format", "extension")
    )
    if "markdown" in kind or kind.strip() in {"md", ".md"}:
        return "markdown_section"
    if "html" in kind or kind.strip() in {"htm", ".htm", "html", ".html"}:
        return "html_section"
    if any(key in metadata for key in ("thread_id", "message_id", "channel_id")):
        return "message_thread"
    if any(key in metadata for key in ("event_id", "start", "start_time")):
        return "calendar_event"
    return "generic"


def records(path: Path) -> Iterable[dict[str, object]]:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            value = json.loads(line)
            if not isinstance(value, dict):
                raise ValueError("fixture records must be JSON objects")
            yield value


def digest(values: Iterable[str]) -> str:
    return hashlib.sha256("\n".join(values).encode()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path, help="offline Document JSONL fixture")
    args = parser.parse_args()
    report: list[dict[str, object]] = []
    for record in records(args.fixture):
        content = str(record.get("content", ""))
        chunks = bounded(content)
        report.append(
            {
                "source": str(record.get("source", "")),
                "source_id_digest": digest([str(record.get("source_id", ""))])[:16],
                "strategy": strategy(record),
                "source_bytes": len(content.encode()),
                "generic_chunks": len(chunks),
                "generic_derived_bytes": sum(len(chunk.encode()) for chunk in chunks),
                "generic_chunk_digest": digest(chunks),
            }
        )
    print(json.dumps({"contract": "cortana.chunking.v1", "records": report}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
