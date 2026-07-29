from __future__ import annotations

import base64
import datetime as dt
import email
import email.policy
import email.utils
import html
import io
import json
import os
import re
import sqlite3
import tempfile
from collections.abc import Iterable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any
from urllib.parse import quote

import httpx

from .model import Document

DRIVE_FIELDS = "nextPageToken,files(id,name,mimeType,modifiedTime,webViewLink,owners(displayName))"
GOOGLE_EXPORTS = {
    "application/vnd.google-apps.document": ("text/plain", "txt"),
    "application/vnd.google-apps.presentation": ("text/plain", "txt"),
    "application/vnd.google-apps.spreadsheet": ("text/csv", "csv"),
}
TEXT_MIME_TYPES = {
    "application/json",
    "application/rtf",
    "application/xml",
    "text/csv",
    "text/html",
    "text/markdown",
    "text/plain",
}


class GoogleSession:
    """Small OAuth REST client compatible with Google token JSON files."""

    def __init__(self, token_path: Path, client: httpx.Client | None = None) -> None:
        self.token_path = token_path
        self.credentials: dict[str, Any] = json.loads(token_path.read_text(encoding="utf-8"))
        self.client = client or httpx.Client(timeout=60, follow_redirects=True)
        self._owns_client = client is None

    def __enter__(self) -> GoogleSession:
        return self

    def __exit__(self, *_args: object) -> None:
        if self._owns_client:
            self.client.close()

    def request(self, method: str, url: str, **kwargs: Any) -> httpx.Response:
        token = self._access_token()
        headers = dict(kwargs.pop("headers", {}))
        headers["Authorization"] = f"Bearer {token}"
        response = self.client.request(method, url, headers=headers, **kwargs)
        if response.status_code == 401 and self.credentials.get("refresh_token"):
            self._refresh()
            headers["Authorization"] = f"Bearer {self._access_token()}"
            response = self.client.request(method, url, headers=headers, **kwargs)
        response.raise_for_status()
        return response

    def _access_token(self) -> str:
        token = str(self.credentials.get("token") or self.credentials.get("access_token") or "")
        if not token:
            self._refresh()
            token = str(self.credentials.get("token") or self.credentials.get("access_token") or "")
        if not token:
            raise RuntimeError(f"Google token file has no access token: {self.token_path}")
        return token

    def _refresh(self) -> None:
        required = ("refresh_token", "client_id", "client_secret")
        missing = [key for key in required if not self.credentials.get(key)]
        if missing:
            raise RuntimeError(f"Google credentials cannot refresh; missing {', '.join(missing)}")
        response = self.client.post(
            str(self.credentials.get("token_uri") or "https://oauth2.googleapis.com/token"),
            data={
                "grant_type": "refresh_token",
                "refresh_token": self.credentials["refresh_token"],
                "client_id": self.credentials["client_id"],
                "client_secret": self.credentials["client_secret"],
            },
        )
        response.raise_for_status()
        refreshed: dict[str, Any] = response.json()
        self.credentials["token"] = refreshed["access_token"]
        self.credentials["access_token"] = refreshed["access_token"]
        self.credentials["expiry"] = (
            dt.datetime.now(dt.UTC) + dt.timedelta(seconds=int(refreshed.get("expires_in", 3600)))
        ).isoformat()
        body = json.dumps(self.credentials, indent=2, sort_keys=True) + "\n"
        descriptor, temporary = tempfile.mkstemp(
            prefix=f".{self.token_path.name}.", dir=self.token_path.parent
        )
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as output:
                output.write(body)
                output.flush()
                os.fsync(output.fileno())
            Path(temporary).chmod(0o600)
            os.replace(temporary, self.token_path)
        finally:
            Path(temporary).unlink(missing_ok=True)


def fetch_drive(
    token_path: Path,
    project: str,
    query: str = "trashed = false",
    client: httpx.Client | None = None,
    cache_dir: Path | None = None,
) -> Iterable[Document]:
    cache = _drive_cache(cache_dir)
    try:
        with GoogleSession(token_path, client) as session:
            page_token: str | None = None
            pending_writes = 0
            while True:
                params = {
                    "q": query,
                    "pageSize": 1000,
                    "fields": DRIVE_FIELDS,
                    "supportsAllDrives": "true",
                    "includeItemsFromAllDrives": "true",
                }
                if page_token:
                    params["pageToken"] = page_token
                payload = session.request(
                    "GET", "https://www.googleapis.com/drive/v3/files", params=params
                ).json()
                for item in payload.get("files", []):
                    file_id = str(item["id"])
                    modified_time = str(item.get("modifiedTime") or "")
                    body = _cached_drive_content(cache, file_id, modified_time)
                    if body is None:
                        body = _drive_content(session, item)
                        if cache is not None:
                            cache.execute(
                                "INSERT OR REPLACE INTO files(id,modified_time,body) VALUES(?,?,?)",
                                (file_id, modified_time, body),
                            )
                            pending_writes += 1
                    if cache is not None:
                        cache.execute("INSERT OR IGNORE INTO seen(id) VALUES(?)", (file_id,))
                        if pending_writes >= 100:
                            cache.commit()
                            pending_writes = 0
                    if not body.strip():
                        continue
                    yield Document(
                        source="google-drive",
                        source_id=file_id,
                        title=str(item.get("name") or "Untitled Drive file"),
                        content=body,
                        uri=item.get("webViewLink"),
                        updated_at=_timestamp(item.get("modifiedTime")),
                        project=project,
                        metadata={
                            "mime_type": item.get("mimeType"),
                            "owners": [
                                owner.get("displayName")
                                for owner in item.get("owners", [])
                                if owner.get("displayName")
                            ],
                        },
                    )
                page_token = payload.get("nextPageToken")
                if not page_token:
                    break
        if cache is not None:
            cache.execute("DELETE FROM files WHERE id NOT IN (SELECT id FROM seen)")
            cache.commit()
    finally:
        if cache is not None:
            cache.close()


def fetch_gmail(
    token_path: Path,
    project: str,
    query: str = "",
    labels: list[str] | None = None,
    client: httpx.Client | None = None,
    cache_dir: Path | None = None,
) -> Iterable[Document]:
    cache = _gmail_cache(cache_dir)
    try:
        with GoogleSession(token_path, client) as session:
            page_token: str | None = None
            pending_writes = 0
            while True:
                params: dict[str, Any] = {"maxResults": 500, "q": query}
                if labels:
                    params["labelIds"] = labels
                if page_token:
                    params["pageToken"] = page_token
                listing = session.request(
                    "GET",
                    "https://gmail.googleapis.com/gmail/v1/users/me/messages",
                    params=params,
                ).json()
                messages: dict[str, dict[str, Any]] = {}
                missing_ids: list[str] = []
                for reference in listing.get("messages", []):
                    message_id = str(reference["id"])
                    message = _cached_gmail_message(cache, message_id)
                    if message is None:
                        missing_ids.append(message_id)
                    else:
                        messages[message_id] = message
                if missing_ids:
                    with ThreadPoolExecutor(
                        max_workers=min(8, len(missing_ids)),
                        thread_name_prefix="cortana-gmail",
                    ) as pool:
                        fetched = pool.map(
                            lambda message_id: _fetch_gmail_message(session, message_id),
                            missing_ids,
                        )
                        messages.update(zip(missing_ids, fetched, strict=True))
                missing_set = set(missing_ids)
                for reference in listing.get("messages", []):
                    message_id = str(reference["id"])
                    message = messages[message_id]
                    if message_id in missing_set and cache is not None:
                        cache.execute(
                            "INSERT OR REPLACE INTO messages(id,body) VALUES(?,?)",
                            (message_id, json.dumps(message, separators=(",", ":"))),
                        )
                        pending_writes += 1
                    if cache is not None:
                        cache.execute("INSERT OR IGNORE INTO seen(id) VALUES(?)", (message_id,))
                        if pending_writes >= 100:
                            cache.commit()
                            pending_writes = 0
                    yield _gmail_document(message, project)
                page_token = listing.get("nextPageToken")
                if not page_token:
                    break
        if cache is not None:
            cache.execute("DELETE FROM messages WHERE id NOT IN (SELECT id FROM seen)")
            cache.commit()
    finally:
        if cache is not None:
            cache.close()


def _fetch_gmail_message(session: GoogleSession, message_id: str) -> dict[str, Any]:
    message: dict[str, Any] = session.request(
        "GET",
        f"https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}",
        params={"format": "full"},
    ).json()
    return message


def _gmail_cache(cache_dir: Path | None) -> sqlite3.Connection | None:
    if cache_dir is None:
        return None
    connection = _private_cache(cache_dir / "gmail.sqlite3")
    connection.execute(
        "CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,body TEXT NOT NULL)"
    )
    connection.execute("CREATE TEMP TABLE seen(id TEXT PRIMARY KEY)")
    return connection


def _drive_cache(cache_dir: Path | None) -> sqlite3.Connection | None:
    if cache_dir is None:
        return None
    connection = _private_cache(cache_dir / "drive.sqlite3")
    connection.execute(
        "CREATE TABLE IF NOT EXISTS files("
        "id TEXT PRIMARY KEY,modified_time TEXT NOT NULL,body TEXT NOT NULL)"
    )
    connection.execute("CREATE TEMP TABLE seen(id TEXT PRIMARY KEY)")
    return connection


def _private_cache(path: Path) -> sqlite3.Connection:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.parent.chmod(0o700)
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
    os.close(descriptor)
    path.chmod(0o600)
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA journal_mode=MEMORY")
    connection.execute("PRAGMA synchronous=NORMAL")
    return connection


def _cached_drive_content(
    cache: sqlite3.Connection | None, file_id: str, modified_time: str
) -> str | None:
    if cache is None:
        return None
    row = cache.execute(
        "SELECT body FROM files WHERE id=? AND modified_time=?",
        (file_id, modified_time),
    ).fetchone()
    return None if row is None else str(row[0])


def _cached_gmail_message(
    cache: sqlite3.Connection | None, message_id: str
) -> dict[str, Any] | None:
    if cache is None:
        return None
    row = cache.execute("SELECT body FROM messages WHERE id=?", (message_id,)).fetchone()
    if row is None:
        return None
    message: dict[str, Any] = json.loads(str(row[0]))
    return message


def fetch_calendar(
    token_path: Path,
    project: str,
    query: str = "",
    client: httpx.Client | None = None,
) -> Iterable[Document]:
    with GoogleSession(token_path, client) as session:
        calendars = session.request(
            "GET", "https://www.googleapis.com/calendar/v3/users/me/calendarList"
        ).json()
        for calendar in calendars.get("items", []):
            calendar_id = str(calendar.get("id") or "")
            if not calendar_id or calendar.get("deleted") or calendar.get("hidden"):
                continue
            encoded_calendar_id = quote(calendar_id, safe="")
            recurring_series: dict[str, dict[str, Any]] = {}
            page_token: str | None = None
            while True:
                params: dict[str, Any] = {
                    "singleEvents": "true",
                    "orderBy": "startTime",
                    "timeMin": (dt.datetime.now(dt.UTC) - dt.timedelta(days=365 * 5)).isoformat(),
                    "maxResults": 2500,
                }
                if query:
                    params["q"] = query
                if page_token:
                    params["pageToken"] = page_token
                payload = session.request(
                    "GET",
                    f"https://www.googleapis.com/calendar/v3/calendars/{encoded_calendar_id}/events",
                    params=params,
                ).json()
                for event in payload.get("items", []):
                    if event.get("status") == "cancelled":
                        continue
                    recurring_id = str(event.get("recurringEventId") or "")
                    if recurring_id:
                        _add_calendar_occurrence(recurring_series, recurring_id, event)
                    else:
                        yield _calendar_document(event, calendar, project)
                page_token = payload.get("nextPageToken")
                if not page_token:
                    break
            for recurring_id, series in recurring_series.items():
                yield _calendar_series_document(recurring_id, series, calendar, project)


def _calendar_document(event: dict[str, Any], calendar: dict[str, Any], project: str) -> Document:
    start = event.get("start", {}).get("dateTime") or event.get("start", {}).get("date") or ""
    end = event.get("end", {}).get("dateTime") or event.get("end", {}).get("date") or ""
    attendees = [
        str(attendee.get("email") or attendee.get("displayName") or "")
        for attendee in event.get("attendees", [])
        if attendee.get("email") or attendee.get("displayName")
    ]
    content = "\n".join(
        part
        for part in [
            f"Calendar: {calendar.get('summary') or calendar.get('id') or ''}",
            f"Start: {start}",
            f"End: {end}",
            f"Location: {event.get('location') or ''}",
            f"Organizer: {event.get('organizer', {}).get('email') or ''}",
            f"Attendees: {', '.join(attendees)}",
            "",
            str(event.get("description") or ""),
        ]
        if part
    ).strip()
    calendar_id = str(calendar.get("id") or "primary")
    return Document(
        source="google-calendar",
        source_id=f"{calendar_id}:{event['id']}",
        title=str(event.get("summary") or "(untitled event)"),
        content=content,
        uri=event.get("htmlLink"),
        updated_at=_timestamp(event.get("updated") or start),
        project=project,
        metadata={
            "calendar_id": calendar_id,
            "calendar": calendar.get("summary"),
            "attendees": attendees,
            "status": event.get("status"),
            "recurring_event_id": event.get("recurringEventId"),
        },
    )


def _add_calendar_occurrence(
    series: dict[str, dict[str, Any]],
    recurring_id: str,
    event: dict[str, Any],
) -> None:
    start = str(event.get("start", {}).get("dateTime") or event.get("start", {}).get("date") or "")
    updated_at = _timestamp(event.get("updated") or start)
    attendees = {
        str(attendee.get("email") or attendee.get("displayName") or "")
        for attendee in event.get("attendees", [])
        if attendee.get("email") or attendee.get("displayName")
    }
    current = series.get(recurring_id)
    if current is None:
        series[recurring_id] = {
            "event": event,
            "count": 1,
            "first_start": start,
            "last_start": start,
            "updated_at": updated_at,
            "attendees": attendees,
        }
        return
    current["count"] += 1
    if start and (not current["first_start"] or start < current["first_start"]):
        current["first_start"] = start
    if start > current["last_start"]:
        current["last_start"] = start
        current["event"] = event
    if updated_at > current["updated_at"]:
        current["updated_at"] = updated_at
    current["attendees"].update(attendees)


def _calendar_series_document(
    recurring_id: str,
    series: dict[str, Any],
    calendar: dict[str, Any],
    project: str,
) -> Document:
    event = series["event"]
    calendar_id = str(calendar.get("id") or "primary")
    attendees = sorted(series["attendees"])
    content = "\n".join(
        part
        for part in [
            f"Calendar: {calendar.get('summary') or calendar_id}",
            (
                f"Recurring series: {series['count']} occurrences from "
                f"{series['first_start']} through {series['last_start']}"
            ),
            f"Location: {event.get('location') or ''}",
            f"Organizer: {event.get('organizer', {}).get('email') or ''}",
            f"Attendees: {', '.join(attendees)}",
            "",
            str(event.get("description") or ""),
        ]
        if part
    ).strip()
    return Document(
        source="google-calendar",
        source_id=f"{calendar_id}:recurring:{recurring_id}",
        title=str(event.get("summary") or "(untitled recurring event)"),
        content=content,
        uri=event.get("htmlLink"),
        updated_at=series["updated_at"],
        project=project,
        metadata={
            "calendar_id": calendar_id,
            "calendar": calendar.get("summary"),
            "attendees": attendees,
            "status": event.get("status"),
            "recurring_event_id": recurring_id,
            "occurrence_count": series["count"],
            "first_start": series["first_start"],
            "last_start": series["last_start"],
        },
    )


def _drive_content(session: GoogleSession, item: dict[str, Any]) -> str:
    file_id = item["id"]
    mime_type = str(item.get("mimeType") or "")
    if mime_type in GOOGLE_EXPORTS:
        export_mime, _extension = GOOGLE_EXPORTS[mime_type]
        response = session.request(
            "GET",
            f"https://www.googleapis.com/drive/v3/files/{file_id}/export",
            params={"mimeType": export_mime},
        )
        return response.text
    if mime_type in TEXT_MIME_TYPES or mime_type.startswith("text/"):
        response = session.request(
            "GET",
            f"https://www.googleapis.com/drive/v3/files/{file_id}",
            params={"alt": "media"},
        )
        return _plain_text(response.text, mime_type)
    if mime_type == "application/pdf":
        response = session.request(
            "GET",
            f"https://www.googleapis.com/drive/v3/files/{file_id}",
            params={"alt": "media"},
        )
        try:
            from pypdf import PdfReader
        except ImportError as error:
            raise RuntimeError("PDF ingestion requires `uv sync --extra ingestion`") from error
        return "\n\n".join(
            page.extract_text() or "" for page in PdfReader(io.BytesIO(response.content)).pages
        ).strip()
    return ""


def _gmail_document(message: dict[str, Any], project: str) -> Document:
    payload = message.get("payload", {})
    headers = {
        str(item.get("name", "")).lower(): str(item.get("value", ""))
        for item in payload.get("headers", [])
    }
    body = _gmail_parts(payload)
    sent_at = _timestamp(headers.get("date"))
    if "internalDate" in message:
        sent_at = dt.datetime.fromtimestamp(int(message["internalDate"]) / 1000, dt.UTC)
    participants = [headers.get(name, "") for name in ("from", "to", "cc") if headers.get(name)]
    content = "\n".join(
        part
        for part in [
            f"From: {headers.get('from', '')}",
            f"To: {headers.get('to', '')}",
            f"Subject: {headers.get('subject', '')}",
            "",
            body or str(message.get("snippet") or ""),
        ]
        if part or part == ""
    )
    thread_id = str(message.get("threadId") or "")
    return Document(
        source="gmail",
        source_id=str(message["id"]),
        title=headers.get("subject") or "(no subject)",
        content=content.strip(),
        uri=f"https://mail.google.com/mail/u/0/#all/{thread_id or message['id']}",
        updated_at=sent_at,
        project=project,
        metadata={
            "thread_id": thread_id,
            "labels": message.get("labelIds", []),
            "participants": participants,
        },
    )


def _gmail_parts(payload: dict[str, Any]) -> str:
    parts = payload.get("parts") or []
    if parts:
        preferred = [
            _gmail_parts(part)
            for part in parts
            if part.get("mimeType") in {"text/plain", "multipart/alternative", "multipart/mixed"}
        ]
        body = "\n".join(part for part in preferred if part)
        if body:
            return body
        return "\n".join(filter(None, (_gmail_parts(part) for part in parts)))
    encoded = payload.get("body", {}).get("data")
    if not encoded:
        return ""
    decoded = base64.urlsafe_b64decode(str(encoded) + "===")
    text = decoded.decode("utf-8", errors="replace")
    return _plain_text(text, str(payload.get("mimeType") or "text/plain"))


def _plain_text(value: str, mime_type: str) -> str:
    if mime_type == "text/html":
        parsed = email.message_from_string(
            f"Content-Type: text/html; charset=utf-8\n\n{value}", policy=email.policy.default
        )
        value = parsed.get_content()
        value = re.sub(r"<(script|style).*?</\1>", " ", value, flags=re.I | re.S)
        value = re.sub(r"<[^>]+>", " ", value)
        value = html.unescape(value)
    return re.sub(r"\n{3,}", "\n\n", value).strip()


def _timestamp(value: object) -> dt.datetime:
    if not value:
        return dt.datetime.now(dt.UTC)
    text = str(value)
    try:
        parsed = dt.datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        parsed = email.utils.parsedate_to_datetime(text)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.UTC)
    return parsed.astimezone(dt.UTC)
