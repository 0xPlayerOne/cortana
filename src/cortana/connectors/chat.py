from __future__ import annotations

import datetime as dt
import os
import time
from collections.abc import Iterable
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
) -> Iterable[Document]:
    token = _required_env(token_env)
    headers = {"Authorization": f"Bot {token}"}
    with httpx.Client(
        base_url="https://discord.com/api/v10", headers=headers, timeout=30
    ) as client:
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
                    content = str(message.get("content") or "").strip()
                    attachments = "\n".join(
                        item.get("url", "") for item in message.get("attachments", [])
                    )
                    body = "\n".join(part for part in [content, attachments] if part)
                    if not body:
                        continue
                    yield Document(
                        source="discord",
                        source_id=str(message["id"]),
                        title=_title(content, f"Discord {channel_id}"),
                        content=f"{message.get('author', {}).get('username', 'unknown')}: {body}",
                        uri=f"https://discord.com/channels/@me/{channel_id}/{message['id']}",
                        updated_at=dt.datetime.fromisoformat(
                            str(message["timestamp"]).replace("Z", "+00:00")
                        ),
                        project=project,
                        metadata={
                            "channel_id": channel_id,
                            "author_id": message.get("author", {}).get("id"),
                        },
                    )
                before = str(messages[-1]["id"])
                if len(messages) < 100:
                    break


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
