from __future__ import annotations

import datetime as dt
import json
import os
import sqlite3
import time
from collections.abc import Iterable
from pathlib import Path
from typing import Any

import httpx

from .model import Document


def fetch_slack(
    channel_ids: list[str],
    project: str,
    token_env: str = "SLACK_BOT_TOKEN",
) -> Iterable[Document]:
    token = _required_env(token_env)
    headers = {"Authorization": f"Bearer {token}"}
    with httpx.Client(base_url="https://slack.com/api", headers=headers, timeout=30) as client:
        for channel_id in channel_ids:
            cursor = ""
            while True:
                response = _get_with_backoff(
                    client,
                    "/conversations.history",
                    params={"channel": channel_id, "limit": 200, "cursor": cursor},
                )
                payload = _slack_payload(response)
                for parent in payload.get("messages", []):
                    thread = [parent]
                    if int(parent.get("reply_count", 0)):
                        replies = _get_with_backoff(
                            client,
                            "/conversations.replies",
                            params={"channel": channel_id, "ts": parent["ts"], "limit": 200},
                        )
                        thread = _slack_payload(replies).get("messages", thread)
                    text = "\n".join(
                        f"{message.get('user', 'unknown')}: {message.get('text', '')}"
                        for message in thread
                        if message.get("text")
                    )
                    if text:
                        updated = max(float(message["ts"]) for message in thread)
                        yield Document(
                            source="slack",
                            source_id=f"{channel_id}:{parent['ts']}",
                            title=_title(parent.get("text", ""), f"Slack {channel_id}"),
                            content=text,
                            uri=f"slack://channel?team=&id={channel_id}&message={parent['ts']}",
                            updated_at=dt.datetime.fromtimestamp(updated, dt.UTC),
                            project=project,
                            metadata={
                                "channel_id": channel_id,
                                "participants": sorted(
                                    {m.get("user") for m in thread if m.get("user")}
                                ),
                                "message_count": len(thread),
                            },
                        )
                cursor = str(payload.get("response_metadata", {}).get("next_cursor") or "")
                if not cursor:
                    break


def fetch_discord(
    channel_ids: list[str],
    project: str,
    token_env: str = "DISCORD_BOT_TOKEN",
    cache_dir: Path | None = None,
) -> Iterable[Document]:
    token = _required_env(token_env)
    headers = {"Authorization": f"Bot {token}"}
    with httpx.Client(
        base_url="https://discord.com/api/v10", headers=headers, timeout=30
    ) as client:
        if cache_dir is not None:
            yield from _fetch_discord_cached(client, channel_ids, project, cache_dir)
            return
        for channel_id in channel_ids:
            before: str | None = None
            while True:
                params: dict[str, Any] = {"limit": 100}
                if before:
                    params["before"] = before
                response = _get_with_backoff(
                    client, f"/channels/{channel_id}/messages", params=params
                )
                response.raise_for_status()
                messages = response.json()
                if not messages:
                    break
                for message in messages:
                    document = _discord_document(message, channel_id, project)
                    if document is not None:
                        yield document
                before = str(messages[-1]["id"])
                if len(messages) < 100:
                    break


def _fetch_discord_cached(
    client: httpx.Client,
    channel_ids: list[str],
    project: str,
    cache_dir: Path,
) -> Iterable[Document]:
    cache = _discord_cache(cache_dir)
    try:
        for channel_id in channel_ids:
            row = cache.execute(
                "SELECT latest_id,last_full FROM channels WHERE channel_id=?",
                (channel_id,),
            ).fetchone()
            full = row is None or _full_refresh_due(str(row[1]))
            latest_id = None if row is None else str(row[0] or "")
            before: str | None = None
            after = None if full else latest_id
            while True:
                params: dict[str, Any] = {"limit": 100}
                if full and before:
                    params["before"] = before
                elif not full and after:
                    params["after"] = after
                response = _get_with_backoff(
                    client, f"/channels/{channel_id}/messages", params=params
                )
                response.raise_for_status()
                messages: list[dict[str, Any]] = response.json()
                if not messages:
                    break
                for message in messages:
                    message_id = str(message["id"])
                    cache.execute(
                        "INSERT OR REPLACE INTO discord_messages(id,channel_id,body) VALUES(?,?,?)",
                        (
                            message_id,
                            channel_id,
                            json.dumps(message, separators=(",", ":")),
                        ),
                    )
                    if full:
                        cache.execute(
                            "INSERT OR IGNORE INTO seen(channel_id,id) VALUES(?,?)",
                            (channel_id, message_id),
                        )
                    if not latest_id or int(message_id) > int(latest_id):
                        latest_id = message_id
                cache.commit()
                if len(messages) < 100:
                    break
                if full:
                    before = str(messages[-1]["id"])
                else:
                    next_after = max((str(message["id"]) for message in messages), key=int)
                    if next_after == after:
                        break
                    after = next_after
            if full:
                cache.execute(
                    "DELETE FROM discord_messages WHERE channel_id=? "
                    "AND id NOT IN (SELECT id FROM seen WHERE channel_id=?)",
                    (channel_id, channel_id),
                )
            cache.execute(
                "INSERT OR REPLACE INTO channels(channel_id,latest_id,last_full) VALUES(?,?,?)",
                (
                    channel_id,
                    latest_id,
                    dt.datetime.now(dt.UTC).isoformat() if full else str(row[1]),
                ),
            )
            cache.commit()

        rows = cache.execute(
            "SELECT channel_id,body FROM discord_messages ORDER BY CAST(id AS INTEGER)"
        )
        for channel_id, body in rows:
            cached_message: dict[str, Any] = json.loads(str(body))
            document = _discord_document(cached_message, str(channel_id), project)
            if document is not None:
                yield document
    finally:
        cache.close()


def _discord_cache(cache_dir: Path) -> sqlite3.Connection:
    cache_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    cache_dir.chmod(0o700)
    path = cache_dir / "discord.sqlite3"
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
    os.close(descriptor)
    path.chmod(0o600)
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA journal_mode=MEMORY")
    connection.execute("PRAGMA synchronous=NORMAL")
    connection.execute(
        "CREATE TABLE IF NOT EXISTS discord_messages("
        "id TEXT PRIMARY KEY,channel_id TEXT NOT NULL,body TEXT NOT NULL)"
    )
    connection.execute(
        "CREATE TABLE IF NOT EXISTS channels("
        "channel_id TEXT PRIMARY KEY,latest_id TEXT,last_full TEXT NOT NULL)"
    )
    connection.execute(
        "CREATE TEMP TABLE seen(channel_id TEXT NOT NULL,id TEXT NOT NULL,"
        "PRIMARY KEY(channel_id,id))"
    )
    return connection


def _full_refresh_due(last_full: str) -> bool:
    try:
        previous = dt.datetime.fromisoformat(last_full)
    except ValueError:
        return True
    return dt.datetime.now(dt.UTC) - previous.astimezone(dt.UTC) >= dt.timedelta(days=1)


def _discord_document(message: dict[str, Any], channel_id: str, project: str) -> Document | None:
    content = str(message.get("content") or "").strip()
    attachments = "\n".join(item.get("url", "") for item in message.get("attachments", []))
    body = "\n".join(part for part in [content, attachments] if part)
    if not body:
        return None
    return Document(
        source="discord",
        source_id=str(message["id"]),
        title=_title(content, f"Discord {channel_id}"),
        content=f"{message.get('author', {}).get('username', 'unknown')}: {body}",
        uri=f"https://discord.com/channels/@me/{channel_id}/{message['id']}",
        updated_at=dt.datetime.fromisoformat(str(message["timestamp"]).replace("Z", "+00:00")),
        project=project,
        metadata={
            "channel_id": channel_id,
            "author_id": message.get("author", {}).get("id"),
        },
    )


def _required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise RuntimeError(f"{name} is required")
    return value


def _get_with_backoff(
    client: httpx.Client,
    url: str,
    *,
    params: dict[str, Any],
    max_attempts: int = 8,
) -> httpx.Response:
    for attempt in range(max_attempts):
        response = client.get(url, params=params)
        if response.status_code not in {429, 500, 502, 503, 504}:
            _respect_rate_limit_headers(response)
            return response
        if attempt + 1 == max_attempts:
            return response
        time.sleep(_retry_after(response, attempt))
    raise AssertionError("retry loop must return")


def _retry_after(response: httpx.Response, attempt: int) -> float:
    header = response.headers.get("retry-after")
    if header:
        try:
            return min(max(float(header), 0.0), 60.0)
        except ValueError:
            pass
    try:
        payload = response.json()
        if isinstance(payload, dict) and payload.get("retry_after") is not None:
            return min(max(float(payload["retry_after"]), 0.0), 60.0)
    except (TypeError, ValueError):
        pass
    return min(float(2**attempt), 30.0)


def _respect_rate_limit_headers(response: httpx.Response) -> None:
    if response.headers.get("x-ratelimit-remaining") != "0":
        return
    reset_after = response.headers.get("x-ratelimit-reset-after")
    if not reset_after:
        return
    try:
        delay = min(max(float(reset_after), 0.0), 60.0)
    except ValueError:
        return
    if delay:
        time.sleep(delay)


def _slack_payload(response: httpx.Response) -> dict[str, Any]:
    response.raise_for_status()
    payload: dict[str, Any] = response.json()
    if not payload.get("ok"):
        raise RuntimeError(f"Slack API error: {payload.get('error', 'unknown')}")
    return payload


def _title(text: str, fallback: str) -> str:
    normalized = " ".join(text.split())
    return normalized[:100] or fallback
