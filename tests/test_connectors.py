from __future__ import annotations

import base64
import datetime as dt
import io
import json
import os
import sqlite3
import subprocess
from pathlib import Path
from typing import Any

import httpx
import pytest

from cortana.connectors import __main__ as connector_cli
from cortana.connectors import apple_notes, buzz, chat, google
from cortana.connectors.__main__ import main
from cortana.connectors.google import (
    GoogleSession,
    _gmail_document,
    _plain_text,
    _private_cache,
    _timestamp,
    fetch_calendar,
    fetch_drive,
    fetch_gmail,
    validate_token_path,
)
from cortana.connectors.model import Document, emit


def response(
    payload: Any, status: int = 200, request: httpx.Request | None = None
) -> httpx.Response:
    return httpx.Response(
        status, json=payload, request=request or httpx.Request("GET", "https://example.test")
    )


def test_document_jsonl_emit_uses_utc_and_skips_empty() -> None:
    output = io.StringIO()
    count = emit(
        [
            Document(
                source="test",
                source_id="1",
                title="Example",
                content="useful",
                updated_at=dt.datetime(2026, 1, 2, tzinfo=dt.timezone(dt.timedelta(hours=-4))),
                acl=("agent",),
            ),
            Document(source="test", source_id="2", title="Empty", content=" "),
        ],
        output,
    )

    assert count == 1
    payload = json.loads(output.getvalue())
    assert payload["updated_at"] == "2026-01-02T04:00:00+00:00"
    assert payload["acl"] == ["agent"]


def test_apple_notes_normalizes_jxa_rows(monkeypatch: pytest.MonkeyPatch) -> None:
    completed = subprocess.CompletedProcess(
        args=[],
        returncode=0,
        stdout=json.dumps(
            [
                {
                    "id": "x-coredata://note/1",
                    "name": "Plan",
                    "body": "Ship Cortana",
                    "modified": "2026-07-29T10:00:00.000Z",
                    "account": "iCloud",
                    "folder": "Notes",
                },
                {
                    "id": "x-coredata://note/2",
                    "name": "Blank",
                    "body": " ",
                    "modified": "2026-07-29T10:00:00.000Z",
                },
                {
                    "id": "x-coredata://note/bad",
                    "name": "Malformed",
                    "body": "Keep the rest of the export usable",
                    "modified": "not-a-timestamp",
                },
            ]
        ),
        stderr="",
    )
    monkeypatch.setattr(subprocess, "run", lambda *_args, **_kwargs: completed)

    documents = list(apple_notes.fetch(project="personal"))

    assert len(documents) == 1
    assert documents[0].source_id == "x-coredata://note/1"
    assert documents[0].metadata == {"account": "iCloud", "folder": "Notes"}


def test_apple_notes_reports_actionable_timeout(monkeypatch: pytest.MonkeyPatch) -> None:
    def timeout(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(["osascript"], 120)

    monkeypatch.setattr(subprocess, "run", timeout)

    with pytest.raises(RuntimeError, match="grant Automation access"):
        list(apple_notes.fetch())


def test_apple_notes_rejects_malformed_or_oversized_exports(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    malformed = subprocess.CompletedProcess(args=[], returncode=0, stdout="{}", stderr="")
    monkeypatch.setattr(subprocess, "run", lambda *_args, **_kwargs: malformed)
    with pytest.raises(RuntimeError, match="invalid export shape"):
        list(apple_notes.fetch())

    oversized = subprocess.CompletedProcess(
        args=[], returncode=0, stdout="x" * (apple_notes.MAX_EXPORT_BYTES + 1), stderr=""
    )
    monkeypatch.setattr(subprocess, "run", lambda *_args, **_kwargs: oversized)
    with pytest.raises(RuntimeError, match="safety limit"):
        list(apple_notes.fetch())


def test_buzz_reads_personas_and_logs_read_only(tmp_path: Path) -> None:
    agents = tmp_path / "agents"
    logs = agents / "logs"
    logs.mkdir(parents=True)
    database = agents / "retention.db"
    with sqlite3.connect(database) as connection:
        connection.execute(
            "CREATE TABLE persona_events("
            "kind INTEGER,pubkey TEXT,d_tag TEXT,content TEXT,created_at INTEGER,"
            "raw_event TEXT,pending_sync INTEGER)"
        )
        connection.execute(
            "INSERT INTO persona_events VALUES(?,?,?,?,?,?,?)",
            (30078, "pub", "profile", "Agent profile", 1_700_000_000, '{"id":"event"}', 0),
        )
        connection.execute(
            "INSERT INTO persona_events VALUES(?,?,?,?,?,?,?)",
            (30078, "pub", "bad-time", "Skip this event", "not-a-time", "{}", 0),
        )
        connection.execute(
            "INSERT INTO persona_events VALUES(?,?,?,?,?,?,?)",
            (30078, "pub", "bad-json", "Keep this event", 1_700_000_001, "not-json", 0),
        )
        connection.execute(
            "INSERT INTO persona_events VALUES(?,?,?,?,?,?,?)",
            (30078, "", "bad-identity", "Skip this event", 1_700_000_002, "{}", 0),
        )
        connection.execute(
            "INSERT INTO persona_events VALUES(?,?,?,?,?,?,?)",
            (30078, "pub", "empty-content", "", 1_700_000_003, "{}", 0),
        )
    (logs / "agent.log").write_text("started agent", encoding="utf-8")

    documents = list(buzz.fetch(tmp_path))

    assert [document.source_id for document in documents] == [
        "persona:30078:pub:profile",
        "persona:30078:pub:bad-json",
        "log:agent.log",
    ]
    assert documents[0].metadata["raw_event"]["id"] == "event"
    assert documents[1].metadata["raw_event"] is None


def test_buzz_rejects_symlinked_retention_files_and_logs(tmp_path: Path) -> None:
    agents = tmp_path / "agents"
    logs = agents / "logs"
    logs.mkdir(parents=True)
    external_database = tmp_path / "external-retention.db"
    external_database.touch()
    try:
        (agents / "retention.db").symlink_to(external_database)
    except (NotImplementedError, OSError):
        return

    with pytest.raises(RuntimeError, match="retention database must be a regular"):
        list(buzz.fetch(tmp_path))

    (agents / "retention.db").unlink()
    external_log = tmp_path / "external.log"
    external_log.write_text("private", encoding="utf-8")
    (logs / "linked.log").symlink_to(external_log)
    with pytest.raises(RuntimeError, match="log must not be a symlink"):
        list(buzz.fetch(tmp_path))


class FakeSlackClient:
    def __init__(self, *_args: object, **_kwargs: object) -> None:
        self.calls: list[str] = []

    def __enter__(self) -> FakeSlackClient:
        return self

    def __exit__(self, *_args: object) -> None:
        pass

    def get(self, path: str, **_kwargs: object) -> httpx.Response:
        self.calls.append(path)
        if path == "/conversations.history":
            return response(
                {
                    "ok": True,
                    "messages": [
                        {"ts": "10.0", "user": "U1", "text": "Launch?", "reply_count": 1},
                        {"ts": "not-a-timestamp", "user": "U9", "text": "Ignore me"},
                    ],
                    "response_metadata": {"next_cursor": ""},
                }
            )
        return response(
            {
                "ok": True,
                "messages": [
                    {"ts": "10.0", "user": "U1", "text": "Launch?"},
                    {"ts": "11.0", "user": "U2", "text": "Yes"},
                    {"ts": "invalid", "user": "U9", "text": "Ignore me"},
                ],
            }
        )


class FakeDiscordClient:
    def __init__(self, *_args: object, **_kwargs: object) -> None:
        pass

    def __enter__(self) -> FakeDiscordClient:
        return self

    def __exit__(self, *_args: object) -> None:
        pass

    def get(self, _path: str, **_kwargs: object) -> httpx.Response:
        return response(
            [
                {
                    "id": "99",
                    "content": "Status",
                    "attachments": [{"url": "https://files.test/report.pdf"}],
                    "timestamp": "2026-07-29T12:00:00Z",
                    "author": {"id": "u1", "username": "Ada"},
                }
            ]
        )


def test_chat_retries_server_directed_rate_limits(monkeypatch: pytest.MonkeyPatch) -> None:
    calls = 0
    delays: list[float] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal calls
        calls += 1
        if calls == 1:
            return httpx.Response(
                429,
                json={"retry_after": 0.25},
                request=request,
            )
        return httpx.Response(200, json={"ok": True}, request=request)

    monkeypatch.setattr(chat.time, "sleep", delays.append)
    with httpx.Client(transport=httpx.MockTransport(handler)) as client:
        result = chat._get_with_backoff(client, "https://api.test/messages", params={})

    assert result.status_code == 200
    assert calls == 2
    assert delays == [0.25]


def test_chat_retry_policy_respects_headers_and_bounded_fallback(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    request = httpx.Request("GET", "https://api.test/messages")
    assert (
        chat._retry_after(
            httpx.Response(429, headers={"retry-after": "2.5"}, request=request),
            0,
        )
        == 2.5
    )
    assert (
        chat._retry_after(
            httpx.Response(
                429,
                headers={"retry-after": "invalid"},
                json={"retry_after": 0.5},
                request=request,
            ),
            0,
        )
        == 0.5
    )
    assert chat._retry_after(httpx.Response(503, text="not-json", request=request), 3) == 8.0

    delays: list[float] = []
    monkeypatch.setattr(chat.time, "sleep", delays.append)
    chat._respect_rate_limit_headers(
        httpx.Response(
            200,
            headers={"x-ratelimit-remaining": "0", "x-ratelimit-reset-after": "0.75"},
            request=request,
        )
    )
    chat._respect_rate_limit_headers(
        httpx.Response(
            200,
            headers={"x-ratelimit-remaining": "1"},
            request=request,
        )
    )
    assert delays == [0.75]


def test_chat_returns_final_retryable_response_after_bound() -> None:
    request = httpx.Request("GET", "https://api.test/messages")

    def handler(_request: httpx.Request) -> httpx.Response:
        return httpx.Response(503, request=request)

    with httpx.Client(transport=httpx.MockTransport(handler)) as client:
        result = chat._get_with_backoff(
            client,
            "https://api.test/messages",
            params={},
            max_attempts=1,
        )
    assert result.status_code == 503


def test_chat_connectors_reassemble_slack_and_normalize_discord(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    monkeypatch.setattr(chat.httpx, "Client", FakeSlackClient)
    slack_documents = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN"))
    assert slack_documents[0].content == "U1: Launch?\nU2: Yes"
    assert slack_documents[0].metadata["participants"] == ["U1", "U2"]

    monkeypatch.setenv("DISCORD_TEST_TOKEN", "secret")
    monkeypatch.setattr(chat.httpx, "Client", FakeDiscordClient)
    discord_documents = list(chat.fetch_discord(["D1"], "work", "DISCORD_TEST_TOKEN"))
    assert "https://files.test/report.pdf" in discord_documents[0].content
    assert discord_documents[0].metadata["author_id"] == "u1"


def test_discord_skips_malformed_messages_without_aborting_the_batch() -> None:
    assert (
        chat._discord_document(
            {"id": "bad", "content": "content", "timestamp": "not-a-timestamp"},
            "D1",
            "work",
        )
        is None
    )
    document = chat._discord_document(
        {
            "id": "100",
            "content": "Status",
            "attachments": [None, {"url": "https://files.test/report.pdf"}],
            "timestamp": "2026-07-29T12:00:00Z",
            "author": None,
        },
        "D1",
        "work",
    )
    assert document is not None
    assert document.source_id == "100"
    assert document.metadata["author_id"] is None


def test_discord_cache_uses_incremental_after_cursor(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("DISCORD_TEST_TOKEN", "secret")
    real_client = httpx.Client
    cursors: list[str | None] = []

    def handler(request: httpx.Request) -> httpx.Response:
        after = request.url.params.get("after")
        cursors.append(after)
        messages = (
            [
                {
                    "id": "99",
                    "content": "Status",
                    "attachments": [],
                    "timestamp": "2026-07-29T12:00:00Z",
                    "author": {"id": "u1", "username": "Ada"},
                }
            ]
            if after is None
            else []
        )
        return response(messages, request=request)

    monkeypatch.setattr(
        chat.httpx,
        "Client",
        lambda **_kwargs: real_client(
            base_url="https://discord.test",
            transport=httpx.MockTransport(handler),
        ),
    )
    cache = tmp_path / "cache"
    first = list(chat.fetch_discord(["D1"], "work", "DISCORD_TEST_TOKEN", cache_dir=cache))
    second = list(chat.fetch_discord(["D1"], "work", "DISCORD_TEST_TOKEN", cache_dir=cache))

    assert first == second
    assert cursors == [None, "99"]
    assert (cache / "discord.sqlite3").stat().st_mode & 0o777 == 0o600
    assert cache.stat().st_mode & 0o777 == 0o700


def test_discord_cache_rejects_symlinked_directory_and_database(tmp_path: Path) -> None:
    external = tmp_path / "external-cache"
    external.mkdir()
    linked_directory = tmp_path / "cache"
    try:
        linked_directory.symlink_to(external, target_is_directory=True)
    except (NotImplementedError, OSError):
        return

    with pytest.raises(RuntimeError, match="directory must not contain a symlink"):
        chat._discord_cache(linked_directory)

    linked_directory.unlink()
    linked_directory.mkdir()
    external_database = tmp_path / "external.sqlite3"
    external_database.touch()
    (linked_directory / "discord.sqlite3").symlink_to(external_database)
    with pytest.raises(RuntimeError, match="cache path must not be a symlink"):
        chat._discord_cache(linked_directory)


def test_chat_connector_rejects_missing_token(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("MISSING_TOKEN", raising=False)
    with pytest.raises(RuntimeError, match="MISSING_TOKEN is required"):
        list(chat.fetch_slack(["C1"], "work", "MISSING_TOKEN"))


def test_google_token_path_is_absolute_bounded_and_not_symlinked(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    if os.name == "posix":
        token.chmod(0o600)
    assert validate_token_path(token) == token

    if os.name == "posix":
        broad = tmp_path / "broad-token.json"
        broad.write_text('{"token":"access"}', encoding="utf-8")
        broad.chmod(0o644)
        with pytest.raises(RuntimeError, match="owner-only"):
            validate_token_path(broad)

    with pytest.raises(RuntimeError, match="must be absolute"):
        validate_token_path(Path("relative-token.json"))

    oversized = tmp_path / "oversized.json"
    oversized.write_bytes(b"x" * (64 * 1024 + 1))
    if os.name == "posix":
        oversized.chmod(0o600)
    with pytest.raises(RuntimeError, match="exceeds"):
        validate_token_path(oversized)

    try:
        linked = tmp_path / "linked-token.json"
        linked.symlink_to(token)
    except (NotImplementedError, OSError):
        pass
    else:
        with pytest.raises(RuntimeError, match="must not be a symlink"):
            validate_token_path(linked)


def test_google_private_cache_rejects_symlink(tmp_path: Path) -> None:
    target = tmp_path / "cache-target.sqlite3"
    target.touch()
    linked = tmp_path / "cache.sqlite3"
    try:
        linked.symlink_to(target)
    except (NotImplementedError, OSError):
        return

    with pytest.raises(RuntimeError, match="must not be a symlink"):
        _private_cache(linked)


def test_google_private_cache_rejects_symlinked_directory(tmp_path: Path) -> None:
    external = tmp_path / "external-cache"
    external.mkdir()
    linked = tmp_path / "cache"
    try:
        linked.symlink_to(external, target_is_directory=True)
    except (NotImplementedError, OSError):
        return

    with pytest.raises(RuntimeError, match="directory must not contain a symlink"):
        _private_cache(linked / "drive.sqlite3")


def test_google_drive_exports_supported_content(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "application/vnd.google-apps.document",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                            "webViewLink": "https://docs.google.com/document/d/doc1",
                            "owners": [{"displayName": "Ada"}],
                        },
                        {
                            "id": "bin1",
                            "name": "Image",
                            "mimeType": "image/png",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="Quarterly roadmap", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_drive(token, "work", client=client))

    assert len(documents) == 1
    assert documents[0].content == "Quarterly roadmap"
    assert documents[0].metadata["owners"] == ["Ada"]


def test_google_drive_skips_malformed_listing_records(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        None,
                        {"name": "missing id"},
                        {
                            "id": "bad-time",
                            "name": "Bad timestamp",
                            "mimeType": "text/plain",
                            "modifiedTime": "not-a-timestamp",
                        },
                        {
                            "id": "valid",
                            "name": "Keep this file",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="Useful content", request=request)

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert [document.source_id for document in documents] == ["valid"]
    diagnostic = capsys.readouterr().err
    assert "Drive file skipped: record=0 is not an object" in diagnostic
    assert "Drive file skipped: id=bad-time" in diagnostic


def test_google_drive_bounds_oversized_exports_with_explicit_metadata(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    body = "header\n" + ("middle-row\n" * 100) + "final-row"

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "csv1",
                            "name": "Large export",
                            "mimeType": "text/csv",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text=body, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    document = list(fetch_drive(token, "work", client=client, max_content_chars=200))[0]

    assert len(document.content) <= 200
    assert document.content.startswith("header")
    assert document.content.endswith("final-row")
    assert "Cortana omitted" in document.content
    assert document.metadata["content_truncated"] is True
    assert document.metadata["content_original_chars"] == len(body)


def test_google_drive_reuses_content_until_modified(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    cache = tmp_path / "cache"
    content_requests = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal content_requests
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "application/vnd.google-apps.document",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        content_requests += 1
        return httpx.Response(200, text="Download once", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_drive(token, "work", client=client, cache_dir=cache))
    second = list(fetch_drive(token, "work", client=client, cache_dir=cache))

    assert first == second
    assert content_requests == 1
    assert (cache / "drive.sqlite3").stat().st_mode & 0o777 == 0o600


def test_google_drive_isolates_content_failure_and_uses_stale_cache(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    cache = tmp_path / "cache"
    modified_time = "2026-07-29T12:00:00Z"
    fail = False

    def handler(request: httpx.Request) -> httpx.Response:
        return response(
            {
                "files": [
                    {
                        "id": "doc1",
                        "name": "Roadmap",
                        "mimeType": "application/vnd.google-apps.document",
                        "modifiedTime": modified_time,
                    }
                ]
            },
            request=request,
        )

    def content(_session: GoogleSession, _item: dict[str, Any]) -> str:
        if fail:
            raise ValueError("sensitive provider detail")
        return "Last known good content"

    monkeypatch.setattr(google, "_drive_content", content)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_drive(token, "work", client=client, cache_dir=cache))
    modified_time = "2026-07-29T13:00:00Z"
    fail = True
    second = list(fetch_drive(token, "work", client=client, cache_dir=cache))

    assert second[0].content == first[0].content
    assert first[0].metadata["content_stale"] is False
    assert second[0].metadata["content_stale"] is True
    diagnostic = capsys.readouterr().err
    assert "error=ValueError using_stale_cache=True" in diagnostic
    assert "sensitive provider detail" not in diagnostic


def test_google_gmail_decodes_message_body(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    encoded = base64.urlsafe_b64encode(b"Deployment is green").decode()

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        return response(
            {
                "id": "m1",
                "threadId": "t1",
                "internalDate": "1700000000000",
                "labelIds": ["INBOX"],
                "payload": {
                    "headers": [
                        {"name": "From", "value": "ada@example.test"},
                        {"name": "To", "value": "team@example.test"},
                        {"name": "Subject", "value": "Release"},
                    ],
                    "mimeType": "text/plain",
                    "body": {"data": encoded},
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_gmail(token, "work", client=client))

    assert documents[0].title == "Release"
    assert "Deployment is green" in documents[0].content
    assert documents[0].metadata["thread_id"] == "t1"


def test_google_gmail_skips_malformed_listing_records(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response(
                {"messages": [None, {"labelIds": ["INBOX"]}, {"id": "m1"}]},
                request=request,
            )
        return response(
            {
                "id": "m1",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Keep this message"}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(b"Useful mail").decode(),
                    },
                },
            },
            request=request,
        )

    documents = list(
        fetch_gmail(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert [document.source_id for document in documents] == ["m1"]
    assert "Gmail message skipped: record=0 is not an object" in capsys.readouterr().err


def test_google_gmail_reuses_private_message_cache(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    cache = tmp_path / "cache"
    detail_requests = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal detail_requests
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        detail_requests += 1
        return response(
            {
                "id": "m1",
                "threadId": "t1",
                "internalDate": "1700000000000",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Cached"}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(b"Download once").decode(),
                    },
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_gmail(token, "work", client=client, cache_dir=cache))
    second = list(fetch_gmail(token, "work", client=client, cache_dir=cache))

    assert first == second
    assert detail_requests == 1
    assert (cache / "gmail.sqlite3").stat().st_mode & 0o777 == 0o600
    assert cache.stat().st_mode & 0o777 == 0o700


def test_google_gmail_skips_isolated_inaccessible_message(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response(
                {"messages": [{"id": "available"}, {"id": "denied"}]},
                request=request,
            )
        if request.url.path.endswith("/denied"):
            return response({"error": "forbidden"}, status=403, request=request)
        return response(
            {
                "id": "available",
                "threadId": "t1",
                "internalDate": "1700000000000",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Available"}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(b"Still indexed").decode(),
                    },
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_gmail(token, "work", client=client))

    assert [document.source_id for document in documents] == ["available"]


def test_google_gmail_refuses_broad_detail_denial(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")
    client = httpx.Client(
        transport=httpx.MockTransport(
            lambda request: response(
                {"messages": [{"id": f"m{index}"} for index in range(11)]},
                request=request,
            )
        )
    )
    monkeypatch.setattr(
        google,
        "_fetch_gmail_message",
        lambda _session, _message_id: None,
    )

    with pytest.raises(RuntimeError, match="refusing partial snapshot"):
        list(fetch_gmail(token, "work", client=client))


def test_google_calendar_normalizes_events(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/calendarList"):
            return response(
                {"items": [{"id": "primary", "summary": "Work"}]},
                request=request,
            )
        return response(
            {
                "items": [
                    {
                        "id": "event-1",
                        "summary": "Release review",
                        "description": "Approve the rollout.",
                        "start": {"dateTime": "2026-07-29T12:00:00Z"},
                        "end": {"dateTime": "2026-07-29T12:30:00Z"},
                        "updated": "2026-07-29T11:00:00Z",
                        "htmlLink": "https://calendar.google.com/event?eid=1",
                        "status": "confirmed",
                        "attendees": [{"email": "ada@example.test"}],
                    }
                ]
            },
            request=request,
        )

    documents = list(
        fetch_calendar(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert documents[0].source_id == "primary:event-1"
    assert "Approve the rollout." in documents[0].content
    assert documents[0].metadata["attendees"] == ["ada@example.test"]


def test_google_calendar_collapses_recurring_occurrences(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/calendarList"):
            return response(
                {"items": [{"id": "primary", "summary": "Work"}]},
                request=request,
            )
        return response(
            {
                "items": [
                    {
                        "id": "instance-1",
                        "recurringEventId": "daily-standup",
                        "summary": "Standup",
                        "start": {"dateTime": "2026-07-28T12:00:00Z"},
                        "end": {"dateTime": "2026-07-28T12:15:00Z"},
                        "updated": "2026-07-28T13:00:00Z",
                        "attendees": [{"email": "ada@example.test"}],
                    },
                    {
                        "id": "instance-2",
                        "recurringEventId": "daily-standup",
                        "summary": "Standup",
                        "start": {"dateTime": "2026-07-29T12:00:00Z"},
                        "end": {"dateTime": "2026-07-29T12:15:00Z"},
                        "updated": "2026-07-29T13:00:00Z",
                        "attendees": [{"email": "grace@example.test"}],
                    },
                ]
            },
            request=request,
        )

    documents = list(
        fetch_calendar(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert len(documents) == 1
    assert documents[0].source_id == "primary:recurring:daily-standup"
    assert "2 occurrences" in documents[0].content
    assert documents[0].metadata["occurrence_count"] == 2
    assert documents[0].metadata["attendees"] == [
        "ada@example.test",
        "grace@example.test",
    ]


def test_google_calendar_skips_malformed_events(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    token.write_text('{"token":"access"}', encoding="utf-8")

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/calendarList"):
            return response({"items": [{"id": "primary", "summary": "Work"}]}, request=request)
        return response(
            {
                "items": [
                    {"id": "broken", "start": "not-an-object"},
                    {
                        "id": "valid",
                        "summary": "Keep this event",
                        "start": {"dateTime": "2026-07-29T12:00:00Z"},
                        "end": {"dateTime": "2026-07-29T12:30:00Z"},
                        "updated": "2026-07-29T11:00:00Z",
                    },
                ]
            },
            request=request,
        )

    documents = list(
        fetch_calendar(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert [document.source_id for document in documents] == ["primary:valid"]
    assert "Calendar event skipped: id=broken" in capsys.readouterr().err


def test_google_session_refreshes_and_secures_token_file(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text(
        json.dumps(
            {
                "refresh_token": "refresh",
                "client_id": "client",
                "client_secret": "secret",
                "token_uri": "https://oauth2.test/token",
            }
        ),
        encoding="utf-8",
    )

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.host == "oauth2.test":
            return response({"access_token": "new-access", "expires_in": 100}, request=request)
        return response({"ok": True}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session:
        assert session.request("GET", "https://api.test/data").json() == {"ok": True}

    saved = json.loads(token.read_text(encoding="utf-8"))
    assert saved["token"] == "new-access"
    assert token.stat().st_mode & 0o777 == 0o600


def test_google_session_retries_unauthorized_response(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text(
        json.dumps(
            {
                "token": "expired",
                "refresh_token": "refresh",
                "client_id": "client",
                "client_secret": "secret",
            }
        ),
        encoding="utf-8",
    )
    calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal calls
        if request.url.path == "/token":
            return response({"access_token": "fresh"}, request=request)
        calls += 1
        return response({"ok": True}, 401 if calls == 1 else 200, request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session:
        assert session.request("GET", "https://api.test/data").json() == {"ok": True}
    assert calls == 2


def test_google_session_refresh_allows_desktop_client_without_secret(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text(
        json.dumps({"refresh_token": "refresh", "client_id": "desktop-client"}),
        encoding="utf-8",
    )

    def handler(request: httpx.Request) -> httpx.Response:
        assert b"client_secret" not in request.content
        return response({"access_token": "fresh"}, request=request)

    with GoogleSession(
        token,
        httpx.Client(transport=httpx.MockTransport(handler)),
    ) as session:
        assert session._access_token() == "fresh"


def test_google_helpers_normalize_html_dates_and_snippets() -> None:
    assert _plain_text("<style>x{}</style><p>Hello &amp; world</p>", "text/html") == "Hello & world"
    assert _timestamp("Tue, 29 Jul 2026 12:00:00 +0000").year == 2026
    message = _gmail_document(
        {
            "id": "m2",
            "snippet": "Fallback text",
            "payload": {
                "headers": [
                    {"name": "Subject", "value": "Fallback"},
                    {"name": "Date", "value": "Tue, 29 Jul 2026 12:00:00 +0000"},
                ]
            },
        },
        "personal",
    )
    assert "Fallback text" in message.content
    assert message.uri.endswith("/m2")


def test_google_session_reports_unrefreshable_credentials(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    token.write_text("{}", encoding="utf-8")
    with (
        GoogleSession(token, httpx.Client()) as session,
        pytest.raises(RuntimeError, match="missing refresh_token"),
    ):
        session.request("GET", "https://api.test/data")


def test_connector_cli_emits_buzz_jsonl(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    logs = tmp_path / "agents" / "logs"
    logs.mkdir(parents=True)
    (logs / "agent.log").write_text("event", encoding="utf-8")

    assert main(["--project", "agents", "buzz", "--root", str(tmp_path)]) == 0

    captured = capsys.readouterr()
    assert json.loads(captured.out)["project"] == "agents"
    assert "emitted=1" in captured.err


def test_connector_cli_dispatches_chat_sources(monkeypatch: pytest.MonkeyPatch) -> None:
    expected = [Document(source="test", source_id="1", title="One", content="Body")]
    monkeypatch.setattr(connector_cli, "fetch_slack", lambda *_args: expected)
    monkeypatch.setattr(connector_cli, "fetch_discord", lambda *_args, **_kwargs: expected)

    slack_args = connector_cli.parser().parse_args(
        ["--project", "work", "slack", "--channel", "C1"]
    )
    discord_args = connector_cli.parser().parse_args(
        ["--project", "work", "discord", "--channel", "D1"]
    )
    assert list(connector_cli._documents(slack_args)) == expected
    assert list(connector_cli._documents(discord_args)) == expected


def test_connector_cli_requires_existing_google_token(tmp_path: Path) -> None:
    args = connector_cli.parser().parse_args(
        ["google-drive", "--token", str(tmp_path / "missing.json")]
    )
    with pytest.raises(RuntimeError, match="does not exist"):
        connector_cli._documents(args)


def test_connector_entrypoint_reports_expected_failures(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def fail(_argv: list[str] | None = None) -> int:
        raise RuntimeError("expected failure")

    monkeypatch.setattr(connector_cli, "main", fail)

    assert connector_cli.entrypoint() == 1
    assert capsys.readouterr().err == "connector error: expected failure\n"
