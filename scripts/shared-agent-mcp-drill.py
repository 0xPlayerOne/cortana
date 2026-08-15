#!/usr/bin/env python3
"""Run a disposable, subprocess-level MCP authorization drill.

The drill creates a temporary offline index, starts the real ``cortana mcp``
stdio transport with a file-backed scoped principal, and verifies the public
tool surface, workspace ACL filtering, and in-process token rotation.  It
never reads the live configuration or index, contacts a provider, authorizes a
source, or installs a service.
"""

from __future__ import annotations

import argparse
import json
import os
import selectors
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

MAX_LINE_BYTES = 1 * 1024 * 1024
PROCESS_TIMEOUT_SECONDS = 30.0
TOKEN_ENV = "CORTANA_MCP_DRILL_TOKEN"


class DrillError(RuntimeError):
    """A bounded MCP drill assertion failed."""


def _run(binary: str, args: list[str], *, timeout: float = PROCESS_TIMEOUT_SECONDS) -> None:
    try:
        subprocess.run(
            [binary, *args],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            timeout=timeout,
            text=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise DrillError(f"command failed: {args[0] if args else binary}") from error


def _write_config(config: Path, secrets: Path) -> None:
    text = config.read_text(encoding="utf-8")
    runtime = "[runtime]\n"
    if runtime not in text or "tokens = []" not in text:
        raise DrillError("generated config is missing the runtime or auth tables")
    text = text.replace(
        runtime,
        f"[runtime]\nenv_file = {json.dumps(str(secrets))}\n",
        1,
    )
    token_table = f'''[[auth.tokens]]
principal = "mcp-drill-agent"
token_env = "{TOKEN_ENV}"
scopes = ["query", "status"]
acl = ["work"]
'''
    config.write_text(text.replace("tokens = []", token_table.rstrip(), 1), encoding="utf-8")


def _write_secret(secrets: Path, value: str | None) -> None:
    secrets.write_text(
        f"{TOKEN_ENV}={'' if value is None else value}\n",
        encoding="utf-8",
    )
    os.chmod(secrets, 0o600)


def _send(process: subprocess.Popen[str], message: dict[str, Any]) -> None:
    if process.stdin is None:
        raise DrillError("MCP stdin is unavailable")
    process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
    process.stdin.flush()


def _receive(
    process: subprocess.Popen[str], selector: selectors.BaseSelector[Any], request_id: int
) -> dict[str, Any]:
    deadline = time.monotonic() + PROCESS_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise DrillError(f"MCP process exited with status {process.returncode}")
        events = selector.select(max(0.05, deadline - time.monotonic()))
        for key, _ in events:
            if key.data != "stdout":
                continue
            line = key.fileobj.readline()
            if not line:
                raise DrillError("MCP stdout closed before the response")
            if len(line.encode("utf-8")) > MAX_LINE_BYTES:
                raise DrillError("MCP response exceeded the bounded line limit")
            try:
                message = json.loads(line)
            except json.JSONDecodeError as error:
                raise DrillError("MCP returned invalid JSON") from error
            if message.get("id") == request_id:
                if not isinstance(message, dict):
                    raise DrillError("MCP response was not an object")
                return message
    raise DrillError("MCP response exceeded the bounded timeout")


def _request(
    process: subprocess.Popen[str],
    selector: selectors.BaseSelector[Any],
    request_id: int,
    method: str,
    params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    _send(
        process,
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            **({"params": params} if params is not None else {}),
        },
    )
    return _receive(process, selector, request_id)


def _assert_success(response: dict[str, Any], label: str) -> dict[str, Any]:
    if "error" in response:
        raise DrillError(f"{label} returned an MCP error")
    result = response.get("result")
    if not isinstance(result, dict):
        raise DrillError(f"{label} returned no result")
    return result


def _call_tool(
    process: subprocess.Popen[str],
    selector: selectors.BaseSelector[Any],
    request_id: int,
    name: str,
    arguments: dict[str, Any] | None = None,
) -> dict[str, Any]:
    result = _assert_success(
        _request(
            process,
            selector,
            request_id,
            "tools/call",
            {"name": name, "arguments": arguments or {}},
        ),
        f"tools/call {name}",
    )
    if result.get("isError") is True:
        raise DrillError(f"tools/call {name} returned an error result")
    return result


def _text(result: dict[str, Any]) -> str:
    content = result.get("content")
    if not isinstance(content, list):
        return ""
    return " ".join(
        item.get("text", "")
        for item in content
        if isinstance(item, dict) and isinstance(item.get("text"), str)
    )


def run(binary: str, keep: bool) -> None:
    root = Path(tempfile.mkdtemp(prefix="cortana-shared-agent-mcp-drill."))
    process: subprocess.Popen[str] | None = None
    selector: selectors.BaseSelector[Any] | None = None
    try:
        data = root / "data"
        config = root / "config.toml"
        secrets = root / "secrets.env"
        fixture = root / "fixture.jsonl"
        _run(binary, ["--offline", "--config", str(config), "init", "--data-dir", str(data)])
        _write_config(config, secrets)
        _write_secret(secrets, "mcp-secret")
        fixture.write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "source": "mcp-drill",
                            "source_id": "work-launch",
                            "title": "Work launch",
                            "content": "The launch phrase belongs to work.",
                            "project": "work",
                            "acl": ["work"],
                        }
                    ),
                    json.dumps(
                        {
                            "source": "mcp-drill",
                            "source_id": "personal-secret",
                            "title": "Personal note",
                            "content": "This must never cross the ACL boundary.",
                            "project": "personal",
                            "acl": ["personal"],
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        _run(binary, ["--offline", "--config", str(config), "ingest", str(fixture)])

        environment = os.environ.copy()
        environment[TOKEN_ENV] = "mcp-secret"
        process = subprocess.Popen(
            [
                binary,
                "--offline",
                "mcp",
                "--config",
                str(config),
                "--token-env",
                TOKEN_ENV,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=environment,
        )
        if process.stdout is None or process.stderr is None:
            raise DrillError("MCP stdio pipes are unavailable")
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")

        initialize = _assert_success(
            _request(
                process,
                selector,
                1,
                "initialize",
                {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "cortana-mcp-drill", "version": "1"},
                },
            ),
            "initialize",
        )
        if not initialize.get("protocolVersion"):
            raise DrillError("MCP initialize response omitted protocolVersion")
        _send(process, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

        tools = _assert_success(_request(process, selector, 2, "tools/list", {}), "tools/list")
        names = {
            item.get("name")
            for item in tools.get("tools", [])
            if isinstance(item, dict) and isinstance(item.get("name"), str)
        }
        required = {"search", "context", "brain_status"}
        if not required.issubset(names):
            raise DrillError("MCP tool listing omitted a required retrieval/status tool")

        search_text = _text(
            _call_tool(
                process,
                selector,
                3,
                "search",
                {"query": "launch phrase", "project": "work", "limit": 10},
            )
        )
        if "work-launch" not in search_text or "personal-secret" in search_text:
            raise DrillError("MCP search crossed the work ACL boundary")
        _call_tool(process, selector, 4, "brain_status")

        _write_secret(secrets, "mcp-secret-rotated")
        _call_tool(process, selector, 5, "brain_status")
        _write_secret(secrets, None)
        revoked = _request(process, selector, 6, "tools/call", {"name": "brain_status", "arguments": {}})
        revoked_text = _text(revoked.get("result", {})) if "error" not in revoked else ""
        if "error" not in revoked and not revoked_text.startswith("authorization error:"):
            raise DrillError("MCP accepted a principal after its file-backed token was revoked")

        print(json.dumps({"passed": True, "tools": len(names), "acl": "work", "rotation": True}))
    finally:
        if selector is not None:
            selector.close()
        if process is not None:
            try:
                process.terminate()
                process.wait(timeout=3)
            except (OSError, subprocess.TimeoutExpired):
                process.kill()
                process.wait(timeout=3)
        if keep:
            print(f"MCP drill retained: {root}", file=sys.stderr)
        else:
            shutil.rmtree(root, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        default=os.environ.get("CORTANA_BINARY", "cortana"),
        help="Cortana executable (default: CORTANA_BINARY or cortana)",
    )
    parser.add_argument(
        "--keep",
        action="store_true",
        default=os.environ.get("CORTANA_KEEP_DRILL") == "1",
        help="retain the temporary drill directory for incident review",
    )
    args = parser.parse_args()
    try:
        run(args.binary, args.keep)
    except DrillError as error:
        print(f"MCP drill failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
