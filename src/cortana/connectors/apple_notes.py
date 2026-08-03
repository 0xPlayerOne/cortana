from __future__ import annotations

import datetime as dt
import json
import subprocess
import sys
from collections.abc import Iterable
from typing import Any
from urllib.parse import quote

from .model import Document

SCRIPT = r"""
const Notes = Application("Notes");
const rows = [];
const maxDocuments = undefined;
outer:
for (const account of Notes.accounts()) {
  if (maxDocuments !== undefined && rows.length >= maxDocuments) {
    break outer;
  }
  for (const folder of account.folders()) {
    if (maxDocuments !== undefined && rows.length >= maxDocuments) {
      break outer;
    }
    for (const note of folder.notes()) {
      if (maxDocuments !== undefined && rows.length >= maxDocuments) {
        break outer;
      }
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


def _build_script(max_documents: int | None) -> str:
    if max_documents is None:
        return SCRIPT
    return SCRIPT.replace(
        "const maxDocuments = undefined;", f"const maxDocuments = {max_documents};"
    )


def fetch(
    project: str = "personal",
    timeout: int = 120,
    max_documents: int | None = None,
) -> Iterable[Document]:
    try:
        result = subprocess.run(
            ["osascript", "-l", "JavaScript", "-e", _build_script(max_documents)],
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
    emitted = 0
    for row in rows:
        if not isinstance(row, dict):
            continue
        content = str(row.get("body") or "").strip()
        if not content:
            continue
        source_id = str(row.get("id") or "").strip()
        modified = _parse_modified(row.get("modified"))
        if not source_id or modified is None:
            print(
                "connector warning: skipping Apple Note with missing identity or timestamp",
                file=sys.stderr,
            )
            continue
        yield Document(
            source="apple-notes",
            source_id=source_id,
            title=str(row.get("name") or "Untitled note"),
            content=content,
            uri=f"notes://showNote?identifier={quote(source_id, safe='')}",
            updated_at=modified,
            project=project,
            metadata={"account": row.get("account"), "folder": row.get("folder")},
        )
        emitted += 1
        if max_documents is not None and emitted >= max_documents:
            return


def _parse_modified(value: object) -> dt.datetime | None:
    if not isinstance(value, str) or not value.strip():
        return None
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=dt.UTC)
    return parsed
