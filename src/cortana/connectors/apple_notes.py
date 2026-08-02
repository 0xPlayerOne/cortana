from __future__ import annotations

import datetime as dt
import json
import subprocess
from collections.abc import Iterable
from typing import Any

from .model import Document

SCRIPT = r"""
const Notes = Application("Notes");
const rows = [];
for (const account of Notes.accounts()) {
  for (const folder of account.folders()) {
    for (const note of folder.notes()) {
      rows.push({
        id: note.id(),
        name: note.name(),
        body: note.plaintext(),
        modified: note.modificationDate().toISOString(),
        account: account.name(),
        folder: folder.name()
      });
    }
  }
}
JSON.stringify(rows);
"""
MAX_EXPORT_BYTES = 64 * 1024 * 1024


def fetch(project: str = "personal", timeout: int = 120) -> Iterable[Document]:
    try:
        result = subprocess.run(
            ["osascript", "-l", "JavaScript", "-e", SCRIPT],
            check=True,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(
            "Apple Notes automation timed out; open Notes and grant Automation access to "
            "the invoking terminal or Cortana service"
        ) from error
    if len(result.stdout.encode("utf-8")) > MAX_EXPORT_BYTES:
        raise RuntimeError(
            f"Apple Notes export exceeds the {MAX_EXPORT_BYTES} byte safety limit; "
            "narrow the source before retrying"
        )
    try:
        rows: Any = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("Apple Notes returned malformed JSON") from error
    if not isinstance(rows, list):
        raise RuntimeError("Apple Notes returned an invalid export shape")
    for row in rows:
        if not isinstance(row, dict):
            continue
        content = str(row.get("body") or "").strip()
        if not content:
            continue
        modified = dt.datetime.fromisoformat(str(row["modified"]).replace("Z", "+00:00"))
        yield Document(
            source="apple-notes",
            source_id=str(row["id"]),
            title=str(row.get("name") or "Untitled note"),
            content=content,
            uri=f"notes://showNote?identifier={row['id']}",
            updated_at=modified,
            project=project,
            metadata={"account": row.get("account"), "folder": row.get("folder")},
        )
