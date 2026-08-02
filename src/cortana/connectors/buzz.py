from __future__ import annotations

import datetime as dt
import json
import sqlite3
import stat
from collections.abc import Iterable
from pathlib import Path

from .model import Document


def fetch(root: Path, project: str = "buzz") -> Iterable[Document]:
    database = root / "agents" / "retention.db"
    if _is_regular_non_symlink(database):
        uri = f"file:{database}?mode=ro"
        with sqlite3.connect(uri, uri=True) as connection:
            for kind, pubkey, tag, content, created_at, raw_event in connection.execute(
                "SELECT kind,pubkey,d_tag,content,created_at,raw_event FROM persona_events"
            ):
                yield Document(
                    source="buzz",
                    source_id=f"persona:{kind}:{pubkey}:{tag}",
                    title=f"Buzz persona {tag}",
                    content=content,
                    uri=f"buzz://persona/{pubkey}/{tag}",
                    updated_at=dt.datetime.fromtimestamp(created_at, dt.UTC),
                    project=project,
                    metadata={"kind": kind, "pubkey": pubkey, "raw_event": json.loads(raw_event)},
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
    return stat.S_ISREG(metadata.st_mode)
