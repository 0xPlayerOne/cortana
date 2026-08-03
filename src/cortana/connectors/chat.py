from __future__ import annotations

import datetime as dt
import json
import os
import sqlite3
import stat
import sys
import time
from collections.abc import Iterable
from pathlib import Path
from typing import Any

import httpx

from .http import json_payload
from .model import Document


def fetch_slack(
    channel_ids: list[str],
    project: str,
    token_env: str = "SLACK_BOT_TOKEN",
    max_documents: int | None = None,
) -> Iterable[Document]:
    token = _required_env(token_env)
    headers = {"Authorization": f"Bearer {token}"}
    with httpx.Client(
        base_url="https://slack.com/api",
        headers=headers,
        timeout=30,
        follow_redirects=False,
    ) as client:
        for channel_id in channel_ids:
            cursor = ""
            while True:
                response = _get_with_backoff(
                    client,
                    "/conversations.history",
                    params={
                        "channel": channel_id,
                        "limit": min(200, max_documents or 200),
                        "cursor": cursor,
                    },
                )
                payload = _slack_payload(response)
                for parent in _slack_messages(payload):
                    parent_timestamp = _slack_timestamp(parent.get("ts"))
                    if parent_timestamp is None:
                        continue
                    thread = [parent]
                    try:
                        has_replies = int(parent.get("reply_count", 0)) > 0
                    except (TypeError, ValueError):
                        has_replies = False
                    if has_replies:
                        replies = _get_with_backoff(
                            client,
                            "/conversations.replies",
                            params={"channel": channel_id, "ts": parent["ts"], "limit": 200},
                        )
                        thread = _slack_messages(_slack_payload(replies))
                    valid_thread = [
                        message
                        for message in thread
                        if isinstance(message, dict)
                        and _slack_timestamp(message.get("ts")) is not None
                    ]
                    text = "\n".join(
                        f"{message.get('user', 'unknown')}: {message.get('text', '')}"
                        for message in valid_thread
                        if message.get("text")
                    )
                    if text:
                        updated = max(
                            _slack_timestamp(message.get("ts"))
                            for message in valid_thread
                            if _slack_timestamp(message.get("ts")) is not None
                        )
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
                                    {m.get("user") for m in valid_thread if m.get("user")}
                                ),
                                "message_count": len(valid_thread),
                            },
                        )
                response_metadata = payload.get("response_metadata")
                cursor = str(
                    response_metadata.get("next_cursor")
                    if isinstance(response_metadata, dict)
                    else ""
                )
                if not cursor:
                    break


def fetch_discord(
    channel_ids: list[str],
    project: str,
    token_env: str = "DISCORD_BOT_TOKEN",
    cache_dir: Path | None = None,
    max_documents: int | None = None,
) -> Iterable[Document]:
    token = _required_env(token_env)
    headers = {"Authorization": f"Bot {token}"}
    with httpx.Client(
        base_url="https://discord.com/api/v10",
        headers=headers,
        timeout=30,
        follow_redirects=False,
    ) as client:
        if cache_dir is not None:
            yield from _fetch_discord_cached(
                client,
                channel_ids,
                project,
                cache_dir,
                max_documents=max_documents,
            )
            return
        emitted = 0
        for channel_id in channel_ids:
            before: str | None = None
            while True:
                params: dict[str, Any] = {"limit": min(100, max_documents or 100)}
                if before:
                    params["before"] = before
                response = _get_with_backoff(
                    client, f"/channels/{channel_id}/messages", params=params
                )
                response.raise_for_status()
                messages = _discord_page(json_payload(response))
                if not messages:
                    break
                for message in messages:
                    document = _discord_document(message, channel_id, project)
                    if document is not None:
                        yield document
                        emitted += 1
                        if max_documents is not None and emitted >= max_documents:
                            return
                before = str(messages[-1]["id"])
                if len(messages) < 100:
                    break


def _fetch_discord_cached(
    client: httpx.Client,
    channel_ids: list[str],
    project: str,
    cache_dir: Path,
    max_documents: int | None = None,
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
                messages = _discord_page(json_payload(response))
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
            "SELECT rowid,channel_id,body FROM discord_messages ORDER BY CAST(id AS INTEGER)"
        )
        for rowid, channel_id, body in rows:
            try:
                cached_message = json.loads(str(body))
            except json.JSONDecodeError:
                print(
                    f"connector warning: removing malformed Discord cache row {rowid}",
                    file=sys.stderr,
                )
                cache.execute("DELETE FROM discord_messages WHERE rowid=?", (rowid,))
                continue
            if (
                not isinstance(cached_message, dict)
                or not str(cached_message.get("id") or "").isdigit()
            ):
                print(
                    f"connector warning: removing malformed Discord cache row {rowid}",
                    file=sys.stderr,
                )
                cache.execute("DELETE FROM discord_messages WHERE rowid=?", (rowid,))
                continue
            document = _discord_document(cached_message, str(channel_id), project)
            if document is not None:
                yield document
        cache.commit()
    finally:
        cache.close()


def _discord_cache(cache_dir: Path) -> sqlite3.Connection:
    _prepare_private_cache_directory(cache_dir)
    path = cache_dir / "discord.sqlite3"
    if path.is_symlink():
        raise RuntimeError(f"Discord cache path must not be a symlink: {path}")
    flags = os.O_CREAT | os.O_RDWR | getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
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


def _prepare_private_cache_directory(path: Path) -> None:
    _reject_symlink_components(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    current = path
    while True:
        try:
            metadata = current.lstat()
        except FileNotFoundError as error:
            raise RuntimeError(f"Discord cache directory does not exist: {current}") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise RuntimeError(f"Discord cache directory must not contain a symlink: {current}")
        if not stat.S_ISDIR(metadata.st_mode):
            raise RuntimeError(f"Discord cache path is not a directory: {current}")
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
            raise RuntimeError(f"Discord cache directory must not contain a symlink: {current}")
        if current == current.parent:
            return
        current = current.parent


def _full_refresh_due(last_full: str) -> bool:
    try:
        previous = dt.datetime.fromisoformat(last_full)
    except ValueError:
        return True
    return dt.datetime.now(dt.UTC) - previous.astimezone(dt.UTC) >= dt.timedelta(days=1)


def _discord_document(message: dict[str, Any], channel_id: str, project: str) -> Document | None:
    message_id = str(message.get("id") or "").strip()
    if not message_id:
        return None
    content = str(message.get("content") or "").strip()
    attachments = "\n".join(
        str(item.get("url") or "")
        for item in message.get("attachments", [])
        if isinstance(item, dict) and item.get("url")
    )
    body = "\n".join(part for part in [content, attachments] if part)
    if not body:
        return None
    updated_at = _parse_discord_timestamp(message.get("timestamp"))
    if updated_at is None:
        return None
    author = message.get("author")
    username = author.get("username", "unknown") if isinstance(author, dict) else "unknown"
    author_id = author.get("id") if isinstance(author, dict) else None
    return Document(
        source="discord",
        source_id=message_id,
        title=_title(content, f"Discord {channel_id}"),
        content=f"{username}: {body}",
        uri=f"https://discord.com/channels/@me/{channel_id}/{message_id}",
        updated_at=updated_at,
        project=project,
        metadata={
            "channel_id": channel_id,
            "author_id": author_id,
        },
    )


def _discord_page(value: object) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        raise RuntimeError("Discord message page must be a list")
    messages: list[dict[str, Any]] = []
    for index, message in enumerate(value):
        if not isinstance(message, dict):
            print(
                f"connector warning: skipping malformed Discord message {index}",
                file=sys.stderr,
            )
            continue
        message_id = str(message.get("id") or "").strip()
        if not message_id.isdigit():
            print(
                f"connector warning: skipping Discord message {index} with invalid id",
                file=sys.stderr,
            )
            continue
        message["id"] = message_id
        messages.append(message)
    if value and not messages:
        raise RuntimeError("Discord message page contained no usable records")
    return messages


def _parse_discord_timestamp(value: object) -> dt.datetime | None:
    if not isinstance(value, str) or not value.strip():
        return None
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=dt.UTC)
    return parsed


def _required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise RuntimeError(f"{name} is required")
    return value


def _slack_timestamp(value: object) -> float | None:
    try:
        timestamp = float(value)
    except (TypeError, ValueError):
        return None
    return timestamp if timestamp >= 0 else None


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
        payload = json_payload(response)
        if isinstance(payload, dict) and payload.get("retry_after") is not None:
            return min(max(float(payload["retry_after"]), 0.0), 60.0)
    except (RuntimeError, TypeError, ValueError):
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
    raw_payload = json_payload(response)
    if not isinstance(raw_payload, dict):
        raise RuntimeError("Slack API returned an invalid response")
    payload: dict[str, Any] = raw_payload
    if not payload.get("ok"):
        raise RuntimeError(f"Slack API error: {payload.get('error', 'unknown')}")
    return payload


def _slack_messages(payload: dict[str, Any]) -> list[dict[str, Any]]:
    value = payload.get("messages")
    if value is None:
        return []
    if not isinstance(value, list):
        raise RuntimeError("Slack API returned an invalid message page")
    messages = [message for message in value if isinstance(message, dict)]
    if value and not messages:
        raise RuntimeError("Slack API message page contained no usable records")
    if len(messages) != len(value):
        print("connector warning: skipping malformed Slack message records", file=sys.stderr)
    return messages


def _title(text: str, fallback: str) -> str:
    normalized = " ".join(text.split())
    return normalized[:100] or fallback
