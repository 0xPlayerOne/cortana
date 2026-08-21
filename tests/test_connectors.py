from __future__ import annotations

import base64
import datetime as dt
import io
import json
import os
import sqlite3
import subprocess
import sys
import threading
import time
import types
import zipfile
from pathlib import Path
from typing import Any

import httpx
import pytest

from cortana import __version__
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
from cortana.connectors.http import MAX_JSON_RESPONSE_BYTES, json_payload
from cortana.connectors.model import Document, emit


def response(
    payload: Any, status: int = 200, request: httpx.Request | None = None
) -> httpx.Response:
    return httpx.Response(
        status, json=payload, request=request or httpx.Request("GET", "https://example.test")
    )


def write_token(path: Path, body: str) -> None:
    path.write_text(body, encoding="utf-8")
    if os.name == "posix":
        path.chmod(0o600)


def test_connector_json_payload_is_bounded_and_reports_invalid_bodies() -> None:
    request = httpx.Request("GET", "https://example.test")
    assert json_payload(httpx.Response(200, json={"ok": True}, request=request)) == {"ok": True}

    oversized = httpx.Response(
        200,
        content=b"{}" + b" " * MAX_JSON_RESPONSE_BYTES,
        request=request,
    )
    with pytest.raises(RuntimeError, match="exceeds"):
        json_payload(oversized)

    invalid = httpx.Response(200, content=b"not-json", request=request)
    with pytest.raises(RuntimeError, match="invalid JSON"):
        json_payload(invalid)


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
    assert documents[0].uri == "notes://showNote?identifier=x-coredata%3A%2F%2Fnote%2F1"
    assert documents[0].metadata == {"account": "iCloud", "folder": "Notes"}


def test_apple_notes_honors_document_cap(monkeypatch: pytest.MonkeyPatch) -> None:
    rows = [
        {
            "id": f"x-coredata://note/{index}",
            "name": f"Note {index}",
            "body": f"Body {index}",
            "modified": "2026-07-29T10:00:00.000Z",
        }
        for index in range(3)
    ]
    completed = subprocess.CompletedProcess(
        args=[], returncode=0, stdout=json.dumps(rows), stderr=""
    )
    monkeypatch.setattr(subprocess, "run", lambda *_args, **_kwargs: completed)

    documents = list(apple_notes.fetch(project="personal", max_documents=1))

    assert [document.source_id for document in documents] == ["x-coredata://note/0"]


def test_apple_notes_bounds_script_when_max_documents_is_set(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured: dict[str, list[str]] = {}

    def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        captured["args"] = args
        return subprocess.CompletedProcess(args=[], returncode=0, stdout="[]", stderr="")

    monkeypatch.setattr(subprocess, "run", run)

    list(apple_notes.fetch(project="personal", max_documents=2))

    assert captured["args"][0] == apple_notes.OSASCRIPT
    script = captured["args"][4]
    assert "const maxDocuments = 2;" in script
    assert "break outer;" in script


def test_apple_notes_passes_exact_folder_filters_to_jxa(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured: dict[str, list[str]] = {}

    def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        captured["args"] = args
        return subprocess.CompletedProcess(args=[], returncode=0, stdout="[]", stderr="")

    monkeypatch.setattr(subprocess, "run", run)

    list(
        apple_notes.fetch(
            project="work",
            folders=["Nifty League"],
            exclude_folders=["The Pink Binder"],
        )
    )

    script = captured["args"][4]
    assert 'const includeFolders = ["Nifty League"];' in script
    assert 'const excludeFolders = ["The Pink Binder"];' in script
    assert "folderName = folder.name()" in script


def test_apple_notes_uses_unbounded_script_when_not_capped(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured: dict[str, list[str]] = {}

    def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        captured["args"] = args
        return subprocess.CompletedProcess(args=[], returncode=0, stdout="[]", stderr="")

    monkeypatch.setattr(subprocess, "run", run)

    list(apple_notes.fetch(project="personal"))

    script = captured["args"][4]
    assert "const maxDocuments = undefined;" in script


def test_apple_notes_reports_actionable_timeout(monkeypatch: pytest.MonkeyPatch) -> None:
    def timeout(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(["osascript"], 120)

    monkeypatch.setattr(subprocess, "run", timeout)

    with pytest.raises(RuntimeError, match="grant Automation access"):
        list(apple_notes.fetch())


def test_apple_notes_reports_actionable_permission_denial(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def denied(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[str]:
        raise subprocess.CalledProcessError(
            1,
            ["osascript"],
            stderr="Not authorized to send Apple events to Notes.",
        )

    monkeypatch.setattr(subprocess, "run", denied)

    with pytest.raises(RuntimeError, match="Automation access to Apple Notes was denied"):
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
    assert documents[0].uri == "buzz://persona/pub/profile"
    assert documents[0].metadata["raw_event"]["id"] == "event"
    assert documents[1].metadata["raw_event"] is None


def test_buzz_honors_document_cap(tmp_path: Path) -> None:
    logs = tmp_path / "agents" / "logs"
    logs.mkdir(parents=True)
    (logs / "first.log").write_text("first", encoding="utf-8")
    (logs / "second.log").write_text("second", encoding="utf-8")

    documents = list(buzz.fetch(tmp_path, max_documents=1))

    assert [document.source_id for document in documents] == ["log:first.log"]


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
    try:
        (agents / "retention.db").symlink_to(tmp_path / "missing-retention.db")
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


def test_buzz_rejects_symlinked_source_directories(tmp_path: Path) -> None:
    external_root = tmp_path / "external-root"
    external_root.mkdir()
    linked_root = tmp_path / "linked-root"
    linked_root.symlink_to(external_root, target_is_directory=True)
    with pytest.raises(RuntimeError, match="root must not be a symlink"):
        list(buzz.fetch(linked_root))

    agents = tmp_path / "agents"
    external_agents = tmp_path / "external-agents"
    external_agents.mkdir()
    agents.symlink_to(external_agents, target_is_directory=True)
    with pytest.raises(RuntimeError, match="agents directory must not be a symlink"):
        list(buzz.fetch(tmp_path))

    agents.unlink()
    agents.mkdir()
    external_logs = tmp_path / "external-logs"
    external_logs.mkdir()
    (agents / "logs").symlink_to(external_logs, target_is_directory=True)
    with pytest.raises(RuntimeError, match="logs directory must not be a symlink"):
        list(buzz.fetch(tmp_path))


def test_buzz_rejects_oversized_logs(tmp_path: Path) -> None:
    logs = tmp_path / "agents" / "logs"
    logs.mkdir(parents=True)
    oversized = logs / "oversized.log"
    with oversized.open("wb") as stream:
        stream.truncate(buzz.MAX_LOG_BYTES + 1)

    with pytest.raises(RuntimeError, match="log exceeds"):
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


class FakeDiscordRpc:
    messages = [
        {
            "id": "99",
            "content": "Status",
            "attachments": [{"url": "https://files.test/report.pdf"}],
            "timestamp": "2026-07-29T12:00:00Z",
            "author": {"id": "u1", "username": "Ada"},
        }
    ]

    def __init__(self) -> None:
        self.closed = False

    @classmethod
    def connect(cls, _client_id: str, _access_token: str) -> FakeDiscordRpc:
        return cls()

    def get_channel(self, _channel_id: str) -> dict[str, Any]:
        return {"messages": list(self.messages)}

    def close(self) -> None:
        self.closed = True


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
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    monkeypatch.setattr(chat.httpx, "Client", FakeSlackClient)
    slack_documents = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", max_documents=1))
    assert slack_documents[0].content == "U1: Launch?\nU2: Yes"
    assert slack_documents[0].metadata["participants"] == ["U1", "U2"]

    token = tmp_path / "discord-rpc-token.json"
    client = tmp_path / "discord-rpc-client.json"
    write_token(token, '{"access_token":"secret","expiry":"2099-01-01T00:00:00Z"}')
    write_token(client, '{"client_id":"client"}')
    monkeypatch.setattr(chat, "_DiscordRpc", FakeDiscordRpc)
    discord_documents = list(chat.fetch_discord(["D1"], "work", token, client, max_documents=1))
    assert "https://files.test/report.pdf" in discord_documents[0].content
    assert discord_documents[0].metadata["author_id"] == "u1"


def test_discord_connector_honors_document_cap(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "discord-rpc-token.json"
    client = tmp_path / "discord-rpc-client.json"
    write_token(token, '{"access_token":"secret","expiry":"2099-01-01T00:00:00Z"}')
    write_token(client, '{"client_id":"client"}')
    FakeDiscordRpc.messages = [
        {
            "id": "100",
            "content": "First",
            "attachments": [],
            "timestamp": "2026-07-29T12:00:00Z",
            "author": {"id": "u1", "username": "Ada"},
        },
        {
            "id": "99",
            "content": "Second",
            "attachments": [],
            "timestamp": "2026-07-29T11:00:00Z",
            "author": {"id": "u2", "username": "Grace"},
        },
    ]
    monkeypatch.setattr(chat, "_DiscordRpc", FakeDiscordRpc)
    cache = tmp_path / "cache"
    documents = list(
        chat.fetch_discord(["D1"], "work", token, client, cache_dir=cache, max_documents=1)
    )

    assert [document.source_id for document in documents] == ["100"]
    assert not (cache / "discord.sqlite3").exists()


def test_discord_connector_refreshes_expired_desktop_rpc_tokens(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "discord-rpc-token.json"
    client = tmp_path / "discord-rpc-client.json"
    write_token(
        token,
        json.dumps(
            {
                "access_token": "expired-access",
                "refresh_token": "refresh-token",
                "token_type": "Bearer",
                "expiry": "2000-01-01T00:00:00Z",
            }
        ),
    )
    write_token(client, '{"client_id":"client","client_secret":"secret"}')
    calls: list[dict[str, str]] = []

    class RefreshClient:
        def __init__(self, *_args: object, **_kwargs: object) -> None:
            pass

        def __enter__(self) -> RefreshClient:
            return self

        def __exit__(self, *_args: object) -> None:
            pass

        def post(self, _url: str, *, data: dict[str, str]) -> httpx.Response:
            calls.append(data)
            return response(
                {
                    "access_token": "fresh-access",
                    "refresh_token": "fresh-refresh",
                    "expires_in": 3600,
                    "token_type": "Bearer",
                }
            )

    monkeypatch.setattr(chat.httpx, "Client", RefreshClient)
    monkeypatch.setattr(chat, "_DiscordRpc", FakeDiscordRpc)
    list(chat.fetch_discord(["D1"], "work", token, client, max_documents=1))

    assert calls == [
        {
            "client_id": "client",
            "client_secret": "secret",
            "refresh_token": "refresh-token",
            "grant_type": "refresh_token",
        }
    ]
    refreshed = json.loads(token.read_text(encoding="utf-8"))
    assert refreshed["access_token"] == "fresh-access"
    assert refreshed["refresh_token"] == "fresh-refresh"
    assert token.stat().st_mode & 0o777 == 0o600


def test_slack_message_pages_fail_closed_on_invalid_shapes() -> None:
    assert chat._slack_messages({"messages": [None, {"ts": "1.0"}]}) == [{"ts": "1.0"}]
    with pytest.raises(RuntimeError, match="invalid message page"):
        chat._slack_messages({"messages": "not-a-list"})
    with pytest.raises(RuntimeError, match="no usable records"):
        chat._slack_messages({"messages": [None]})


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
    assert chat._discord_page([None, {"id": "bad"}, {"id": "100"}]) == [{"id": "100"}]
    with pytest.raises(RuntimeError, match="no usable records"):
        chat._discord_page([None, {"id": "bad"}])


def test_discord_cache_upserts_rpc_snapshots_without_pruning_history(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "discord-rpc-token.json"
    client = tmp_path / "discord-rpc-client.json"
    write_token(token, '{"access_token":"secret","expiry":"2099-01-01T00:00:00Z"}')
    write_token(client, '{"client_id":"client"}')
    FakeDiscordRpc.messages = [
        {
            "id": "99",
            "content": "Status",
            "attachments": [],
            "timestamp": "2026-07-29T12:00:00Z",
            "author": {"id": "u1", "username": "Ada"},
        }
    ]
    monkeypatch.setattr(chat, "_DiscordRpc", FakeDiscordRpc)
    cache = tmp_path / "cache"
    first = list(chat.fetch_discord(["D1"], "work", token, client, cache_dir=cache))
    FakeDiscordRpc.messages = [
        *FakeDiscordRpc.messages,
        {
            "id": "100",
            "content": "Follow-up",
            "attachments": [],
            "timestamp": "2026-07-29T13:00:00Z",
            "author": {"id": "u2", "username": "Grace"},
        },
    ]
    second = list(chat.fetch_discord(["D1"], "work", token, client, cache_dir=cache))

    assert [document.source_id for document in first] == ["99"]
    assert [document.source_id for document in second] == ["99", "100"]
    assert (cache / "discord.sqlite3").stat().st_mode & 0o777 == 0o600
    assert cache.stat().st_mode & 0o777 == 0o700


def test_slack_cache_uses_incremental_oldest_and_emits_complete_snapshots(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    real_client = httpx.Client
    history_calls = 0
    oldest_values: list[str | None] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal history_calls
        if request.url.path.endswith("/conversations.history"):
            history_calls += 1
            oldest = request.url.params.get("oldest")
            oldest_values.append(oldest)
            if history_calls == 1:
                messages = [{"ts": "10.0", "user": "U1", "text": "Launch?", "reply_count": 1}]
            elif history_calls == 2:
                messages = [{"ts": "12.0", "user": "U3", "text": "Shipped", "reply_count": 0}]
            else:
                messages = []
            return response(
                {"ok": True, "messages": messages, "response_metadata": {"next_cursor": ""}},
                request=request,
            )
        if request.url.path.endswith("/conversations.replies"):
            return response(
                {
                    "ok": True,
                    "messages": [
                        {"ts": "10.0", "user": "U1", "text": "Launch?"},
                        {"ts": "11.0", "user": "U2", "text": "Yes"},
                    ],
                },
                request=request,
            )
        raise AssertionError(f"unexpected Slack request: {request.url}")

    monkeypatch.setattr(
        chat.httpx,
        "Client",
        lambda **_kwargs: real_client(
            base_url="https://slack.test",
            transport=httpx.MockTransport(handler),
        ),
    )
    cache = tmp_path / "cache"
    first = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache))
    second = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache))

    assert [document.source_id for document in first] == ["C1:10.0"]
    assert [document.source_id for document in second] == ["C1:10.0", "C1:12.0"]
    assert oldest_values == [None, "10.0"]
    assert (cache / "slack.sqlite3").stat().st_mode & 0o777 == 0o600
    assert cache.stat().st_mode & 0o777 == 0o700

    # A complete refresh removes a deleted parent without relying on a bounded
    # emission cap to decide what remains searchable.
    with sqlite3.connect(cache / "slack.sqlite3") as connection:
        connection.execute("UPDATE slack_channels SET last_full=?", ("2000-01-01T00:00:00+00:00",))
        connection.commit()
    assert list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache)) == []


def test_slack_cache_rebuilds_when_cursor_is_corrupt(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    real_client = httpx.Client
    oldest_values: list[str | None] = []

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/conversations.history"):
            oldest_values.append(request.url.params.get("oldest"))
            return response(
                {
                    "ok": True,
                    "messages": [{"ts": "20.0", "user": "U1", "text": "Recovered"}],
                    "response_metadata": {"next_cursor": ""},
                },
                request=request,
            )
        raise AssertionError(f"unexpected Slack request: {request.url}")

    monkeypatch.setattr(
        chat.httpx,
        "Client",
        lambda **_kwargs: real_client(
            base_url="https://slack.test",
            transport=httpx.MockTransport(handler),
        ),
    )
    cache = tmp_path / "cache"
    cache.mkdir()
    with sqlite3.connect(cache / "slack.sqlite3") as connection:
        connection.executescript(
            """
            CREATE TABLE slack_threads(
                channel_id TEXT NOT NULL,parent_ts TEXT NOT NULL,body TEXT NOT NULL,
                PRIMARY KEY(channel_id,parent_ts)
            );
            CREATE TABLE slack_channels(
                channel_id TEXT PRIMARY KEY,latest_ts TEXT,last_full TEXT NOT NULL
            );
            INSERT INTO slack_channels VALUES ('C1', 'not-a-timestamp', '2099-01-01T00:00:00+00:00');
            """
        )

    documents = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache))

    assert [document.source_id for document in documents] == ["C1:20.0"]
    assert oldest_values == [None]


def test_slack_incremental_pagination_continues_while_next_cursor_exists(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    real_client = httpx.Client
    history_requests: list[tuple[str | None, str | None]] = []

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/conversations.history"):
            cursor = request.url.params.get("cursor")
            history_requests.append((request.url.params.get("oldest"), cursor))
            messages = (
                [{"ts": "11.0", "user": "U2", "text": "Older new message"}]
                if cursor
                else [{"ts": "12.0", "user": "U1", "text": "Newest"}]
            )
            return response(
                {
                    "ok": True,
                    "messages": messages,
                    "response_metadata": {"next_cursor": "" if cursor else "next-page"},
                },
                request=request,
            )
        raise AssertionError(f"unexpected Slack request: {request.url}")

    monkeypatch.setattr(
        chat.httpx,
        "Client",
        lambda **_kwargs: real_client(
            base_url="https://slack.test",
            transport=httpx.MockTransport(handler),
        ),
    )
    cache = tmp_path / "cache"
    cache.mkdir()
    with sqlite3.connect(cache / "slack.sqlite3") as connection:
        connection.execute(
            "CREATE TABLE slack_channels("
            "channel_id TEXT PRIMARY KEY,latest_ts TEXT,last_full TEXT NOT NULL)"
        )
        connection.execute(
            "INSERT INTO slack_channels(channel_id,latest_ts,last_full) VALUES(?,?,?)",
            ("C1", "10.0", "2099-01-01T00:00:00+00:00"),
        )
        connection.commit()

    documents = list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache))

    assert [document.source_id for document in documents] == ["C1:11.0", "C1:12.0"]
    assert history_requests == [("10.0", None), (None, "next-page")]
    connection = sqlite3.connect(cache / "slack.sqlite3")
    try:
        row = connection.execute(
            "SELECT latest_ts,last_full FROM slack_channels WHERE channel_id='C1'"
        ).fetchone()
    finally:
        connection.close()
    assert row == ("12.0", "2099-01-01T00:00:00+00:00")


def test_slack_incremental_page_failure_keeps_cached_cursor(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    real_client = httpx.Client
    history_calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal history_calls
        if request.url.path.endswith("/conversations.history"):
            history_calls += 1
            if history_calls == 1:
                return response(
                    {
                        "ok": True,
                        "messages": [{"ts": "12.0", "user": "U1", "text": "Newest"}],
                        "response_metadata": {"next_cursor": "next-page"},
                    },
                    request=request,
                )
            raise RuntimeError("simulated Slack history failure")
        raise AssertionError(f"unexpected Slack request: {request.url}")

    monkeypatch.setattr(
        chat.httpx,
        "Client",
        lambda **_kwargs: real_client(
            base_url="https://slack.test",
            transport=httpx.MockTransport(handler),
        ),
    )
    cache = tmp_path / "cache"
    cache.mkdir()
    with sqlite3.connect(cache / "slack.sqlite3") as connection:
        connection.execute(
            "CREATE TABLE slack_channels("
            "channel_id TEXT PRIMARY KEY,latest_ts TEXT,last_full TEXT NOT NULL)"
        )
        connection.execute(
            "INSERT INTO slack_channels(channel_id,latest_ts,last_full) VALUES(?,?,?)",
            ("C1", "10.0", "2099-01-01T00:00:00+00:00"),
        )
        connection.commit()

    with pytest.raises(RuntimeError, match="simulated Slack history failure"):
        list(chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache))

    assert history_calls == 2
    connection = sqlite3.connect(cache / "slack.sqlite3")
    try:
        row = connection.execute(
            "SELECT latest_ts,last_full FROM slack_channels WHERE channel_id='C1'"
        ).fetchone()
    finally:
        connection.close()
    assert row == ("10.0", "2099-01-01T00:00:00+00:00")


def test_slack_bounded_run_does_not_mutate_cursor_cache(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("SLACK_TEST_TOKEN", "secret")
    real_client = httpx.Client
    history_calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal history_calls
        if request.url.path.endswith("/conversations.history"):
            history_calls += 1
            return response(
                {
                    "ok": True,
                    "messages": [
                        {"ts": "20.0", "user": "U1", "text": "Newest"},
                        {"ts": "19.0", "user": "U2", "text": "Older"},
                    ],
                    "response_metadata": {"next_cursor": "next-page"},
                },
                request=request,
            )
        raise AssertionError(f"unexpected Slack request: {request.url}")

    monkeypatch.setattr(
        chat.httpx,
        "Client",
        lambda **_kwargs: real_client(
            base_url="https://slack.test",
            transport=httpx.MockTransport(handler),
        ),
    )
    cache = tmp_path / "cache"

    documents = list(
        chat.fetch_slack(["C1"], "work", "SLACK_TEST_TOKEN", cache_dir=cache, max_documents=1)
    )

    assert [document.source_id for document in documents] == ["C1:20.0"]
    assert history_calls == 1
    assert not (cache / "slack.sqlite3").exists()


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
    write_token(token, '{"token":"access"}')
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


def test_google_token_path_rejects_symlinked_parent(tmp_path: Path) -> None:
    real = tmp_path / "real"
    real.mkdir()
    token = real / "token.json"
    write_token(token, '{"token":"access"}')
    linked = tmp_path / "linked"
    try:
        linked.symlink_to(real, target_is_directory=True)
    except (NotImplementedError, OSError):
        return

    with pytest.raises(RuntimeError, match="component must not be a symlink"):
        validate_token_path(linked / "token.json")


def test_google_drive_content_concurrency_is_bounded_parallel() -> None:
    assert google.DRIVE_CONTENT_CONCURRENCY == 4


def test_google_drive_fetches_bodies_in_bounded_parallel_batches(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    active = 0
    maximum = 0
    lock = threading.Lock()

    def fake_content(_session: GoogleSession, _item: dict[str, Any]) -> tuple[str, None]:
        nonlocal active, maximum
        with lock:
            active += 1
            maximum = max(maximum, active)
        time.sleep(0.03)
        with lock:
            active -= 1
        return "body", None

    monkeypatch.setattr(google, "_safe_drive_content", fake_content)

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": f"doc-{index}",
                            "name": f"Document {index}",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                        for index in range(8)
                    ]
                },
                request=request,
            )
        raise AssertionError(f"unexpected Drive request: {request.url}")

    with httpx.Client(transport=httpx.MockTransport(handler)) as client:
        documents = list(fetch_drive(token, "work", client=client, max_documents=8))

    assert len(documents) == 8
    assert 1 < maximum <= google.DRIVE_CONTENT_CONCURRENCY


def test_large_pdf_uses_bounded_head_tail_page_sample() -> None:
    class FakePage:
        def __init__(self, text: str) -> None:
            self.text = text
            self.calls = 0

        def extract_text(self) -> str:
            self.calls += 1
            return self.text

    pages = [FakePage(f"page-{index}\n" + ("x" * 10_000)) for index in range(129)]
    result = google._extract_pdf_text(type("Reader", (), {"pages": pages})())

    assert result.truncated is True
    assert result.original_chars is None
    assert len(result) <= google.MAX_DRIVE_STREAM_CHARS
    assert "page-0" in result
    assert "page-128" in result
    assert google.PDF_SAMPLE_MARKER.strip() in result
    assert sum(page.calls for page in pages) < len(pages)


def test_large_low_text_pdf_does_not_walk_every_page() -> None:
    class FakePage:
        def __init__(self, index: int) -> None:
            self.index = index
            self.calls = 0

        def extract_text(self) -> str:
            self.calls += 1
            return f"page-{self.index}\n"

    pages = [FakePage(index) for index in range(10_000)]
    result = google._extract_pdf_text(type("Reader", (), {"pages": pages})())

    assert result.truncated is True
    assert google.PDF_SAMPLE_MARKER.strip() in result
    assert sum(page.calls for page in pages) <= google.MAX_DRIVE_SAMPLE_PAGES
    assert "page-0" in result
    assert "page-9999" in result


def test_empty_text_pdf_emits_explicit_unavailable_marker() -> None:
    class EmptyPage:
        def extract_text(self) -> str:
            return ""

    result = google._extract_pdf_text(type("Reader", (), {"pages": [EmptyPage()]})())

    assert str(result) == google.PDF_NO_TEXT_MARKER
    assert result.truncated is True
    assert result.original_chars is None


def test_docx_text_is_extracted_with_a_bounded_zip_reader(tmp_path: Path) -> None:
    path = tmp_path / "document.docx"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr(
            "word/document.xml",
            """<document xmlns='urn:word'><body><p><t>Hello</t><t> world</t></p></body></document>""",
        )

    result = google._extract_docx_text(str(path))

    assert "Hello world" in result
    assert result.truncated is False
    assert result.original_chars is not None


def test_text_heavy_pdf_stops_after_bounded_payload() -> None:
    class FakePage:
        def __init__(self, text: str) -> None:
            self.text = text
            self.calls = 0

        def extract_text(self) -> str:
            self.calls += 1
            return self.text

    pages = [FakePage("x" * 10_000) for _ in range(30)]
    result = google._extract_pdf_text(type("Reader", (), {"pages": pages})())

    assert result.truncated is True
    assert result.original_chars is None
    assert len(result) <= google.MAX_DRIVE_STREAM_CHARS
    assert google.PDF_EARLY_STOP_MARKER.strip() in result
    assert sum(page.calls for page in pages) < len(pages)


def test_pdf_extraction_enforces_a_cooperative_wall_clock_budget() -> None:
    class SlowPage:
        def extract_text(self) -> str:
            time.sleep(0.02)
            return "slow page"

    with pytest.raises(google._DrivePdfExtractionTimeout):
        google._extract_pdf_text(
            type("Reader", (), {"pages": [SlowPage(), SlowPage()]})(),
            deadline=time.monotonic() + 0.005,
        )


def test_pdf_parser_timeout_preserves_an_explicit_unavailable_marker(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    class Response:
        def __enter__(self) -> Response:
            return self

        def __exit__(self, *_args: object) -> None:
            return None

        def iter_bytes(self):
            yield b"complete-pdf-body"

    class Session:
        def stream(self, *_args: object, **_kwargs: object) -> Response:
            return Response()

    monkeypatch.setitem(
        sys.modules,
        "pypdf",
        types.SimpleNamespace(PdfReader=lambda *_args, **_kwargs: object()),
    )

    def timeout(*_args: object, **_kwargs: object) -> str:
        raise google._DrivePdfExtractionTimeout("test timeout")

    monkeypatch.setattr(google, "_extract_pdf_text", timeout)
    result = google._drive_content(Session(), {"id": "pdf", "mimeType": "application/pdf"})

    assert result == google.PDF_NO_TEXT_MARKER
    assert result.truncated is True


def test_google_gmail_detail_concurrency_is_four() -> None:
    assert google.GMAIL_DETAIL_CONCURRENCY == 4


def test_google_session_retries_transient_get_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    attempts = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            raise httpx.ReadTimeout("transient read timeout", request=request)
        return response({"ok": True}, request=request)

    monkeypatch.setattr(google.time, "sleep", lambda _seconds: None)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session:
        result = session.request("GET", "https://example.test/retry")

    assert result.json() == {"ok": True}
    assert attempts == 2


def test_google_session_retries_transient_403_with_rate_limit_reason(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    attempts = 0
    delays: list[float] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            return response(
                {"error": {"errors": [{"reason": "rateLimitExceeded"}]}, "code": 403},
                status=403,
                request=request,
            )
        return response({"ok": True}, request=request)

    monkeypatch.setattr(google.time, "sleep", delays.append)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session:
        result = session.request("GET", "https://example.test/retry")

    assert result.json() == {"ok": True}
    assert attempts == 2
    assert delays == [0.25]


def test_google_session_does_not_retry_permanent_403(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    attempts = 0
    delays: list[float] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal attempts
        attempts += 1
        return response(
            {"error": {"errors": [{"reason": "forbidden"}]}, "code": 403},
            status=403,
            request=request,
        )

    monkeypatch.setattr(google.time, "sleep", delays.append)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    with GoogleSession(token, client) as session, pytest.raises(httpx.HTTPStatusError):
        session.request("GET", "https://example.test/retry")

    assert attempts == 1
    assert delays == []


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
    write_token(token, '{"token":"access"}')

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
    # Bounded trials tolerate the unsupported image/png file (no content);
    # complete runs fail closed on unsupported content.
    documents = list(fetch_drive(token, "work", client=client, max_documents=10))

    assert len(documents) == 1
    assert documents[0].content == "Quarterly roadmap"
    assert documents[0].metadata["owners"] == ["Ada"]


def test_google_drive_ignores_folder_containers_in_strict_mode(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "folder1",
                            "name": "Holtz",
                            "mimeType": google.GOOGLE_DRIVE_FOLDER_MIME_TYPE,
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                        {
                            "id": "shortcut-folder1",
                            "name": "Pink Binder",
                            "mimeType": google.GOOGLE_DRIVE_SHORTCUT_MIME_TYPE,
                            "shortcutDetails": {
                                "targetId": "folder2",
                                "targetMimeType": google.GOOGLE_DRIVE_FOLDER_MIME_TYPE,
                            },
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="Quarterly roadmap", request=request)

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert [document.source_id for document in documents] == ["doc1"]


def test_google_drive_bounded_validation_caps_listing_page_and_documents(
    tmp_path: Path,
) -> None:
    token = tmp_path / "token.json"
    write_token(
        token,
        '{"token":"access","refresh_token":"refresh","client_id":"client",'
        '"token_uri":"https://oauth2.googleapis.com/token"}',
    )
    files = [
        {
            "id": f"doc{index}",
            "name": f"Document {index}",
            "mimeType": "text/plain",
            "modifiedTime": "2026-07-29T12:00:00Z",
        }
        for index in range(3)
    ]
    listing_page_sizes: list[str] = []
    content_requests: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            page_size = request.url.params.get("pageSize", "")
            listing_page_sizes.append(page_size)
            size = int(page_size)
            payload: dict[str, Any] = {"files": files[:size]}
            if size < len(files):
                payload["nextPageToken"] = "next"
            return response(payload, request=request)
        content_requests.append(request.url.path)
        return httpx.Response(200, text="bounded content", request=request)

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
            max_documents=2,
        )
    )

    assert [document.source_id for document in documents] == ["doc0", "doc1"]
    assert listing_page_sizes == ["2"]
    assert len(content_requests) == 2


def test_google_drive_downloads_are_batched_before_first_document(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    files = [
        {
            "id": f"doc{index}",
            "name": f"Document {index}",
            "mimeType": "text/plain",
            "modifiedTime": "2026-07-29T12:00:00Z",
        }
        for index in range(65)
    ]
    started: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        assert request.url.path == "/drive/v3/files"
        return response({"files": files}, request=request)

    def safe_content(_session: GoogleSession, item: dict[str, Any]) -> tuple[str, str | None]:
        started.append(str(item["id"]))
        return "bounded body", None

    monkeypatch.setattr(google, "_safe_drive_content", safe_content)
    documents = fetch_drive(
        token,
        "work",
        client=httpx.Client(transport=httpx.MockTransport(handler)),
    )

    first = next(documents)
    assert first.source_id == "doc0"
    assert len(started) == google.DRIVE_BATCH_SIZE
    remaining = list(documents)
    assert len(remaining) == len(files) - 1
    assert len(started) == len(files)


def test_google_drive_streams_large_text_with_bounded_memory_and_metadata(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    body = "header\n" + ("middle-row\n" * (google.MAX_DRIVE_STREAM_CHARS // 8 + 1000)) + "final-row"

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "large",
                            "name": "Large text",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text=body, request=request)

    cache = tmp_path / "cache"
    first = next(
        iter(
            fetch_drive(
                token,
                "work",
                client=httpx.Client(transport=httpx.MockTransport(handler)),
                cache_dir=cache,
                max_content_chars=1000,
            )
        )
    )
    second = next(
        iter(
            fetch_drive(
                token,
                "work",
                client=httpx.Client(transport=httpx.MockTransport(handler)),
                cache_dir=cache,
                max_content_chars=1000,
            )
        )
    )

    for document in (first, second):
        assert len(document.content) <= 1000
        assert document.content.startswith("header")
        assert document.content.endswith("final-row")
        assert "Cortana omitted" in document.content
        assert document.metadata["content_truncated"] is True
        assert document.metadata["content_original_chars"] == len(body)
        serialized = json.loads(document.as_json())
        assert serialized["metadata"]["content_original_chars"] == len(body)


def test_google_drive_skips_malformed_listing_records(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

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
            max_documents=10,
        )
    )

    assert [document.source_id for document in documents] == ["valid"]
    diagnostic = capsys.readouterr().err
    assert "Drive file skipped: record=0 is not an object" in diagnostic
    assert "Drive file skipped: id=bad-time" in diagnostic


def test_google_drive_full_mode_rejects_malformed_listing_records(
    tmp_path: Path,
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response(
            {"files": [None, {"name": "missing id"}]},
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive file record=0 is not an object"):
        list(fetch_drive(token, "work", client=client))


def test_google_drive_full_mode_rejects_non_object_listing(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response(["not", "an", "object"], request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive listing is not an object"):
        list(fetch_drive(token, "work", client=client))

    # A capped run still tolerates the malformed listing with a diagnostic.
    assert list(fetch_drive(token, "work", client=client, max_documents=5)) == []
    assert "Drive listing skipped" in capsys.readouterr().err


def test_google_drive_full_mode_rejects_incomplete_search(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Doc",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ],
                    "incompleteSearch": True,
                },
                request=request,
            )
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive listing is incomplete"):
        list(fetch_drive(token, "work", client=client))

    # Bounded validation runs keep tolerating incomplete searches.
    capped = list(fetch_drive(token, "work", client=client, max_documents=5))
    assert [document.source_id for document in capped] == ["doc1"]


def test_google_drive_full_mode_rejects_invalid_modified_time(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "bad-time",
                            "name": "Bad timestamp",
                            "mimeType": "text/plain",
                            "modifiedTime": "not-a-timestamp",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive file has invalid modifiedTime: id=bad-time"):
        list(fetch_drive(token, "work", client=client))


def test_google_drive_full_mode_rejects_missing_modified_time(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {"id": "missing-time", "name": "Missing time", "mimeType": "text/plain"}
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive file has no modifiedTime: id=missing-time"):
        list(fetch_drive(token, "work", client=client))


def test_google_drive_bounded_mode_skips_records_missing_modified_time(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    detail_requests = 0

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "good",
                            "name": "Good",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                        {"id": "missing", "name": "Missing Time", "mimeType": "text/plain"},
                    ]
                },
                request=request,
            )
        nonlocal detail_requests
        detail_requests += 1
        return httpx.Response(200, text="bounded body", request=request)

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
            cache_dir=cache,
            max_documents=10,
        )
    )

    assert [document.source_id for document in documents] == ["good"]
    assert detail_requests == 1
    assert "Drive file skipped: id=missing reason=missing modifiedTime" in capsys.readouterr().err
    connection = sqlite3.connect(cache / "drive.sqlite3")
    try:
        rows = connection.execute("SELECT id FROM files").fetchall()
    finally:
        connection.close()
    assert rows == [("good",)]


def test_google_drive_bounds_oversized_exports_with_explicit_metadata(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
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
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    content_requests = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal content_requests
        if request.url.path == "/drive/v3/changes/startPageToken":
            return response({"startPageToken": "100"}, request=request)
        if request.url.path == "/drive/v3/changes":
            return response({"changes": [], "newStartPageToken": "100"}, request=request)
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


def test_google_drive_applies_changes_delta_to_persisted_snapshot(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access","refresh_token":"refresh","client_id":"client"}')
    cache = tmp_path / "cache"
    listing_calls = 0
    changes_calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal listing_calls, changes_calls
        if request.url.path == "/drive/v3/changes/startPageToken":
            return response({"startPageToken": "100"}, request=request)
        if request.url.path == "/drive/v3/files":
            listing_calls += 1
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                        {
                            "id": "doc2",
                            "name": "Notes",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        },
                    ]
                },
                request=request,
            )
        if request.url.path == "/drive/v3/changes":
            changes_calls += 1
            assert request.url.params["pageToken"] == "100"
            return response(
                {
                    "changes": [
                        {
                            "fileId": "doc1",
                            "file": {
                                "id": "doc1",
                                "name": "Roadmap updated",
                                "mimeType": "text/plain",
                                "modifiedTime": "2026-07-29T13:00:00Z",
                            },
                        },
                        {"fileId": "doc2", "removed": True},
                    ],
                    "newStartPageToken": "120",
                },
                request=request,
            )
        file_id = request.url.path.rsplit("/", 1)[-1]
        return httpx.Response(200, text=f"body-{file_id}", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_drive(token, "work", client=client, cache_dir=cache))
    second = list(fetch_drive(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in first] == ["doc1", "doc2"]
    assert [document.source_id for document in second] == ["doc1"]
    assert second[0].title == "Roadmap updated"
    assert second[0].content == "body-doc1"
    assert listing_calls == 1
    assert changes_calls == 1
    connection = sqlite3.connect(cache / "drive.sqlite3")
    try:
        assert connection.execute("SELECT page_token FROM sync_state").fetchone() == ("120",)
    finally:
        connection.close()


def test_google_drive_rebuilds_when_changes_cursor_expires(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access","refresh_token":"refresh","client_id":"client"}')
    cache = tmp_path / "cache"
    listing_calls = 0
    change_attempts = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal listing_calls, change_attempts
        if request.url.path == "/drive/v3/changes/startPageToken":
            return response(
                {"startPageToken": "100" if listing_calls == 0 else "200"}, request=request
            )
        if request.url.path == "/drive/v3/files":
            listing_calls += 1
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Rebuilt",
                            "mimeType": "text/plain",
                            "modifiedTime": f"2026-07-29T{11 + listing_calls:02d}:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        if request.url.path == "/drive/v3/changes":
            change_attempts += 1
            return httpx.Response(410, json={"error": "expired"}, request=request)
        return httpx.Response(200, text="rebuilt body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    list(fetch_drive(token, "work", client=client, cache_dir=cache))
    rebuilt = list(fetch_drive(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in rebuilt] == ["doc1"]
    assert rebuilt[0].content == "rebuilt body"
    assert listing_calls == 2
    assert change_attempts == 1
    connection = sqlite3.connect(cache / "drive.sqlite3")
    try:
        assert connection.execute("SELECT page_token FROM sync_state").fetchone() == ("200",)
    finally:
        connection.close()


def test_google_drive_invalidates_cursor_when_account_scope_changes(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access","refresh_token":"account-one","client_id":"client"}')
    cache = tmp_path / "cache"
    listing_calls = 0
    changes_calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal listing_calls, changes_calls
        if request.url.path == "/drive/v3/changes/startPageToken":
            return response(
                {"startPageToken": "100" if listing_calls == 0 else "200"}, request=request
            )
        if request.url.path == "/drive/v3/changes":
            changes_calls += 1
            return response({"changes": [], "newStartPageToken": "101"}, request=request)
        if request.url.path == "/drive/v3/files":
            listing_calls += 1
            return response(
                {
                    "files": [
                        {
                            "id": "same-id",
                            "name": "Account one" if listing_calls == 1 else "Account two",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="account body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_drive(token, "work", client=client, cache_dir=cache))
    write_token(token, '{"token":"access","refresh_token":"account-two","client_id":"client"}')
    second = list(fetch_drive(token, "work", client=client, cache_dir=cache))

    assert first[0].title == "Account one"
    assert second[0].title == "Account two"
    assert listing_calls == 2
    assert changes_calls == 0


def test_google_drive_failed_delta_does_not_advance_cursor(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access","refresh_token":"refresh","client_id":"client"}')
    cache = tmp_path / "cache"
    fail_delta = False

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal fail_delta
        if request.url.path == "/drive/v3/changes/startPageToken":
            return response({"startPageToken": "100"}, request=request)
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        if request.url.path == "/drive/v3/changes":
            return response(
                {
                    "changes": [
                        {
                            "fileId": "doc1",
                            "file": {
                                "id": "doc1",
                                "name": "Changed",
                                "mimeType": "text/plain",
                                "modifiedTime": "2026-07-29T13:00:00Z",
                            },
                        }
                    ],
                    "newStartPageToken": "120",
                },
                request=request,
            )
        if fail_delta:
            return httpx.Response(503, request=request)
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    list(fetch_drive(token, "work", client=client, cache_dir=cache))
    fail_delta = True
    with pytest.raises(RuntimeError, match="refusing cursor advance"):
        list(fetch_drive(token, "work", client=client, cache_dir=cache))

    connection = sqlite3.connect(cache / "drive.sqlite3")
    try:
        assert connection.execute("SELECT page_token FROM sync_state").fetchone() == ("100",)
    finally:
        connection.close()


def test_google_drive_capped_run_does_not_prune_cached_bodies(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    content_requests = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal content_requests
        if request.url.path == "/drive/v3/changes/startPageToken":
            return response({"startPageToken": "100"}, request=request)
        if request.url.path == "/drive/v3/files":
            page_size = int(request.url.params.get("pageSize") or 1000)
            return response(
                {
                    "files": [
                        {
                            "id": f"doc{index}",
                            "name": f"Doc {index}",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                        for index in (1, 2)
                    ][:page_size]
                },
                request=request,
            )
        content_requests += 1
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    full = list(fetch_drive(token, "work", client=client, cache_dir=cache))
    capped = list(fetch_drive(token, "work", client=client, cache_dir=cache, max_documents=1))

    assert [document.source_id for document in full] == ["doc1", "doc2"]
    assert [document.source_id for document in capped] == ["doc1"]
    assert content_requests == 2, "the capped run must reuse cached bodies"
    connection = sqlite3.connect(cache / "drive.sqlite3")
    try:
        bodies = connection.execute("SELECT id FROM files ORDER BY id").fetchall()
        cursor = connection.execute("SELECT page_token FROM sync_state").fetchone()
    finally:
        connection.close()
    assert bodies == [("doc1",), ("doc2",)], "a capped run must not prune cached bodies"
    assert cursor == ("100",), "a capped run must not advance the Drive cursor"


def test_google_drive_bounded_run_commits_new_cache_content(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    fail = False

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Roadmap",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="body", request=request)

    def content(_session: GoogleSession, _item: dict[str, Any]) -> str:
        if fail:
            raise ValueError("detail unavailable")
        return "bounded cache body"

    monkeypatch.setattr(google, "_drive_content", content)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_drive(token, "work", client=client, cache_dir=cache, max_documents=1))
    fail = True
    second = list(fetch_drive(token, "work", client=client, cache_dir=cache, max_documents=1))

    assert first[0].content == "bounded cache body"
    assert second[0].content == first[0].content


def test_google_drive_cache_preserves_unknown_pdf_character_count(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "large-pdf",
                            "name": "Large PDF",
                            "mimeType": "application/pdf",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="unused", request=request)

    monkeypatch.setattr(
        google,
        "_drive_content",
        lambda _session, _item: google._DriveContent("head\n[marker]\ntail", None, True),
    )
    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_drive(token, "work", client=client, cache_dir=cache, max_documents=1))
    second = list(fetch_drive(token, "work", client=client, cache_dir=cache, max_documents=1))

    assert first[0].metadata["content_original_chars"] is None
    assert second[0].metadata["content_original_chars"] is None
    connection = sqlite3.connect(cache / "drive.sqlite3")
    try:
        assert connection.execute(
            "SELECT original_chars,truncated FROM files WHERE id='large-pdf'"
        ).fetchone() == (google.UNKNOWN_CONTENT_CHARS, 1)
    finally:
        connection.close()


def test_google_drive_full_mode_rejects_unresolved_content(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

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
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="never reached", request=request)

    def content(_session: GoogleSession, _item: dict[str, Any]) -> str:
        raise ValueError("sensitive provider detail")

    monkeypatch.setattr(google, "_drive_content", content)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive file content unavailable: id=doc1") as error:
        list(fetch_drive(token, "work", client=client))
    assert "sensitive provider detail" not in str(error.value)

    # Bounded trials keep the diagnostic skip for unresolved content.
    capped = list(fetch_drive(token, "work", client=client, max_documents=5))
    assert capped == []
    assert "drive file content unavailable: id=doc1 error=ValueError" in capsys.readouterr().err


def test_google_drive_full_mode_uses_stale_cached_content(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
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


def test_google_drive_full_mode_preserves_metadata_for_binary_content(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "bin1",
                            "name": "Image",
                            "mimeType": "image/png",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ]
                },
                request=request,
            )
        return httpx.Response(200, text="unused", request=request)

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
        )
    )

    assert len(documents) == 1
    assert documents[0].content == google.DRIVE_NO_TEXT_MARKER
    assert documents[0].metadata["content_unavailable"] is True
    assert documents[0].metadata["content_truncated"] is True


def test_google_drive_rejects_oversized_pdf_from_listing_metadata() -> None:
    item = {
        "id": "huge-pdf",
        "mimeType": "application/pdf",
        "size": str(google.MAX_DRIVE_PDF_BYTES + 1),
    }

    with pytest.raises(
        RuntimeError,
        match=f"Drive PDF exceeds the {google.MAX_DRIVE_PDF_BYTES} byte safety limit",
    ):
        google._drive_content(None, item)  # type: ignore[arg-type]


def test_google_drive_full_mode_rejects_falsey_next_page_token(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Doc",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ],
                    "nextPageToken": "",
                },
                request=request,
            )
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive listing has invalid nextPageToken"):
        list(fetch_drive(token, "work", client=client, cache_dir=tmp_path / "cache"))


def test_google_drive_full_mode_rejects_invalid_next_page_token(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": "doc1",
                            "name": "Doc",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                    ],
                    "nextPageToken": 123,
                },
                request=request,
            )
        return httpx.Response(200, text="body", request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive listing has invalid nextPageToken"):
        list(fetch_drive(token, "work", client=client, cache_dir=tmp_path / "cache"))


def test_google_drive_bounded_mode_stops_on_invalid_next_page_token(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/drive/v3/files":
            return response(
                {
                    "files": [
                        {
                            "id": f"doc{index}",
                            "name": f"Doc {index}",
                            "mimeType": "text/plain",
                            "modifiedTime": "2026-07-29T12:00:00Z",
                        }
                        for index in range(2)
                    ],
                    "nextPageToken": 123,
                },
                request=request,
            )
        return httpx.Response(200, text="content", request=request)

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
            cache_dir=tmp_path / "cache",
            max_documents=10,
        )
    )

    assert [document.source_id for document in documents] == ["doc0", "doc1"]
    assert (
        "Drive listing skipped: nextPageToken is not a non-empty string" in capsys.readouterr().err
    )


def test_google_drive_bounded_mode_stops_on_falsey_next_page_token(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response(
            {
                "files": [
                    {
                        "id": "doc",
                        "name": "Doc",
                        "mimeType": "text/plain",
                        "modifiedTime": "2026-07-29T12:00:00Z",
                    }
                ],
                "nextPageToken": "",
            },
            request=request,
        )

    documents = list(
        fetch_drive(
            token,
            "work",
            client=httpx.Client(transport=httpx.MockTransport(handler)),
            cache_dir=tmp_path / "cache",
            max_documents=10,
        )
    )

    assert [document.source_id for document in documents] == ["doc"]
    assert (
        "Drive listing skipped: nextPageToken is not a non-empty string" in capsys.readouterr().err
    )


def test_google_gmail_decodes_message_body(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
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


def test_google_gmail_caps_listing_page_and_documents(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    listing_limits: list[str] = []
    detail_requests: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            listing_limits.append(request.url.params.get("maxResults", ""))
            return response(
                {"messages": [{"id": "m1"}, {"id": "m2"}]},
                request=request,
            )
        detail_requests.append(request.url.path)
        message_id = request.url.path.rsplit("/", 1)[-1]
        return response(
            {
                "id": message_id,
                "payload": {
                    "headers": [{"name": "Subject", "value": message_id}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(message_id.encode()).decode(),
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
            max_documents=1,
        )
    )

    assert [document.source_id for document in documents] == ["m1"]
    assert listing_limits == ["1"]
    assert detail_requests == ["/gmail/v1/users/me/messages/m1"]


def test_google_gmail_skips_malformed_listing_records(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

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
            max_documents=10,
        )
    )

    assert [document.source_id for document in documents] == ["m1"]
    assert "Gmail message skipped: record=0 is not an object" in capsys.readouterr().err


def test_google_gmail_full_mode_rejects_malformed_listing_records(
    tmp_path: Path,
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response(
            {"messages": [None, {"labelIds": ["INBOX"]}]},
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail message record=0 is not an object"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_full_mode_rejects_detail_id_mismatch(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        return response(
            {
                "id": "other-id",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Mismatch"}],
                    "mimeType": "text/plain",
                    "body": {"data": base64.urlsafe_b64encode(b"body").decode()},
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail message detail id mismatch: requested=m1"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_full_mode_rejects_cached_detail_id_mismatch(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    cache.mkdir()

    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        connection.execute(
            "CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,body TEXT NOT NULL)"
        )
        connection.execute(
            "INSERT INTO messages(id,body) VALUES(?,?)",
            (
                "m1",
                '{"id":"other-id","payload":{"headers":[{"name":"Subject","value":"cached"}]}}',
            ),
        )
        connection.commit()
    finally:
        connection.close()

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        return response(
            {
                "id": "m1",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Recovered"}],
                    "mimeType": "text/plain",
                    "body": {"data": base64.urlsafe_b64encode(b"recovered").decode()},
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(
        fetch_gmail(
            token,
            "work",
            client=client,
            cache_dir=cache,
        )
    )

    assert [document.source_id for document in documents] == ["m1"]
    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        cached = connection.execute("SELECT body FROM messages WHERE id='m1'").fetchone()
    finally:
        connection.close()
    assert cached is not None
    assert '"id":"m1"' in cached[0]


def test_google_gmail_bounded_mode_skips_cached_detail_id_mismatch(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    cache.mkdir()

    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        connection.execute(
            "CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,body TEXT NOT NULL)"
        )
        connection.execute(
            "INSERT INTO messages(id,body) VALUES(?,?)",
            (
                "m1",
                '{"id":"other-id","payload":{"headers":[{"name":"Subject","value":"cached"}]}}',
            ),
        )
        connection.commit()
    finally:
        connection.close()

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        return response({"error": "forbidden"}, status=403, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(
        fetch_gmail(
            token,
            "work",
            client=client,
            cache_dir=cache,
            max_documents=1,
        )
    )

    assert documents == []
    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        cached = connection.execute("SELECT body FROM messages WHERE id='m1'").fetchone()
    finally:
        connection.close()
    assert cached is None
    assert "Gmail message skipped: id=m1 reason=cached id mismatch" in capsys.readouterr().err


def test_google_gmail_full_mode_rejects_falsey_next_page_token(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response(
                {"messages": [], "nextPageToken": ""},
                request=request,
            )
        return response({"error": "unexpected"}, status=500, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail listing has invalid nextPageToken"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_full_mode_rejects_non_string_next_page_token(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [], "nextPageToken": 123}, request=request)
        return response({"error": "unexpected"}, status=500, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail listing has invalid nextPageToken"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_bounded_mode_stops_on_non_string_next_page_token(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response({"messages": [], "nextPageToken": 123}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_gmail(token, "work", client=client, max_documents=1))

    assert documents == []
    assert (
        "Gmail listing skipped: nextPageToken is not a non-empty string" in capsys.readouterr().err
    )


def test_google_gmail_bounded_mode_stops_on_falsey_next_page_token(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response({"messages": [], "nextPageToken": ""}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_gmail(token, "work", client=client, max_documents=1))

    assert documents == []
    assert (
        "Gmail listing skipped: nextPageToken is not a non-empty string" in capsys.readouterr().err
    )


def test_google_drive_full_mode_rejects_non_string_id_listings(
    tmp_path: Path,
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response({"files": [{"id": 123, "name": "bad-id"}]}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Drive file record=0 has a non-string id"):
        list(fetch_drive(token, "work", client=client))


def test_google_gmail_full_mode_rejects_non_string_id_records(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        return response({"messages": [{"id": 123}]}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail message record=0 has a non-string id"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_full_mode_rejects_conversion_failure(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        return response(
            {
                "id": "m1",
                "payload": {
                    "headers": [{"name": "Date", "value": "not-a-date"}],
                    "mimeType": "text/plain",
                    "body": {"data": base64.urlsafe_b64encode(b"body").decode()},
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail message conversion failed: id=m1"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_reuses_private_message_cache(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
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


def test_google_gmail_applies_history_delta_to_persisted_snapshot(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(
        token,
        json.dumps(
            {
                "token": "access",
                "refresh_token": "refresh",
                "client_id": "client",
                "token_uri": "https://oauth2.googleapis.com/token",
                "email": "owner@example.test",
            }
        ),
    )
    cache = tmp_path / "cache"
    phase = "full"
    list_requests = 0
    history_requests: list[str] = []

    def message(message_id: str, history_id: str, body: str) -> dict[str, Any]:
        return {
            "id": message_id,
            "historyId": history_id,
            "payload": {
                "headers": [{"name": "Subject", "value": message_id}],
                "mimeType": "text/plain",
                "body": {"data": base64.urlsafe_b64encode(body.encode()).decode()},
            },
        }

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal list_requests
        path = request.url.path
        if path.endswith("/messages"):
            list_requests += 1
            return response({"messages": [{"id": "m1"}, {"id": "m2"}]}, request=request)
        if path.endswith("/history"):
            history_requests.append(request.url.params["startHistoryId"])
            assert phase == "delta"
            return response(
                {
                    "historyId": "120",
                    "history": [
                        {"id": "110", "messagesDeleted": [{"message": {"id": "m1"}}]},
                        {"id": "115", "messagesAdded": [{"message": {"id": "m3"}}]},
                        {"id": "120", "labelsAdded": [{"message": {"id": "m2"}}]},
                    ],
                },
                request=request,
            )
        message_id = path.rsplit("/", 1)[-1]
        if phase == "full":
            return response(
                {
                    "m1": message("m1", "100", "one"),
                    "m2": message("m2", "105", "two"),
                }[message_id],
                request=request,
            )
        return response(
            {
                "m2": message("m2", "120", "two updated"),
                "m3": message("m3", "115", "three"),
            }[message_id],
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_gmail(token, "work", client=client, cache_dir=cache))
    phase = "delta"
    second = list(fetch_gmail(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in first] == ["m1", "m2"]
    assert [document.source_id for document in second] == ["m2", "m3"]
    assert second[0].content.endswith("two updated")
    assert list_requests == 1, "the delta run must not relist the mailbox"
    assert history_requests == ["100"], "the cursor must start at the oldest cached history id"
    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        cursor = connection.execute("SELECT history_id FROM sync_state").fetchone()
        cached_ids = connection.execute("SELECT id FROM messages ORDER BY id").fetchall()
    finally:
        connection.close()
    assert cursor == ("120",)
    assert cached_ids == [("m2",), ("m3",)]


def test_google_gmail_rebuilds_when_history_cursor_expires(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access","email":"owner@example.test"}')
    cache = tmp_path / "cache"
    phase = "full"
    list_requests = 0
    history_requests = 0

    def message(message_id: str, history_id: str) -> dict[str, Any]:
        return {
            "id": message_id,
            "historyId": history_id,
            "payload": {
                "headers": [{"name": "Subject", "value": message_id}],
                "mimeType": "text/plain",
                "body": {
                    "data": base64.urlsafe_b64encode(message_id.encode()).decode(),
                },
            },
        }

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal history_requests, list_requests
        path = request.url.path
        if path.endswith("/history"):
            history_requests += 1
            assert phase == "expired"
            return response({"error": "historyExpired"}, status=404, request=request)
        if path.endswith("/messages"):
            list_requests += 1
            message_id = "m1" if phase == "full" else "m2"
            return response({"messages": [{"id": message_id}]}, request=request)
        message_id = path.rsplit("/", 1)[-1]
        return response(
            message(message_id, "100" if message_id == "m1" else "200"), request=request
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_gmail(token, "work", client=client, cache_dir=cache))
    phase = "expired"
    second = list(fetch_gmail(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in first] == ["m1"]
    assert [document.source_id for document in second] == ["m2"]
    assert history_requests == 1
    assert list_requests == 2, "expired history must fall back to a full listing"
    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        cached_ids = connection.execute("SELECT id FROM messages").fetchall()
        cursor = connection.execute("SELECT history_id FROM sync_state").fetchone()
    finally:
        connection.close()
    assert cached_ids == [("m2",)]
    assert cursor == ("200",)


def test_google_gmail_invalidates_cursor_when_account_scope_changes(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access","email":"first@example.test"}')
    cache = tmp_path / "cache"
    phase = "first"
    list_requests = 0
    history_requests = 0

    def message(message_id: str) -> dict[str, Any]:
        return {
            "id": message_id,
            "historyId": "100",
            "payload": {
                "headers": [{"name": "Subject", "value": message_id}],
                "mimeType": "text/plain",
                "body": {"data": base64.urlsafe_b64encode(message_id.encode()).decode()},
            },
        }

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal history_requests, list_requests
        path = request.url.path
        if path.endswith("/history"):
            history_requests += 1
            return response({"historyId": "110", "history": []}, request=request)
        if path.endswith("/messages"):
            list_requests += 1
            message_id = "m1" if phase == "first" else "m2"
            return response({"messages": [{"id": message_id}]}, request=request)
        return response(message(path.rsplit("/", 1)[-1]), request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_gmail(token, "work", client=client, cache_dir=cache))
    write_token(token, '{"token":"access","email":"second@example.test"}')
    phase = "second"
    second = list(fetch_gmail(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in first] == ["m1"]
    assert [document.source_id for document in second] == ["m2"]
    assert list_requests == 2, "a changed account must rebuild from a listing"
    assert history_requests == 0, "a changed account must not reuse the old cursor"


def test_google_gmail_capped_run_does_not_prune_cached_messages(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    detail_requests = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal detail_requests
        if request.url.path.endswith("/messages"):
            maximum = int(request.url.params.get("maxResults") or 500)
            return response(
                {"messages": [{"id": "m1"}, {"id": "m2"}][:maximum]},
                request=request,
            )
        detail_requests += 1
        message_id = request.url.path.rsplit("/", 1)[-1]
        return response(
            {
                "id": message_id,
                "payload": {
                    "headers": [{"name": "Subject", "value": message_id}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(message_id.encode()).decode(),
                    },
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    full = list(fetch_gmail(token, "work", client=client, cache_dir=cache))
    capped = list(fetch_gmail(token, "work", client=client, cache_dir=cache, max_documents=1))

    assert [document.source_id for document in full] == ["m1", "m2"]
    assert [document.source_id for document in capped] == ["m1"]
    assert detail_requests == 2, "the capped run must reuse cached messages"
    connection = sqlite3.connect(cache / "gmail.sqlite3")
    try:
        messages = connection.execute("SELECT id FROM messages ORDER BY id").fetchall()
    finally:
        connection.close()
    assert messages == [("m1",), ("m2",)], "a capped run must not prune cached messages"


def test_google_gmail_bounded_run_commits_new_cache_content(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    fail = False

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/messages"):
            return response({"messages": [{"id": "m1"}]}, request=request)
        if fail:
            return response({"error": "forbidden"}, status=403, request=request)
        return response(
            {
                "id": "m1",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Cached"}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(b"bounded cache body").decode(),
                    },
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_gmail(token, "work", client=client, cache_dir=cache, max_documents=1))
    fail = True
    second = list(fetch_gmail(token, "work", client=client, cache_dir=cache, max_documents=1))

    assert first[0].content.endswith("bounded cache body")
    assert second[0].content == first[0].content


def test_google_gmail_retries_transient_bad_request_detail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    attempts = 0
    delays: list[float] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal attempts
        if request.url.path.endswith("/messages"):
            return response(
                {"messages": [{"id": "m1"}]},
                request=request,
            )
        attempts += 1
        if attempts <= 3:
            return response({"error": "transient"}, status=400, request=request)
        return response(
            {
                "id": "m1",
                "threadId": "t1",
                "internalDate": "1700000000000",
                "payload": {
                    "headers": [{"name": "Subject", "value": "Recovered"}],
                    "mimeType": "text/plain",
                    "body": {
                        "data": base64.urlsafe_b64encode(b"Recovered body").decode(),
                    },
                },
            },
            request=request,
        )

    monkeypatch.setattr(google.time, "sleep", delays.append)
    client = httpx.Client(transport=httpx.MockTransport(handler))
    documents = list(fetch_gmail(token, "work", client=client, max_documents=1))

    assert [document.source_id for document in documents] == ["m1"]
    assert documents[0].title == "Recovered"
    assert attempts == 4
    assert delays == [0.25, 0.75, 1.5]


def test_google_gmail_skips_isolated_inaccessible_message(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

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
    # Bounded trials tolerate an isolated denied detail; complete runs fail closed.
    documents = list(fetch_gmail(token, "work", client=client, max_documents=10))

    assert [document.source_id for document in documents] == ["available"]
    assert "gmail message skipped: id=denied status=403" in capsys.readouterr().err


def test_google_gmail_full_mode_rejects_isolated_detail_denial(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

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
                    "body": {"data": base64.urlsafe_b64encode(b"Still indexed").decode()},
                },
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Gmail message detail unavailable: id=denied"):
        list(fetch_gmail(token, "work", client=client))


def test_google_gmail_refuses_broad_detail_denial(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
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
    write_token(token, '{"token":"access"}')

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


def test_google_calendar_reuses_persisted_sync_token_and_snapshot(
    tmp_path: Path,
) -> None:
    token = tmp_path / "token.json"
    write_token(
        token,
        '{"token":"access","refresh_token":"refresh","client_id":"client",'
        '"token_uri":"https://oauth2.googleapis.com/token"}',
    )
    cache = tmp_path / "cache"
    phase = 0
    event_requests: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal phase
        if request.url.path.endswith("/calendarList"):
            return response(
                {"items": [{"id": "primary", "summary": "Work"}]},
                request=request,
            )
        event_requests.append(request)
        if phase == 0:
            return response(
                {
                    "items": [
                        {
                            "id": "event-1",
                            "summary": "Keep this event",
                            "start": {"dateTime": "2026-07-29T12:00:00Z"},
                            "end": {"dateTime": "2026-07-29T12:30:00Z"},
                            "updated": "2026-07-29T11:00:00Z",
                        },
                        {
                            "id": "event-2",
                            "summary": "Remove this event",
                            "start": {"dateTime": "2026-07-29T13:00:00Z"},
                            "end": {"dateTime": "2026-07-29T13:30:00Z"},
                            "updated": "2026-07-29T11:00:00Z",
                        },
                    ],
                    "nextSyncToken": "sync-1",
                },
                request=request,
            )
        assert request.url.params.get("syncToken") == "sync-1"
        assert request.url.params.get("showDeleted") == "true"
        return response(
            {
                "items": [
                    {"id": "event-2", "status": "cancelled"},
                    {
                        "id": "event-3",
                        "summary": "New event",
                        "start": {"dateTime": "2026-07-30T12:00:00Z"},
                        "end": {"dateTime": "2026-07-30T12:30:00Z"},
                        "updated": "2026-07-30T11:00:00Z",
                    },
                ],
                "nextSyncToken": "sync-2",
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    first = list(fetch_calendar(token, "work", client=client, cache_dir=cache))
    # A normal OAuth refresh changes only the short-lived access token. The
    # durable calendar cursor must survive that rotation.
    write_token(
        token,
        '{"token":"rotated-access","refresh_token":"refresh","client_id":"client",'
        '"token_uri":"https://oauth2.googleapis.com/token"}',
    )
    phase = 1
    second = list(fetch_calendar(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in first] == [
        "primary:event-1",
        "primary:event-2",
    ]
    assert [document.source_id for document in second] == [
        "primary:event-1",
        "primary:event-3",
    ]
    assert len(event_requests) == 2
    assert "syncToken" not in event_requests[0].url.params
    assert event_requests[1].url.params["syncToken"] == "sync-1"


def test_google_calendar_rebuilds_when_sync_token_expires(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    cache = tmp_path / "cache"
    phase = 0
    event_requests: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal phase
        if request.url.path.endswith("/calendarList"):
            return response(
                {"items": [{"id": "primary", "summary": "Work"}]},
                request=request,
            )
        event_requests.append(request)
        if phase == 0:
            return response(
                {
                    "items": [
                        {
                            "id": "old",
                            "summary": "Old",
                            "start": {"dateTime": "2026-07-29T12:00:00Z"},
                            "end": {"dateTime": "2026-07-29T12:30:00Z"},
                            "updated": "2026-07-29T11:00:00Z",
                        }
                    ],
                    "nextSyncToken": "expired-token",
                },
                request=request,
            )
        if phase == 1:
            phase = 2
            return response({"error": "sync token expired"}, status=410, request=request)
        assert "syncToken" not in request.url.params
        return response(
            {
                "items": [
                    {
                        "id": "replacement",
                        "summary": "Replacement",
                        "start": {"dateTime": "2026-07-30T12:00:00Z"},
                        "end": {"dateTime": "2026-07-30T12:30:00Z"},
                        "updated": "2026-07-30T11:00:00Z",
                    }
                ],
                "nextSyncToken": "fresh-token",
            },
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    list(fetch_calendar(token, "work", client=client, cache_dir=cache))
    phase = 1
    rebuilt = list(fetch_calendar(token, "work", client=client, cache_dir=cache))

    assert [document.source_id for document in rebuilt] == ["primary:replacement"]
    assert event_requests[1].url.params["syncToken"] == "expired-token"
    assert "syncToken" not in event_requests[2].url.params


def test_google_calendar_caps_listing_page_and_documents(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')
    event_limits: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/calendarList"):
            return response({"items": [{"id": "primary", "summary": "Work"}]}, request=request)
        event_limits.append(request.url.params.get("maxResults", ""))
        return response(
            {
                "items": [
                    {
                        "id": "event-1",
                        "summary": "First",
                        "start": {"dateTime": "2026-07-29T12:00:00Z"},
                        "end": {"dateTime": "2026-07-29T12:30:00Z"},
                        "updated": "2026-07-29T11:00:00Z",
                    },
                    {
                        "id": "event-2",
                        "summary": "Second",
                        "start": {"dateTime": "2026-07-29T13:00:00Z"},
                        "end": {"dateTime": "2026-07-29T13:30:00Z"},
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
            max_documents=1,
        )
    )

    assert [document.source_id for document in documents] == ["primary:event-1"]
    assert event_limits == ["1"]


def test_google_calendar_collapses_recurring_occurrences(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

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
    write_token(token, '{"token":"access"}')

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
            max_documents=10,
        )
    )

    assert [document.source_id for document in documents] == ["primary:valid"]
    assert "Calendar event skipped: id=broken" in capsys.readouterr().err


def test_google_calendar_full_mode_rejects_malformed_events(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/calendarList"):
            return response({"items": [{"id": "primary", "summary": "Work"}]}, request=request)
        return response(
            {"items": [{"id": "broken", "start": "not-an-object"}]},
            request=request,
        )

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Calendar event conversion failed: id=broken"):
        list(fetch_calendar(token, "work", client=client))


def test_google_calendar_full_mode_rejects_invalid_next_page_token(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, '{"token":"access"}')

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/calendarList"):
            return response({"items": [{"id": "primary"}]}, request=request)
        return response({"items": [], "nextPageToken": 123}, request=request)

    client = httpx.Client(transport=httpx.MockTransport(handler))
    with pytest.raises(RuntimeError, match="Calendar events have invalid nextPageToken"):
        list(fetch_calendar(token, "work", client=client))


def test_google_session_refreshes_and_secures_token_file(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(
        token,
        json.dumps(
            {
                "refresh_token": "refresh",
                "client_id": "client",
                "client_secret": "secret",
                "token_uri": "https://oauth2.googleapis.com/token",
            }
        ),
    )

    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.host == "oauth2.googleapis.com":
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
    write_token(
        token,
        json.dumps(
            {
                "token": "expired",
                "refresh_token": "refresh",
                "client_id": "client",
                "client_secret": "secret",
            }
        ),
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
    write_token(
        token,
        json.dumps({"refresh_token": "refresh", "client_id": "desktop-client"}),
    )

    def handler(request: httpx.Request) -> httpx.Response:
        assert b"client_secret" not in request.content
        return response({"access_token": "fresh"}, request=request)

    with GoogleSession(
        token,
        httpx.Client(transport=httpx.MockTransport(handler)),
    ) as session:
        assert session._access_token() == "fresh"


@pytest.mark.parametrize("code", ["invalid_grant", "invalid_client"])
def test_google_session_reports_sanitized_refresh_error_code(tmp_path: Path, code: str) -> None:
    token = tmp_path / "token.json"
    write_token(
        token,
        json.dumps(
            {
                "refresh_token": "refresh-token-secret",
                "client_id": "client-id-secret",
            }
        ),
    )

    def handler(request: httpx.Request) -> httpx.Response:
        return response(
            {
                "error": code,
                "error_description": "do not expose this provider detail or token",
            },
            status=400,
            request=request,
        )

    with (
        GoogleSession(token, httpx.Client(transport=httpx.MockTransport(handler))) as session,
        pytest.raises(RuntimeError, match=rf"Google OAuth refresh failed \(400: {code}\)") as error,
    ):
        session._access_token()

    message = str(error.value)
    assert "refresh-token-secret" not in message
    assert "client-id-secret" not in message
    assert "do not expose" not in message
    if code == "invalid_grant":
        assert "reauthorize the Google source" in message


def test_google_session_keeps_unknown_refresh_error_generic(tmp_path: Path) -> None:
    token = tmp_path / "token.json"
    write_token(token, json.dumps({"refresh_token": "refresh", "client_id": "client"}))

    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            400,
            content=b'{"error":"provider-specific-secret","error_description":"private"}',
            request=request,
        )

    with (
        GoogleSession(token, httpx.Client(transport=httpx.MockTransport(handler))) as session,
        pytest.raises(RuntimeError, match=r"Google OAuth refresh failed \(400\)$") as error,
    ):
        session._access_token()

    assert "provider-specific-secret" not in str(error.value)
    assert "private" not in str(error.value)


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
    write_token(token, "{}")
    with (
        GoogleSession(token, httpx.Client()) as session,
        pytest.raises(RuntimeError, match="missing refresh_token"),
    ):
        session.request("GET", "https://api.test/data")


def test_google_session_rejects_invalid_json_and_non_google_refresh_uri(tmp_path: Path) -> None:
    invalid = tmp_path / "invalid.json"
    write_token(invalid, "not-json")
    with pytest.raises(RuntimeError, match="not valid JSON"):
        GoogleSession(invalid, httpx.Client())

    token = tmp_path / "external.json"
    write_token(
        token,
        json.dumps(
            {
                "refresh_token": "refresh",
                "client_id": "client",
                "token_uri": "https://attacker.example/token",
            }
        ),
    )
    with (
        GoogleSession(token, httpx.Client()) as session,
        pytest.raises(RuntimeError, match="HTTPS Google OAuth"),
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


def test_connector_cli_applies_bounded_document_cap(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    documents = [
        Document(source="buzz", source_id=str(index), title="Event", content="body")
        for index in range(3)
    ]
    monkeypatch.setattr(connector_cli, "fetch_buzz", lambda *_args: documents)

    assert connector_cli.main(["--max-documents", "2", "buzz"]) == 0

    captured = capsys.readouterr()
    assert len(captured.out.splitlines()) == 2
    assert "emitted=2" in captured.err


def test_connector_cli_preserves_drive_cap_in_either_argument_position(
    tmp_path: Path,
) -> None:
    before = connector_cli.parser().parse_args(
        ["--max-documents", "7", "google-drive", "--token", str(tmp_path / "token.json")]
    )
    after = connector_cli.parser().parse_args(
        ["google-drive", "--token", str(tmp_path / "token.json"), "--max-documents", "9"]
    )

    assert before.max_documents == 7
    assert after.max_documents == 9


def test_connector_cli_rejects_non_positive_document_cap() -> None:
    with pytest.raises(RuntimeError, match="greater than zero"):
        connector_cli.main(["--max-documents", "0", "buzz"])


def test_connector_cli_reports_version(capsys: pytest.CaptureFixture[str]) -> None:
    assert connector_cli.main(["--version"]) == 0
    output = capsys.readouterr()
    assert output.out.strip() == __version__
    assert output.err == ""


def test_connector_cli_dispatches_chat_sources(monkeypatch: pytest.MonkeyPatch) -> None:
    expected = [Document(source="test", source_id="1", title="One", content="Body")]
    monkeypatch.setattr(connector_cli, "fetch_slack", lambda *_args, **_kwargs: expected)
    monkeypatch.setattr(connector_cli, "fetch_discord", lambda *_args, **_kwargs: expected)

    slack_args = connector_cli.parser().parse_args(
        ["--project", "work", "slack", "--channel", "C1"]
    )
    discord_args = connector_cli.parser().parse_args(
        [
            "--project",
            "work",
            "discord",
            "--channel",
            "D1",
            "--token",
            "/tmp/discord-token.json",
            "--oauth-client",
            "/tmp/discord-client.json",
        ]
    )
    assert list(connector_cli._documents(slack_args)) == expected
    assert list(connector_cli._documents(discord_args)) == expected


def test_connector_cli_requires_existing_google_token(tmp_path: Path) -> None:
    args = connector_cli.parser().parse_args(
        ["google-drive", "--token", str(tmp_path / "missing.json")]
    )
    with pytest.raises(RuntimeError, match="does not exist"):
        connector_cli._documents(args)


def test_connector_cli_requires_connector_when_not_reporting_version() -> None:
    with pytest.raises(RuntimeError, match="connector command is required"):
        connector_cli.main([])


def test_connector_entrypoint_reports_expected_failures(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def fail(_argv: list[str] | None = None) -> int:
        raise RuntimeError("expected failure")

    monkeypatch.setattr(connector_cli, "main", fail)

    assert connector_cli.entrypoint() == 1
    assert capsys.readouterr().err == "connector error: expected failure\n"
