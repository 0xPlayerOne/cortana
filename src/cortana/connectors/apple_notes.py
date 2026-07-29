from __future__ import annotations

import datetime as dt
import json
import subprocess
from collections.abc import Iterable

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


def fetch(project: str = "personal", timeout: int = 120) -> Iterable[Document]:
    result = subprocess.run(
        ["osascript", "-l", "JavaScript", "-e", SCRIPT],
        check=True,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    for row in json.loads(result.stdout):
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
