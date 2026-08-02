from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
from pathlib import Path
from typing import Any

from .hindsight import HindsightConfig, HindsightHttpProvider
from .honcho import HonchoConfig, HonchoHttpProvider
from .models import MemoryArgumentError
from .outbox import Outbox, OutboxError
from .provider import MemoryProvider
from .worker import MemorySyncWorker

_ENV_NAME_RE = re.compile(r"^[A-Z_][A-Z0-9_]{0,63}$")
_MAX_BATCH_SIZE = 1024
_MAX_LEASE_SECONDS = 3600.0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(
        prog="cortana-memory-sync",
        description="Drain an explicitly selected optional memory-provider outbox.",
    )
    root.add_argument(
        "--outbox", type=Path, required=True, help="private memory outbox SQLite path"
    )
    root.add_argument("--provider", choices=("hindsight", "honcho"), required=True)
    root.add_argument(
        "--allow-append-only",
        action="store_true",
        help="acknowledge Honcho's append-only retain behavior",
    )
    root.add_argument(
        "--token-env", default=None, help="environment variable containing the bearer token"
    )
    root.add_argument("--base-url", default=None)
    root.add_argument("--bank", default="default", help="Hindsight bank")
    root.add_argument("--workspace-id", default="default", help="Honcho workspace ID")
    root.add_argument("--peer-id", default="cortana", help="Honcho peer ID")
    root.add_argument("--session-prefix", default="cortana", help="Honcho document-session prefix")
    root.add_argument(
        "--limit",
        type=_bounded_limit,
        default=64,
        help=f"number of rows claimed per batch (1-{_MAX_BATCH_SIZE})",
    )
    root.add_argument(
        "--lease-seconds",
        type=_bounded_lease_seconds,
        default=60.0,
        help=f"lease duration in seconds (0-{_MAX_LEASE_SECONDS:g}]",
    )
    root.add_argument("--worker-id", default="memory-sync")
    return root


def main(argv: list[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    provider: MemoryProvider | None = None
    try:
        if arguments.provider == "honcho" and not arguments.allow_append_only:
            raise MemoryArgumentError("Honcho sync requires explicit --allow-append-only")
        provider = _build_provider(arguments)
        with Outbox(arguments.outbox) as outbox:
            processed = MemorySyncWorker(
                outbox=outbox,
                provider=provider,
                worker_id=_bounded_worker_id(arguments.worker_id),
            ).run(limit=arguments.limit, lease_seconds=arguments.lease_seconds)
            print(
                json.dumps(
                    {
                        "provider": arguments.provider,
                        "processed": processed,
                        "telemetry": outbox.telemetry(),
                    },
                    sort_keys=True,
                )
            )
        return 0
    except (MemoryArgumentError, OutboxError, OSError, ValueError) as error:
        print(f"memory sync error: {error}", file=sys.stderr)
        return 1
    finally:
        if provider is not None:
            _close_provider(provider)


def _build_provider(arguments: argparse.Namespace) -> MemoryProvider:
    token_env = arguments.token_env or _default_token_env(arguments.provider)
    token = _token_from_env(token_env)
    base_url = arguments.base_url or _default_base_url(arguments.provider)
    if arguments.provider == "hindsight":
        return HindsightHttpProvider(
            HindsightConfig(base_url=base_url, bank=arguments.bank, token=token)
        )
    return HonchoHttpProvider(
        HonchoConfig(
            base_url=base_url,
            workspace_id=arguments.workspace_id,
            peer_id=arguments.peer_id,
            token=token,
            session_prefix=arguments.session_prefix,
        )
    )


def _default_token_env(provider: str) -> str:
    return "CORTANA_HINDSIGHT_TOKEN" if provider == "hindsight" else "CORTANA_HONCHO_TOKEN"


def _default_base_url(provider: str) -> str:
    return "http://127.0.0.1:8888" if provider == "hindsight" else "https://api.honcho.dev"


def _token_from_env(name: str) -> str:
    if not _ENV_NAME_RE.fullmatch(name):
        raise MemoryArgumentError("token environment variable name is invalid")
    token = os.environ.get(name, "")
    if not token.strip():
        raise MemoryArgumentError(f"{name} is not set")
    if any(character in token for character in "\r\n"):
        raise MemoryArgumentError("token contains unsafe characters")
    return token


def _bounded_worker_id(value: Any) -> str:
    worker_id = str(value).strip()
    if not worker_id or len(worker_id) > 128 or any(character in worker_id for character in "\r\n"):
        raise MemoryArgumentError("worker id must be 1-128 characters without newlines")
    return worker_id


def _bounded_limit(value: str) -> int:
    try:
        limit = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("limit must be an integer") from error
    if not 1 <= limit <= _MAX_BATCH_SIZE:
        raise argparse.ArgumentTypeError(f"limit must be between 1 and {_MAX_BATCH_SIZE}")
    return limit


def _bounded_lease_seconds(value: str) -> float:
    try:
        lease_seconds = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("lease-seconds must be a number") from error
    if not math.isfinite(lease_seconds) or not 0 < lease_seconds <= _MAX_LEASE_SECONDS:
        raise argparse.ArgumentTypeError(
            f"lease-seconds must be greater than 0 and at most {_MAX_LEASE_SECONDS:g}"
        )
    return lease_seconds


def _close_provider(provider: MemoryProvider) -> None:
    close = getattr(provider, "close", None)
    if callable(close):
        close()


def entrypoint() -> int:
    return main()


__all__ = ["entrypoint", "main", "parser"]
