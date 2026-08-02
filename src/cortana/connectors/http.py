"""Small safety helpers for connector HTTP responses."""

from __future__ import annotations

import json
from typing import Any

import httpx

MAX_JSON_RESPONSE_BYTES = 16 * 1024 * 1024


def json_payload(
    response: httpx.Response,
    *,
    max_bytes: int = MAX_JSON_RESPONSE_BYTES,
) -> Any:
    """Decode a bounded JSON response without allowing parser amplification.

    ``httpx.Client.request`` buffers non-streaming responses before returning,
    so this guard cannot reduce transport memory for an already-buffered body.
    It does keep malformed or unexpectedly large provider payloads from being
    expanded into an unbounded Python object graph by ``Response.json``.
    """
    if max_bytes <= 0:
        raise ValueError("max_bytes must be greater than zero")
    try:
        body = response.content
    except httpx.ResponseNotRead:
        body = response.read()
    if len(body) > max_bytes:
        raise RuntimeError(f"provider JSON response exceeds {max_bytes} bytes")
    try:
        return json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError("provider returned invalid JSON") from error
