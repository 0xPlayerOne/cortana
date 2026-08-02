from __future__ import annotations

import json
import sqlite3
import time
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .models import (
    MemoryArgumentError,
    MemoryDocument,
    MemoryOperation,
    serialize_json,
    stable_document_id,
    workspace_acl_tags,
)


class OutboxError(RuntimeError):
    """Raised for operational errors from the durable outbox."""


@dataclass(frozen=True)
class OutboxEntry:
    id: int
    operation: str
    document_id: str
    project: str
    source: str
    source_id: str
    title: str
    content: str
    context: str | None
    metadata: dict[str, Any]
    tags: list[str]
    state: str
    attempts: int
    max_attempts: int
    available_after: float
    lease_until: float | None
    leased_by: str | None
    updated_at: float


class Outbox:
    """Durable SQLite outbox for optional memory synchronization."""

    MAX_ATTEMPTS = 8
    SCHEMA_VERSION = 1

    def __init__(self, path: Path) -> None:
        self._connection = sqlite3.connect(path)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA journal_mode=WAL")
        self._connection.execute("PRAGMA foreign_keys=ON")
        self._connection.execute("PRAGMA synchronous=NORMAL")
        self._ensure_schema()

    def close(self) -> None:
        self._connection.close()

    def __enter__(self) -> Outbox:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def _ensure_schema(self) -> None:
        self._connection.execute(
            "CREATE TABLE IF NOT EXISTS outbox_schema (version INTEGER PRIMARY KEY)"
        )
        row = self._connection.execute(
            "SELECT version FROM outbox_schema ORDER BY version DESC LIMIT 1"
        ).fetchone()
        if row is None:
            self._bootstrap_schema()
            return
        if int(row[0]) != self.SCHEMA_VERSION:
            raise OutboxError("incompatible outbox schema version")

    def _bootstrap_schema(self) -> None:
        self._connection.execute(
            """
            CREATE TABLE memory_outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                operation TEXT NOT NULL CHECK(operation IN ('retain', 'delete')),
                project TEXT NOT NULL,
                source TEXT NOT NULL,
                source_id TEXT NOT NULL,
                document_id TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                context_json TEXT NOT NULL,
                metadata_json TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('pending','in_flight','succeeded','dead_letter')),
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 8,
                available_after REAL NOT NULL DEFAULT 0,
                lease_until REAL,
                leased_by TEXT,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL,
                last_error TEXT,
                UNIQUE(operation, document_id)
            )
            """
        )
        self._connection.execute(
            "CREATE INDEX IF NOT EXISTS memory_outbox_due ON memory_outbox(state, available_after, id)"
        )
        self._connection.execute(
            "INSERT INTO outbox_schema(version) VALUES (?)", (self.SCHEMA_VERSION,)
        )
        self._connection.commit()

    def enqueue_retain(self, document: MemoryDocument, *, max_attempts: int | None = None) -> int:
        return self._upsert_entry(
            operation=MemoryOperation.RETAIN,
            project=document.project,
            source=document.source,
            source_id=document.source_id,
            document_id=document.document_id,
            title=document.title,
            content=document.content,
            context=document.context,
            metadata=document.metadata,
            acl=document.acl,
            max_attempts=max_attempts,
        )

    def enqueue_delete(
        self,
        *,
        project: str,
        source: str,
        source_id: str,
        acl: tuple[str, ...] = (),
        max_attempts: int | None = None,
    ) -> int:
        document_id = stable_document_id(project, source, source_id)
        return self._upsert_entry(
            operation=MemoryOperation.DELETE,
            project=project,
            source=source,
            source_id=source_id,
            document_id=document_id,
            title="delete",
            content="",
            context=None,
            metadata={"deleted": True},
            acl=acl,
            max_attempts=max_attempts,
        )

    def _upsert_entry(
        self,
        *,
        operation: str,
        project: str,
        source: str,
        source_id: str,
        document_id: str,
        title: str,
        content: str,
        context: str | None,
        metadata: dict[str, Any],
        acl: tuple[str, ...],
        max_attempts: int | None,
    ) -> int:
        if operation not in (MemoryOperation.RETAIN, MemoryOperation.DELETE):
            raise OutboxError(f"unsupported operation: {operation}")
        attempts = self.MAX_ATTEMPTS if max_attempts is None else max_attempts
        if not isinstance(attempts, int) or attempts <= 0:
            raise MemoryArgumentError("max_attempts must be a positive integer")

        if operation == MemoryOperation.RETAIN:
            # Enforce full envelope validation through canonical record.
            MemoryDocument(
                project=project,
                source=source,
                source_id=source_id,
                title=title,
                content=content,
                context=context,
                metadata=metadata,
                acl=acl,
            )

        now = time.time()
        tags = workspace_acl_tags(project, acl)
        self._connection.execute(
            """
            INSERT INTO memory_outbox (
                operation, project, source, source_id, document_id, title, content,
                context_json, metadata_json, tags_json, state, attempts, max_attempts,
                available_after, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?, ?)
            ON CONFLICT(operation, document_id)
            DO UPDATE SET
                project=excluded.project,
                source=excluded.source,
                source_id=excluded.source_id,
                title=excluded.title,
                content=excluded.content,
                context_json=excluded.context_json,
                metadata_json=excluded.metadata_json,
                tags_json=excluded.tags_json,
                state='pending',
                attempts=0,
                max_attempts=excluded.max_attempts,
                available_after=excluded.available_after,
                lease_until=NULL,
                leased_by=NULL,
                updated_at=excluded.updated_at,
                last_error=NULL
            WHERE operation=excluded.operation AND document_id=excluded.document_id
            """,
            (
                operation,
                project,
                source,
                source_id,
                document_id,
                title,
                content,
                serialize_json(context),
                serialize_json(metadata),
                serialize_json(tags),
                attempts,
                now,
                now,
                now,
            ),
        )
        self._connection.commit()
        row = self._connection.execute(
            "SELECT id FROM memory_outbox WHERE operation = ? AND document_id = ?",
            (operation, document_id),
        ).fetchone()
        if row is None:
            raise OutboxError("upsert failed")
        return int(row[0])

    def claim_due(
        self,
        limit: int = 10,
        *,
        lease_seconds: float = 60.0,
        worker: str = "worker",
    ) -> list[OutboxEntry]:
        if limit <= 0:
            raise MemoryArgumentError("limit must be a positive integer")
        if not isinstance(lease_seconds, (int, float)) or lease_seconds <= 0:
            raise MemoryArgumentError("lease_seconds must be a positive number")

        now = time.time()
        self._release_expired_leases(now)
        rows = self._connection.execute(
            """
            SELECT id
              FROM memory_outbox
             WHERE state='pending' AND available_after <= ?
             ORDER BY available_after ASC, id ASC
             LIMIT ?
            """,
            (now, limit),
        ).fetchall()
        if not rows:
            return []

        claimed_ids: list[int] = []
        lease_until = now + float(lease_seconds)
        for row in rows:
            row_id = int(row[0])
            cursor = self._connection.execute(
                """
                UPDATE memory_outbox
                   SET state='in_flight', lease_until=?, leased_by=?, updated_at=?
                 WHERE id = ? AND state='pending' AND available_after <= ?
                """,
                (lease_until, worker, now, row_id, now),
            )
            if cursor.rowcount == 1:
                claimed_ids.append(row_id)
        self._connection.commit()
        return [self._load_entry(entry_id) for entry_id in claimed_ids]

    def acknowledge(self, entry_id: int) -> None:
        self._connection.execute(
            """
            UPDATE memory_outbox
               SET state='succeeded',
                   lease_until=NULL,
                   leased_by=NULL,
                   updated_at=?,
                   last_error=NULL
             WHERE id = ?
            """,
            (time.time(), entry_id),
        )
        self._connection.commit()

    def mark_failed(self, entry_id: int, *, error: str, retriable: bool) -> None:
        row = self._connection.execute(
            "SELECT attempts, max_attempts FROM memory_outbox WHERE id = ?",
            (entry_id,),
        ).fetchone()
        if row is None:
            return

        attempts = int(row[0]) + 1
        max_attempts = int(row[1])
        now = time.time()

        if not retriable or attempts >= max_attempts:
            self._connection.execute(
                """
                UPDATE memory_outbox
                   SET state='dead_letter',
                       attempts=?,
                       lease_until=NULL,
                       leased_by=NULL,
                       updated_at=?,
                       available_after=?,
                       last_error=?
                 WHERE id = ?
                """,
                (attempts, now, now, error, entry_id),
            )
        else:
            delay = min(30.0, 2.0**attempts)
            self._connection.execute(
                """
                UPDATE memory_outbox
                   SET state='pending',
                       attempts=?,
                       available_after=?,
                       lease_until=NULL,
                       leased_by=NULL,
                       updated_at=?,
                       last_error=?
                 WHERE id = ?
                """,
                (attempts, now + delay, now, error, entry_id),
            )
        self._connection.commit()

    def export_rows(
        self,
        states: Sequence[str] | None = None,
        *,
        limit: int = 100,
    ) -> list[OutboxEntry]:
        if limit <= 0:
            raise MemoryArgumentError("limit must be a positive integer")
        query = "SELECT * FROM memory_outbox"
        params: list[Any] = []
        if states:
            placeholders = ",".join(["?" for _ in states])
            query += f" WHERE state IN ({placeholders})"
            params.extend(states)
        query += " ORDER BY id ASC LIMIT ?"
        params.append(limit)
        rows = self._connection.execute(query, params).fetchall()
        return [self._to_entry(row) for row in rows]

    def set_available(
        self, *, document_id: str, operation: str, available_after: float = 0.0
    ) -> None:
        """Public helper useful for deterministic retry/transition tests."""

        if not isinstance(available_after, (int, float)):
            raise MemoryArgumentError("available_after must be a numeric value")
        if operation not in (MemoryOperation.RETAIN, MemoryOperation.DELETE):
            raise MemoryArgumentError(f"unsupported operation: {operation}")
        self._connection.execute(
            "UPDATE memory_outbox SET available_after = ?, state='pending', lease_until=NULL, leased_by=NULL WHERE operation=? AND document_id=?",
            (available_after, operation, document_id),
        )
        self._connection.commit()

    def get_entry(
        self,
        *,
        document_id: str,
        operation: str,
    ) -> OutboxEntry | None:
        if operation not in (MemoryOperation.RETAIN, MemoryOperation.DELETE):
            raise MemoryArgumentError(f"unsupported operation: {operation}")
        row = self._connection.execute(
            "SELECT * FROM memory_outbox WHERE operation = ? AND document_id = ?",
            (operation, document_id),
        ).fetchone()
        if row is None:
            return None
        return self._to_entry(row)

    def stats(self) -> dict[str, int]:
        rows = self._connection.execute(
            "SELECT state, COUNT(*) as total FROM memory_outbox GROUP BY state"
        ).fetchall()
        result: dict[str, int] = {"pending": 0, "in_flight": 0, "succeeded": 0, "dead_letter": 0}
        for row in rows:
            result[str(row[0])] = int(row[1])
        return result

    def _release_expired_leases(self, now: float) -> None:
        self._connection.execute(
            """
            UPDATE memory_outbox
               SET state='pending',
                   lease_until=NULL,
                   leased_by=NULL,
                   available_after=?
             WHERE state='in_flight' AND lease_until IS NOT NULL AND lease_until <= ?
            """,
            (now, now),
        )
        self._connection.commit()

    def _load_entry(self, entry_id: int) -> OutboxEntry:
        row = self._connection.execute(
            "SELECT * FROM memory_outbox WHERE id = ?",
            (entry_id,),
        ).fetchone()
        if row is None:
            raise OutboxError(f"entry {entry_id} not found")
        return self._to_entry(row)

    def _to_entry(self, row: sqlite3.Row) -> OutboxEntry:
        return OutboxEntry(
            id=int(row["id"]),
            operation=str(row["operation"]),
            document_id=str(row["document_id"]),
            project=str(row["project"]),
            source=str(row["source"]),
            source_id=str(row["source_id"]),
            title=str(row["title"]),
            content=str(row["content"]),
            context=json.loads(row["context_json"]),
            metadata=json.loads(row["metadata_json"]),
            tags=json.loads(row["tags_json"]),
            state=str(row["state"]),
            attempts=int(row["attempts"]),
            max_attempts=int(row["max_attempts"]),
            available_after=float(row["available_after"]),
            lease_until=None if row["lease_until"] is None else float(row["lease_until"]),
            leased_by=None if row["leased_by"] is None else str(row["leased_by"]),
            updated_at=float(row["updated_at"]),
        )
