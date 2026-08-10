from __future__ import annotations

import base64
import binascii
import datetime as dt
import email
import email.policy
import email.utils
import html
import json
import os
import re
import sqlite3
import stat
import sys
import tempfile
import time
from collections import deque
from collections.abc import Iterable, Iterator
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from pathlib import Path
from typing import Any, SupportsIndex
from urllib.parse import quote

import httpx

from .http import json_payload
from .model import Document

DRIVE_FIELDS = (
    "nextPageToken,incompleteSearch,"
    "files(id,name,mimeType,modifiedTime,webViewLink,owners(displayName))"
)
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
DEFAULT_MAX_DRIVE_CONTENT_CHARS = 50_000
# Drive listing pages hold up to 1000 files; downloaded bodies are fetched,
# emitted, and cached in fixed batches of this size so a full page never keeps
# every body in memory at once (a 1000-file page would otherwise retain the
# whole page's downloaded content before yielding anything).
DRIVE_BATCH_SIZE = 32
# Keep a connector response bounded even when a provider returns a very large
# text export. The fetch layer applies the user-configured limit afterwards.
MAX_DRIVE_STREAM_CHARS = 256_000
# PDFs are spooled to disk before parsing; this cap prevents an untrusted
# provider response from filling the temporary volume.
MAX_DRIVE_PDF_BYTES = 64 * 1024 * 1024
MAX_TOKEN_FILE_BYTES = 64 * 1024
GOOGLE_REQUEST_RETRIES = 2
GOOGLE_RETRY_BACKOFF_SECONDS = (0.25, 0.75)
GOOGLE_RETRY_STATUSES = {408, 429, 500, 502, 503, 504}
GOOGLE_OAUTH_ERROR_CODES = {
    "invalid_client",
    "invalid_grant",
    "invalid_request",
    "unauthorized_client",
    "unsupported_grant_type",
}
GOOGLE_TRANSIENT_403_REASONS = {
    "rateLimitExceeded",
    "userRateLimitExceeded",
    "backendError",
}
GMAIL_DETAIL_RETRIES = 4
GMAIL_DETAIL_RETRY_BACKOFF_SECONDS = (0.25, 0.75, 1.5, 3.0)
GMAIL_DETAIL_CONCURRENCY = 4
DRIVE_CONTENT_CONCURRENCY = 1


class _DriveContent(str):
    """Bounded Drive content with the provider-size metadata kept separately."""

    original_chars: int
    truncated: bool

    def __new__(cls, value: str, original_chars: int, truncated: bool = False) -> _DriveContent:
        result = str.__new__(cls, value)
        result.original_chars = original_chars
        result.truncated = truncated
        return result

    def __reduce_ex__(self, _protocol: SupportsIndex) -> tuple[Any, tuple[str, int, bool]]:
        # dataclasses.asdict deep-copies Document.content before emitting JSON.
        # A plain str subclass would call __new__ without the metadata args and
        # fail closed during a real connector run.
        return (_restore_drive_content, (str(self), self.original_chars, self.truncated))


def _restore_drive_content(value: str, original_chars: int, truncated: bool) -> _DriveContent:
    return _DriveContent(value, original_chars, truncated)


class _BoundedTextAccumulator:
    """Retain a head/tail sample while counting the complete text stream."""

    def __init__(self, maximum: int) -> None:
        if maximum <= 0:
            raise ValueError("maximum must be greater than zero")
        self.maximum = maximum
        self.total_chars = 0
        self._full_parts: list[str] = []
        self._full_chars = 0
        self._overflowed = False
        self._head_limit = maximum // 2
        self._tail_limit = maximum - self._head_limit
        self._head = ""
        self._tail: deque[str] = deque()
        self._tail_chars = 0

    def append(self, value: str) -> None:
        if not value:
            return
        self.total_chars += len(value)
        if not self._overflowed:
            if self._full_chars + len(value) <= self.maximum:
                self._full_parts.append(value)
                self._full_chars += len(value)
                return
            combined = "".join(self._full_parts) + value
            self._head = combined[: self._head_limit]
            self._append_tail(combined[-self._tail_limit :])
            self._full_parts.clear()
            self._full_chars = 0
            self._overflowed = True
            return
        self._append_tail(value)

    def _append_tail(self, value: str) -> None:
        if not value:
            return
        if len(value) > self._tail_limit:
            value = value[-self._tail_limit :]
        self._tail.append(value)
        self._tail_chars += len(value)
        while self._tail_chars > self._tail_limit:
            excess = self._tail_chars - self._tail_limit
            first = self._tail[0]
            if len(first) <= excess:
                self._tail.popleft()
                self._tail_chars -= len(first)
            else:
                self._tail[0] = first[excess:]
                self._tail_chars -= excess

    def finish(self) -> _DriveContent:
        if not self._overflowed:
            return _DriveContent("".join(self._full_parts), self.total_chars)
        return _DriveContent(
            self._head + "".join(self._tail),
            self.total_chars,
            truncated=True,
        )


def validate_token_path(path: Path) -> Path:
    """Validate a Google token path before reading or replacing credentials."""
    path = path.expanduser()
    if not path.is_absolute():
        raise RuntimeError("Google token path must be absolute")
    _reject_token_symlink_components(path)
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise RuntimeError(f"Google token file does not exist: {path}") from error
    if stat.S_ISLNK(metadata.st_mode):
        raise RuntimeError(f"Google token path must not be a symlink: {path}")
    if not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError(f"Google token path is not a regular file: {path}")
    if os.name == "posix" and stat.S_IMODE(metadata.st_mode) & 0o077:
        raise RuntimeError(f"Google token file must be owner-only (mode 600): {path}")
    if metadata.st_size > MAX_TOKEN_FILE_BYTES:
        raise RuntimeError(f"Google token file exceeds {MAX_TOKEN_FILE_BYTES} bytes: {path}")
    return path


def _reject_token_symlink_components(path: Path) -> None:
    current = path
    while True:
        try:
            metadata = current.lstat()
        except FileNotFoundError:
            metadata = None
        except OSError as error:
            raise RuntimeError(f"Google token path could not be inspected: {current}") from error
        if (
            metadata is not None
            and stat.S_ISLNK(metadata.st_mode)
            and not _is_token_system_alias(current)
        ):
            raise RuntimeError(f"Google token path component must not be a symlink: {current}")
        parent = current.parent
        if parent == current:
            break
        current = parent


def _is_token_system_alias(path: Path) -> bool:
    return sys.platform == "darwin" and path in {Path("/tmp"), Path("/var"), Path("/etc")}


class GoogleSession:
    """Small OAuth REST client compatible with Google token JSON files."""

    def __init__(self, token_path: Path, client: httpx.Client | None = None) -> None:
        self.token_path = validate_token_path(token_path)
        try:
            credentials = json.loads(self.token_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise RuntimeError(f"Google token file is not valid JSON: {self.token_path}") from error
        if not isinstance(credentials, dict):
            raise RuntimeError(f"Google token file must contain a JSON object: {self.token_path}")
        self.credentials = credentials
        self.client = client or httpx.Client(timeout=60, follow_redirects=False)
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
        method_upper = method.upper()
        response: httpx.Response | None = None
        for attempt in range(GOOGLE_REQUEST_RETRIES + 1):
            try:
                response = self.client.request(method, url, headers=headers, **kwargs)
            except httpx.TimeoutException:
                if method_upper not in {"GET", "HEAD"} or attempt >= GOOGLE_REQUEST_RETRIES:
                    raise
                time.sleep(GOOGLE_RETRY_BACKOFF_SECONDS[attempt])
                continue
            if (
                response.status_code in GOOGLE_RETRY_STATUSES
                and method_upper in {"GET", "HEAD"}
                and attempt < GOOGLE_REQUEST_RETRIES
            ):
                time.sleep(GOOGLE_RETRY_BACKOFF_SECONDS[attempt])
                continue
            if (
                response.status_code == 403
                and method_upper in {"GET", "HEAD"}
                and attempt < GOOGLE_REQUEST_RETRIES
                and self._google_403_should_retry(response)
            ):
                time.sleep(GOOGLE_RETRY_BACKOFF_SECONDS[attempt])
                continue
            break
        if response is None:  # pragma: no cover - the retry loop either returns or raises.
            raise RuntimeError("Google request returned no response")
        if response.status_code == 401 and self.credentials.get("refresh_token"):
            self._refresh()
            headers["Authorization"] = f"Bearer {self._access_token()}"
            response = self.client.request(method, url, headers=headers, **kwargs)
        response.raise_for_status()
        return response

    @staticmethod
    def _google_403_should_retry(response: httpx.Response) -> bool:
        try:
            error = response.json()
        except (json.JSONDecodeError, ValueError):
            return False
        if not isinstance(error, dict):
            return False
        error_payload = error.get("error")
        if not isinstance(error_payload, dict):
            return False
        errors = error_payload.get("errors")
        if not isinstance(errors, list):
            return False
        for entry in errors:
            if not isinstance(entry, dict):
                continue
            reason = entry.get("reason")
            if reason in GOOGLE_TRANSIENT_403_REASONS:
                return True
        return False

    @contextmanager
    def stream(self, method: str, url: str, **kwargs: Any) -> Iterator[httpx.Response]:
        """Open an authenticated streaming response, refreshing once on 401."""
        token = self._access_token()
        headers = dict(kwargs.pop("headers", {}))
        headers["Authorization"] = f"Bearer {token}"
        with self.client.stream(method, url, headers=headers, **kwargs) as response:
            if response.status_code != 401 or not self.credentials.get("refresh_token"):
                response.raise_for_status()
                yield response
                return
        self._refresh()
        headers["Authorization"] = f"Bearer {self._access_token()}"
        with self.client.stream(method, url, headers=headers, **kwargs) as response:
            response.raise_for_status()
            yield response

    def _access_token(self) -> str:
        token = str(self.credentials.get("token") or self.credentials.get("access_token") or "")
        if not token:
            self._refresh()
            token = str(self.credentials.get("token") or self.credentials.get("access_token") or "")
        if not token:
            raise RuntimeError(f"Google token file has no access token: {self.token_path}")
        return token

    def _refresh(self) -> None:
        required = ("refresh_token", "client_id")
        missing = [key for key in required if not self.credentials.get(key)]
        if missing:
            raise RuntimeError(f"Google credentials cannot refresh; missing {', '.join(missing)}")
        token_uri = str(self.credentials.get("token_uri") or "https://oauth2.googleapis.com/token")
        _validate_token_uri(token_uri)
        data = {
            "grant_type": "refresh_token",
            "refresh_token": self.credentials["refresh_token"],
            "client_id": self.credentials["client_id"],
        }
        if self.credentials.get("client_secret"):
            data["client_secret"] = self.credentials["client_secret"]
        response = self.client.post(
            token_uri,
            data=data,
        )
        if response.is_error:
            code = _google_oauth_error_code(response)
            if code == "invalid_grant":
                raise RuntimeError(
                    "Google OAuth refresh failed (400: invalid_grant); "
                    "reauthorize the Google source"
                )
            detail = f": {code}" if code else ""
            raise RuntimeError(f"Google OAuth refresh failed ({response.status_code}{detail})")
        refreshed = json_payload(response)
        if not isinstance(refreshed, dict):
            raise RuntimeError("Google OAuth provider returned an invalid response")
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


def _google_oauth_error_code(response: httpx.Response) -> str | None:
    """Return only a small allowlisted OAuth error code from a failed refresh."""

    try:
        payload = json_payload(response, max_bytes=8 * 1024)
    except RuntimeError:
        return None
    if not isinstance(payload, dict):
        return None
    code = payload.get("error")
    if isinstance(code, str) and code in GOOGLE_OAUTH_ERROR_CODES:
        return code
    return None


def _validate_token_uri(value: str) -> None:
    try:
        parsed = httpx.URL(value)
    except (TypeError, ValueError) as error:
        raise RuntimeError("Google token URI is invalid") from error
    host = (parsed.host or "").lower()
    if (
        parsed.scheme != "https"
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
        or host not in {"oauth2.googleapis.com", "www.googleapis.com", "accounts.google.com"}
    ):
        raise RuntimeError("Google token URI must use an HTTPS Google OAuth endpoint")


def fetch_drive(
    token_path: Path,
    project: str,
    query: str = "trashed = false",
    client: httpx.Client | None = None,
    cache_dir: Path | None = None,
    max_content_chars: int = DEFAULT_MAX_DRIVE_CONTENT_CHARS,
    max_documents: int | None = None,
) -> Iterable[Document]:
    if max_content_chars <= 0:
        raise ValueError("max_content_chars must be greater than zero")
    if max_documents is not None and max_documents <= 0:
        raise ValueError("max_documents must be greater than zero")
    strict = max_documents is None
    cache = _drive_cache(cache_dir)
    try:
        with GoogleSession(token_path, client) as session:
            page_token: str | None = None
            pending_writes = 0
            emitted = 0
            while True:
                remaining = None if max_documents is None else max_documents - emitted
                if remaining is not None and remaining <= 0:
                    break
                params = {
                    "q": query,
                    "pageSize": min(1000, remaining) if remaining is not None else 1000,
                    "fields": DRIVE_FIELDS,
                    "supportsAllDrives": "true",
                    "includeItemsFromAllDrives": "true",
                }
                if page_token:
                    params["pageToken"] = page_token
                response = session.request(
                    "GET", "https://www.googleapis.com/drive/v3/files", params=params
                )
                payload = json_payload(response)
                if not isinstance(payload, dict):
                    if strict:
                        raise RuntimeError(
                            "Drive listing is not an object; refusing partial snapshot"
                        )
                    print(
                        "Drive listing skipped: provider returned a non-object value",
                        file=sys.stderr,
                    )
                    break
                if strict and payload.get("incompleteSearch"):
                    raise RuntimeError("Drive listing is incomplete; refusing partial snapshot")
                items = _google_records(payload.get("files"), "Drive file", strict=strict)
                # Download and emit one bounded batch at a time. A Drive page can
                # contain 1000 metadata records; retaining all response bodies
                # until the page completes caused multi-gigabyte RSS spikes.
                for batch_start in range(0, len(items), DRIVE_BATCH_SIZE):
                    batch_items = items[batch_start : batch_start + DRIVE_BATCH_SIZE]
                    bodies: dict[str, str] = {}
                    missing_items: list[dict[str, Any]] = []
                    downloaded_ids: set[str] = set()
                    stale_ids: set[str] = set()
                    for item in batch_items:
                        file_id = item["id"]
                        modified_time = _drive_modified_time(item, file_id, strict)
                        if modified_time is None:
                            continue
                        body = _cached_drive_content(cache, file_id, modified_time)
                        if body is None:
                            missing_items.append(item)
                        else:
                            bodies[file_id] = body
                    if missing_items:
                        with ThreadPoolExecutor(
                            max_workers=min(DRIVE_CONTENT_CONCURRENCY, len(missing_items)),
                            thread_name_prefix="cortana-drive",
                        ) as pool:
                            downloaded = pool.map(
                                lambda item: _safe_drive_content(session, item),
                                missing_items,
                            )
                            for item, (body, error_name) in zip(
                                missing_items, downloaded, strict=True
                            ):
                                file_id = item["id"]
                                if error_name is None:
                                    downloaded_ids.add(file_id)
                                else:
                                    stale = _stale_cached_drive_content(cache, file_id)
                                    if stale is not None:
                                        body = stale
                                        stale_ids.add(file_id)
                                    elif strict:
                                        raise RuntimeError(
                                            "Drive file content unavailable: "
                                            f"id={file_id}; refusing partial snapshot"
                                        )
                                    print(
                                        "drive file content unavailable: "
                                        f"id={file_id} error={error_name} "
                                        f"using_stale_cache={stale is not None}",
                                        file=sys.stderr,
                                    )
                                bodies[file_id] = body
                    for item in batch_items:
                        file_id = str(item["id"])
                        modified_time = _drive_modified_time(item, file_id, strict)
                        if modified_time is None:
                            continue
                        body = bodies[file_id]
                        if file_id in downloaded_ids and cache is not None:
                            cache.execute(
                                "INSERT OR REPLACE INTO files("
                                "id,modified_time,body,original_chars,truncated) VALUES(?,?,?,?,?)",
                                (
                                    file_id,
                                    modified_time,
                                    body,
                                    getattr(body, "original_chars", len(body)),
                                    int(bool(getattr(body, "truncated", False))),
                                ),
                            )
                            pending_writes += 1
                        if cache is not None:
                            cache.execute("INSERT OR IGNORE INTO seen(id) VALUES(?)", (file_id,))
                            if pending_writes >= 100:
                                cache.commit()
                                pending_writes = 0
                        if not body.strip():
                            if strict:
                                raise RuntimeError(
                                    "Drive file has no supported content: "
                                    f"id={file_id}; refusing partial snapshot"
                                )
                            continue
                        try:
                            updated_at = _timestamp(item.get("modifiedTime"))
                        except (TypeError, ValueError, OverflowError, OSError) as error:
                            if strict:
                                raise RuntimeError(
                                    f"Drive file has invalid modifiedTime: id={file_id}"
                                ) from error
                            _warn_skipped_record("Drive file", file_id, error)
                            continue
                        content, content_truncated = _bounded_content(body, max_content_chars)
                        yield Document(
                            source="google-drive",
                            source_id=file_id,
                            title=str(item.get("name") or "Untitled Drive file"),
                            content=content,
                            uri=item.get("webViewLink"),
                            updated_at=updated_at,
                            project=project,
                            metadata={
                                "mime_type": item.get("mimeType"),
                                "owners": [
                                    owner.get("displayName")
                                    for owner in item.get("owners", [])
                                    if isinstance(owner, dict) and owner.get("displayName")
                                ],
                                "content_stale": file_id in stale_ids,
                                "content_truncated": content_truncated
                                or bool(getattr(body, "truncated", False)),
                                "content_original_chars": getattr(
                                    body, "original_chars", len(body)
                                ),
                            },
                        )
                        emitted += 1
                        if max_documents is not None and emitted >= max_documents:
                            break
                    if max_documents is not None and emitted >= max_documents:
                        break
                if max_documents is not None and emitted >= max_documents:
                    break
                raw_next_page_token = payload.get("nextPageToken")
                if raw_next_page_token is None:
                    break
                if isinstance(raw_next_page_token, str) and raw_next_page_token:
                    page_token = raw_next_page_token
                    continue
                if strict:
                    raise RuntimeError(
                        "Drive listing has invalid nextPageToken; refusing partial snapshot"
                    )
                print(
                    "Drive listing skipped: nextPageToken is not a non-empty string",
                    file=sys.stderr,
                )
                break
        if cache is not None:
            if max_documents is None:
                # A capped run is a partial snapshot: it must never prune cached
                # bodies it did not list, or every bounded sync would invalidate
                # the whole derived cache. Additive writes above are safe.
                cache.execute("DELETE FROM files WHERE id NOT IN (SELECT id FROM seen)")
            # Small bounded probes must persist additive cache writes too.
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
    max_documents: int | None = None,
) -> Iterable[Document]:
    strict = max_documents is None
    cache = _gmail_cache(cache_dir)
    try:
        with GoogleSession(token_path, client) as session:
            page_token: str | None = None
            pending_writes = 0
            emitted = 0
            limit_reached = False
            while True:
                params: dict[str, Any] = {
                    "maxResults": min(500, max_documents or 500),
                    "q": query,
                }
                if labels:
                    params["labelIds"] = labels
                if page_token:
                    params["pageToken"] = page_token
                response = session.request(
                    "GET",
                    "https://gmail.googleapis.com/gmail/v1/users/me/messages",
                    params=params,
                )
                listing = json_payload(response)
                if not isinstance(listing, dict):
                    if strict:
                        raise RuntimeError(
                            "Gmail listing is not an object; refusing partial snapshot"
                        )
                    print(
                        "Gmail listing skipped: provider returned a non-object value",
                        file=sys.stderr,
                    )
                    break
                messages: dict[str, dict[str, Any]] = {}
                missing_ids: list[str] = []
                references = _google_records(
                    listing.get("messages"), "Gmail message", strict=strict
                )
                if max_documents is not None:
                    references = references[: max_documents - emitted]
                for reference in references:
                    message_id = reference["id"]
                    message = _cached_gmail_message(cache, message_id)
                    if message is None:
                        missing_ids.append(message_id)
                    elif message.get("id") != message_id:
                        _warn_skipped_record("Gmail message", message_id, "cached id mismatch")
                        if cache is not None:
                            cache.execute("DELETE FROM messages WHERE id=?", (message_id,))
                        missing_ids.append(message_id)
                    else:
                        messages[message_id] = message
                if missing_ids:
                    with ThreadPoolExecutor(
                        max_workers=min(GMAIL_DETAIL_CONCURRENCY, len(missing_ids)),
                        thread_name_prefix="cortana-gmail",
                    ) as pool:
                        fetched = pool.map(
                            lambda message_id: _fetch_gmail_message(session, message_id),
                            missing_ids,
                        )
                        unavailable = 0
                        for message_id, message in zip(missing_ids, fetched, strict=True):
                            if message is None:
                                if strict:
                                    raise RuntimeError(
                                        "Gmail message detail unavailable: "
                                        f"id={message_id}; refusing partial snapshot"
                                    )
                                unavailable += 1
                            else:
                                if message.get("id") != message_id:
                                    if strict:
                                        raise RuntimeError(
                                            "Gmail message detail id mismatch: "
                                            f"requested={message_id} received={message.get('id')}"
                                        )
                                    _warn_skipped_record(
                                        "Gmail message",
                                        message_id,
                                        "detail id mismatch",
                                    )
                                else:
                                    messages[message_id] = message
                        maximum_unavailable = max(10, len(missing_ids) // 10)
                        if unavailable > maximum_unavailable:
                            raise RuntimeError(
                                "Gmail denied too many message details "
                                f"({unavailable}/{len(missing_ids)}); refusing partial snapshot"
                            )
                missing_set = set(missing_ids)
                for reference in references:
                    message_id = reference["id"]
                    message = messages.get(message_id)
                    if message is None:
                        continue
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
                    try:
                        yield _gmail_document(message, project)
                        emitted += 1
                        if max_documents is not None and emitted >= max_documents:
                            limit_reached = True
                            break
                    except (AttributeError, TypeError, ValueError, KeyError) as error:
                        if strict:
                            raise RuntimeError(
                                f"Gmail message conversion failed: id={message_id}"
                            ) from error
                        _warn_skipped_record("Gmail message", message.get("id"), error)
                if limit_reached:
                    break
                raw_next_page_token = listing.get("nextPageToken")
                if raw_next_page_token is None:
                    break
                if isinstance(raw_next_page_token, str) and raw_next_page_token:
                    page_token = raw_next_page_token
                    continue
                if strict:
                    raise RuntimeError(
                        "Gmail listing has invalid nextPageToken; refusing partial snapshot"
                    )
                print(
                    "Gmail listing skipped: nextPageToken is not a non-empty string",
                    file=sys.stderr,
                )
                break
        if cache is not None:
            if max_documents is None:
                # A capped run is a partial snapshot and must not prune cached
                # messages it never listed; only a complete run reconciles the
                # persistent message cache.
                cache.execute("DELETE FROM messages WHERE id NOT IN (SELECT id FROM seen)")
            cache.commit()
    finally:
        if cache is not None:
            cache.close()


def _fetch_gmail_message(session: GoogleSession, message_id: str) -> dict[str, Any] | None:
    response: httpx.Response | None = None
    for attempt in range(GMAIL_DETAIL_RETRIES + 1):
        try:
            response = session.request(
                "GET",
                f"https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}",
                params={"format": "full"},
            )
            break
        except httpx.HTTPStatusError as error:
            if error.response.status_code == 400 and attempt < GMAIL_DETAIL_RETRIES:
                time.sleep(GMAIL_DETAIL_RETRY_BACKOFF_SECONDS[attempt])
                continue
            if error.response.status_code not in {403, 404}:
                raise
            print(
                f"gmail message skipped: id={message_id} status={error.response.status_code}",
                file=sys.stderr,
            )
            return None
    if response is None:  # pragma: no cover - loop always breaks or returns above.
        return None
    message = json_payload(response)
    if not isinstance(message, dict) or not str(message.get("id") or "").strip():
        _warn_skipped_record(
            "Gmail message",
            message.get("id") if isinstance(message, dict) else None,
            "missing id",
        )
        return None
    message["id"] = str(message["id"]).strip()
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
        "id TEXT PRIMARY KEY,modified_time TEXT NOT NULL,body TEXT NOT NULL,"
        "original_chars INTEGER NOT NULL DEFAULT 0,truncated INTEGER NOT NULL DEFAULT 0)"
    )
    # Existing installations have the original three-column cache. Add the
    # metadata columns in place so upgrading does not discard cached content.
    for column, definition in (
        ("original_chars", "INTEGER NOT NULL DEFAULT 0"),
        ("truncated", "INTEGER NOT NULL DEFAULT 0"),
    ):
        try:
            connection.execute(f"ALTER TABLE files ADD COLUMN {column} {definition}")
        except sqlite3.OperationalError as error:
            if "duplicate column name" not in str(error).lower():
                raise
    connection.execute("CREATE TEMP TABLE seen(id TEXT PRIMARY KEY)")
    return connection


def _private_cache(path: Path) -> sqlite3.Connection:
    _prepare_private_directory(path.parent)
    if path.is_symlink():
        raise RuntimeError(f"Google cache path must not be a symlink: {path}")
    flags = os.O_CREAT | os.O_RDWR | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
    finally:
        os.close(descriptor)
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA journal_mode=MEMORY")
    connection.execute("PRAGMA synchronous=NORMAL")
    return connection


def _prepare_private_directory(path: Path) -> None:
    _reject_symlink_components(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    current = path
    while True:
        try:
            metadata = current.lstat()
        except FileNotFoundError as error:
            raise RuntimeError(f"Google cache directory does not exist: {current}") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise RuntimeError(f"Google cache directory must not contain a symlink: {current}")
        if not stat.S_ISDIR(metadata.st_mode):
            raise RuntimeError(f"Google cache path is not a directory: {current}")
        if current == current.parent:
            break
        current = current.parent
    path.chmod(0o700)


def _reject_symlink_components(path: Path) -> None:
    current = path
    while True:
        try:
            metadata = current.lstat()
        except FileNotFoundError:
            if current == current.parent:
                return
            current = current.parent
            continue
        if stat.S_ISLNK(metadata.st_mode):
            raise RuntimeError(f"Google cache directory must not contain a symlink: {current}")
        if current == current.parent:
            return
        current = current.parent


def _cached_drive_content(
    cache: sqlite3.Connection | None, file_id: str, modified_time: str
) -> str | None:
    if cache is None:
        return None
    row = cache.execute(
        "SELECT body,original_chars,truncated FROM files WHERE id=? AND modified_time=?",
        (file_id, modified_time),
    ).fetchone()
    if row is None:
        return None
    body = str(row[0])
    original_chars = int(row[1] or 0) or len(body)
    return _DriveContent(body, original_chars, bool(row[2]))


def _stale_cached_drive_content(cache: sqlite3.Connection | None, file_id: str) -> str | None:
    if cache is None:
        return None
    row = cache.execute(
        "SELECT body,original_chars,truncated FROM files WHERE id=?", (file_id,)
    ).fetchone()
    if row is None:
        return None
    body = str(row[0])
    original_chars = int(row[1] or 0) or len(body)
    return _DriveContent(body, original_chars, bool(row[2]))


def _cached_gmail_message(
    cache: sqlite3.Connection | None, message_id: str
) -> dict[str, Any] | None:
    if cache is None:
        return None
    row = cache.execute("SELECT body FROM messages WHERE id=?", (message_id,)).fetchone()
    if row is None:
        return None
    try:
        message = json.loads(str(row[0]))
    except json.JSONDecodeError:
        _warn_skipped_record("Cached Gmail message", message_id, "invalid cached JSON")
        return None
    if not isinstance(message, dict) or not str(message.get("id") or "").strip():
        _warn_skipped_record("Cached Gmail message", message_id, "missing id")
        return None
    message["id"] = str(message["id"]).strip()
    return message


def fetch_calendar(
    token_path: Path,
    project: str,
    query: str = "",
    client: httpx.Client | None = None,
    max_documents: int | None = None,
) -> Iterable[Document]:
    strict = max_documents is None
    with GoogleSession(token_path, client) as session:
        calendar_records: list[dict[str, Any]] = []
        calendar_page_token: str | None = None
        while True:
            calendar_params: dict[str, Any] = {}
            if calendar_page_token:
                calendar_params["pageToken"] = calendar_page_token
            response = session.request(
                "GET",
                "https://www.googleapis.com/calendar/v3/users/me/calendarList",
                params=calendar_params,
            )
            calendars = json_payload(response)
            if not isinstance(calendars, dict):
                if strict:
                    raise RuntimeError(
                        "Calendar listing is not an object; refusing partial snapshot"
                    )
                print(
                    "Calendar listing skipped: provider returned a non-object value",
                    file=sys.stderr,
                )
                break
            calendar_records.extend(
                _google_records(calendars.get("items"), "Calendar", strict=strict)
            )
            raw_next_page_token = calendars.get("nextPageToken")
            if raw_next_page_token is None:
                break
            if isinstance(raw_next_page_token, str) and raw_next_page_token:
                calendar_page_token = raw_next_page_token
                continue
            if strict:
                raise RuntimeError(
                    "Calendar listing has invalid nextPageToken; refusing partial snapshot"
                )
            print(
                "Calendar listing skipped: nextPageToken is not a non-empty string",
                file=sys.stderr,
            )
            break
        emitted = 0
        for calendar in calendar_records:
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
                    "maxResults": min(2500, max_documents or 2500),
                }
                if query:
                    params["q"] = query
                if page_token:
                    params["pageToken"] = page_token
                response = session.request(
                    "GET",
                    f"https://www.googleapis.com/calendar/v3/calendars/{encoded_calendar_id}/events",
                    params=params,
                )
                payload = json_payload(response)
                if not isinstance(payload, dict):
                    if strict:
                        raise RuntimeError(
                            "Calendar events are not an object; refusing partial snapshot"
                        )
                    print(
                        "Calendar events skipped: provider returned a non-object value",
                        file=sys.stderr,
                    )
                    break
                events = _google_records(payload.get("items"), "Calendar event", strict=strict)
                for event in events:
                    if event.get("status") == "cancelled":
                        continue
                    recurring_id = str(event.get("recurringEventId") or "")
                    if recurring_id:
                        try:
                            _add_calendar_occurrence(recurring_series, recurring_id, event)
                        except (AttributeError, TypeError, ValueError, KeyError) as error:
                            if strict:
                                raise RuntimeError(
                                    f"Calendar event conversion failed: id={event.get('id')}"
                                ) from error
                            _warn_skipped_record("Calendar event", event.get("id"), error)
                    else:
                        try:
                            yield _calendar_document(event, calendar, project)
                            emitted += 1
                            if max_documents is not None and emitted >= max_documents:
                                return
                        except (AttributeError, TypeError, ValueError, KeyError) as error:
                            if strict:
                                raise RuntimeError(
                                    f"Calendar event conversion failed: id={event.get('id')}"
                                ) from error
                            _warn_skipped_record("Calendar event", event.get("id"), error)
                raw_next_page_token = payload.get("nextPageToken")
                if raw_next_page_token is None:
                    break
                if isinstance(raw_next_page_token, str) and raw_next_page_token:
                    page_token = raw_next_page_token
                    continue
                if strict:
                    raise RuntimeError(
                        "Calendar events have invalid nextPageToken; refusing partial snapshot"
                    )
                print(
                    "Calendar events skipped: nextPageToken is not a non-empty string",
                    file=sys.stderr,
                )
                break
            for recurring_id, series in recurring_series.items():
                yield _calendar_series_document(recurring_id, series, calendar, project)
                emitted += 1
                if max_documents is not None and emitted >= max_documents:
                    return


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
        return _stream_drive_text(
            session,
            f"https://www.googleapis.com/drive/v3/files/{file_id}/export",
            params={"mimeType": export_mime},
        )
    if mime_type in TEXT_MIME_TYPES or mime_type.startswith("text/"):
        return _stream_drive_text(
            session,
            f"https://www.googleapis.com/drive/v3/files/{file_id}",
            params={"alt": "media"},
            mime_type=mime_type,
        )
    if mime_type == "application/pdf":
        try:
            from pypdf import PdfReader  # type: ignore[import-not-found]
        except ImportError as error:
            raise RuntimeError("PDF ingestion requires `uv sync --extra ingestion`") from error
        with tempfile.NamedTemporaryFile(prefix="cortana-drive-", suffix=".pdf") as output:
            total_bytes = 0
            with session.stream(
                "GET",
                f"https://www.googleapis.com/drive/v3/files/{file_id}",
                params={"alt": "media"},
            ) as response:
                for chunk in response.iter_bytes():
                    total_bytes += len(chunk)
                    if total_bytes > MAX_DRIVE_PDF_BYTES:
                        raise RuntimeError(
                            f"Drive PDF exceeds the {MAX_DRIVE_PDF_BYTES} byte safety limit"
                        )
                    output.write(chunk)
            output.flush()
            accumulator = _BoundedTextAccumulator(MAX_DRIVE_STREAM_CHARS)
            for page in PdfReader(output.name).pages:
                accumulator.append(page.extract_text() or "")
                accumulator.append("\n\n")
            result = accumulator.finish()
            return _DriveContent(str(result).strip(), result.original_chars, result.truncated)
    return ""


def _stream_drive_text(
    session: GoogleSession,
    url: str,
    *,
    params: dict[str, str],
    mime_type: str | None = None,
) -> _DriveContent:
    accumulator = _BoundedTextAccumulator(MAX_DRIVE_STREAM_CHARS)
    with session.stream("GET", url, params=params) as response:
        for chunk in response.iter_text():
            accumulator.append(chunk)
    result = accumulator.finish()
    if mime_type is None:
        return result
    cleaned = _plain_text(str(result), mime_type)
    return _DriveContent(cleaned, result.original_chars, result.truncated)


def _safe_drive_content(session: GoogleSession, item: dict[str, Any]) -> tuple[str, str | None]:
    try:
        return _drive_content(session, item), None
    except Exception as error:
        return "", type(error).__name__


def _bounded_content(value: str, max_chars: int) -> tuple[str, bool]:
    if len(value) <= max_chars:
        return value, False
    marker = f"\n\n[Cortana omitted {len(value) - max_chars:,} middle characters]\n\n"
    available = max_chars - len(marker)
    if available <= 0:
        return value[:max_chars], True
    head = available // 2
    tail = available - head
    return f"{value[:head]}{marker}{value[-tail:]}", True


def _gmail_document(message: dict[str, Any], project: str) -> Document:
    payload = message.get("payload", {})
    if not isinstance(payload, dict):
        payload = {}
    headers = {
        str(item.get("name", "")).lower(): str(item.get("value", ""))
        for item in payload.get("headers", [])
        if isinstance(item, dict) and item.get("name")
    }
    body = _gmail_parts(payload)
    sent_at = _timestamp(headers.get("date"))
    if "internalDate" in message:
        try:
            sent_at = dt.datetime.fromtimestamp(int(message["internalDate"]) / 1000, dt.UTC)
        except (TypeError, ValueError, OverflowError, OSError):
            _warn_skipped_record(
                "Gmail message timestamp", message.get("id"), "invalid internalDate"
            )
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
    parts = [part for part in payload.get("parts") or [] if isinstance(part, dict)]
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
    body_value = payload.get("body")
    encoded = body_value.get("data") if isinstance(body_value, dict) else None
    if not encoded:
        return ""
    try:
        decoded = base64.urlsafe_b64decode(str(encoded) + "===")
    except (binascii.Error, TypeError, ValueError):
        _warn_skipped_record("Gmail body", "unknown", "invalid base64 payload")
        return ""
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


def _drive_modified_time(item: dict[str, Any], file_id: str, strict: bool) -> str | None:
    modified_time = item.get("modifiedTime")
    if not isinstance(modified_time, str) or not modified_time.strip():
        if strict:
            raise RuntimeError(
                f"Drive file has no modifiedTime: id={file_id}; refusing partial snapshot"
            )
        _warn_skipped_record("Drive file", file_id, "missing modifiedTime")
        return None
    return modified_time


def _google_records(value: object, kind: str, strict: bool = False) -> list[dict[str, Any]]:
    """Return usable provider records.

    Capped runs skip malformed records with a diagnostic so bounded validation
    can tolerate provider noise. Strict (uncapped) runs fail closed instead:
    a record that cannot be parsed would otherwise be silently omitted from
    what downstream reconciliation treats as a complete snapshot.
    """
    if value is None:
        if strict:
            raise RuntimeError(f"{kind} list is missing; refusing partial snapshot")
        return []
    if not isinstance(value, list):
        if strict:
            raise RuntimeError(f"{kind} list is not a list; refusing partial snapshot")
        print(f"{kind} list skipped: provider returned a non-list value", file=sys.stderr)
        return []
    records: list[dict[str, Any]] = []
    for index, record in enumerate(value):
        if not isinstance(record, dict):
            if strict:
                raise RuntimeError(
                    f"{kind} record={index} is not an object; refusing partial snapshot"
                )
            print(f"{kind} skipped: record={index} is not an object", file=sys.stderr)
            continue
        record_id = record.get("id")
        if not isinstance(record_id, str):
            if strict:
                raise RuntimeError(
                    f"{kind} record={index} has a non-string id; refusing partial snapshot"
                )
            print(f"{kind} skipped: record={index} has a non-string id", file=sys.stderr)
            continue
        record_id = record_id.strip()
        if not record_id:
            if strict:
                raise RuntimeError(f"{kind} record={index} has no id; refusing partial snapshot")
            print(f"{kind} skipped: record={index} has no id", file=sys.stderr)
            continue
        record["id"] = record_id
        records.append(record)
    return records


def _warn_skipped_record(kind: str, record_id: object, reason: object) -> None:
    safe_id = str(record_id or "unknown")
    print(f"{kind} skipped: id={safe_id} reason={reason}", file=sys.stderr)
