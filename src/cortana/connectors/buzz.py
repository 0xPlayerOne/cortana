from __future__ import annotations

import datetime as dt
import json
import sqlite3
import stat
import sys
from collections.abc import Iterable
from pathlib import Path
from urllib.parse import quote

from .model import Document


def fetch(
    root: Path, project: str = "buzz", max_documents: int | None = None
) -> Iterable[Document]:
    emitted = 0
    database = root / "agents" / "retention.db"
    if _is_regular_non_symlink(database):
        uri = f"file:{database}?mode=ro"
        with sqlite3.connect(uri, uri=True) as connection:
            for kind, pubkey, tag, content, created_at, raw_event in connection.execute(
                "SELECT kind,pubkey,d_tag,content,created_at,raw_event FROM persona_events"
            ):
                source_id = ":".join(str(value or "").strip() for value in (kind, pubkey, tag))
                text = str(content or "").strip()
                if not text or not all(str(value or "").strip() for value in (kind, pubkey, tag)):
                    print(
                        f"connector warning: skipping malformed Buzz persona {source_id or 'unknown'}",
                        file=sys.stderr,
                    )
                    continue
                try:
                    updated_at = dt.datetime.fromtimestamp(float(created_at), dt.UTC)
                except (OverflowError, TypeError, ValueError, OSError):
                    continue
                try:
                    event = json.loads(raw_event) if raw_event else None
                except (TypeError, json.JSONDecodeError):
                    event = None
                yield Document(
                    source="buzz",
                    source_id=f"persona:{source_id}",
                    title=f"Buzz persona {tag}",
                    content=text,
                    uri=f"buzz://persona/{quote(str(pubkey), safe='')}/{quote(str(tag), safe='')}",
                    updated_at=updated_at,
                    project=project,
                    metadata={"kind": kind, "pubkey": pubkey, "raw_event": event},
                )
    elif database.exists():
        raise RuntimeError(
            f"Buzz retention database must be a regular non-symlink file: {database}"
        )
    for path in sorted((root / "agents" / "logs").glob("*.log")):
        if path.is_symlink():
            raise RuntimeError(f"Buzz log must not be a symlink: {path}")
        if not path.is_file():
            continue
        content = path.read_text(encoding="utf-8", errors="replace")
        file_stat = path.stat()
        yield Document(
            source="buzz",
            source_id=f"log:{path.name}",
            title=f"Buzz agent log {path.stem[:12]}",
            content=content,
            uri=path.as_uri(),
            updated_at=dt.datetime.fromtimestamp(file_stat.st_mtime, dt.UTC),
            project=project,
            metadata={"kind": "agent-log", "bytes": file_stat.st_size},
        )


def _is_regular_non_symlink(path: Path) -> bool:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    if stat.S_ISLNK(metadata.st_mode):
        raise RuntimeError(f"Buzz retention database must be a regular non-symlink file: {path}")
    return stat.S_ISREG(metadata.st_mode)
