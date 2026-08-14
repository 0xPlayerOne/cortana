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
const includeFolders = [];
const excludeFolders = [];
outer:
for (const account of Notes.accounts()) {
  if (maxDocuments !== undefined && rows.length >= maxDocuments) {
    break outer;
  }
  for (const folder of account.folders()) {
    const folderName = folder.name();
    const included = includeFolders.length === 0 || includeFolders.indexOf(folderName) !== -1;
    const excluded = excludeFolders.indexOf(folderName) !== -1;
    if (!included || excluded) {
      continue;
    }
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
        folder: folderName
      });
    }
  }
}
JSON.stringify(rows);
"""
MAX_EXPORT_BYTES = 64 * 1024 * 1024
OSASCRIPT = "/usr/bin/osascript"


def _build_script(
    max_documents: int | None,
    folders: Iterable[str] | None = None,
    exclude_folders: Iterable[str] | None = None,
) -> str:
    script = SCRIPT
    if max_documents is not None:
        script = script.replace(
            "const maxDocuments = undefined;", f"const maxDocuments = {max_documents};"
        )
    script = script.replace(
        "const includeFolders = [];",
        f"const includeFolders = {json.dumps(list(folders or []), ensure_ascii=False)};",
    )
    return script.replace(
        "const excludeFolders = [];",
        f"const excludeFolders = {json.dumps(list(exclude_folders or []), ensure_ascii=False)};",
    )


def fetch(
    project: str = "personal",
    timeout: int = 120,
    max_documents: int | None = None,
    folders: Iterable[str] | None = None,
    exclude_folders: Iterable[str] | None = None,
) -> Iterable[Document]:
    try:
        result = subprocess.run(  # noqa: S603 - fixed system executable; no shell
            [
                OSASCRIPT,
                "-l",
                "JavaScript",
                "-e",
                _build_script(max_documents, folders, exclude_folders),
            ],
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
