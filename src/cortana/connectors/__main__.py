from __future__ import annotations

import argparse
import itertools
import os
import sys
from collections.abc import Iterable
from pathlib import Path

import httpx

from .apple_notes import fetch as fetch_apple_notes
from .buzz import fetch as fetch_buzz
from .chat import fetch_discord, fetch_slack
from .google import fetch_calendar, fetch_drive, fetch_gmail, validate_token_path
from .model import Document, emit


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="python -m cortana.connectors")
    root.add_argument("--project", default="default")
    root.add_argument("--cache-dir", type=Path)
    root.add_argument(
        "--no-cache",
        action="store_true",
        help="disable connector caches for bounded, read-only validation",
    )
    root.add_argument(
        "--max-documents",
        type=int,
        help="stop after emitting this many documents (used by bounded validation)",
    )
    commands = root.add_subparsers(dest="connector", required=True)

    commands.add_parser("apple-notes")

    buzz = commands.add_parser("buzz")
    buzz.add_argument(
        "--root",
        type=Path,
        default=Path.home() / "Library/Application Support/xyz.block.buzz.app",
    )

    slack = commands.add_parser("slack")
    slack.add_argument("--channel", action="append", required=True, dest="channels")
    slack.add_argument("--token-env", default="SLACK_BOT_TOKEN")

    discord = commands.add_parser("discord")
    discord.add_argument("--channel", action="append", required=True, dest="channels")
    discord.add_argument("--token-env", default="DISCORD_BOT_TOKEN")

    drive = commands.add_parser("google-drive")
    _google_arguments(drive)
    drive.add_argument("--query", default="trashed = false")
    drive.add_argument("--max-content-chars", type=int, default=50_000)
    drive.add_argument("--max-documents", type=int)

    gmail = commands.add_parser("gmail")
    _google_arguments(gmail)
    gmail.add_argument("--query", default="")
    gmail.add_argument("--label", action="append", dest="labels")

    calendar = commands.add_parser("google-calendar")
    _google_arguments(calendar)
    calendar.add_argument("--query", default="")
    return root


def main(argv: list[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    if arguments.max_documents is not None and arguments.max_documents <= 0:
        raise RuntimeError("--max-documents must be greater than zero")
    documents = _documents(arguments)
    if arguments.max_documents is not None:
        documents = itertools.islice(documents, arguments.max_documents)
    count = emit(documents, sys.stdout)
    print(f"connector={arguments.connector} emitted={count}", file=sys.stderr)
    return 0


def _documents(arguments: argparse.Namespace) -> Iterable[Document]:
    if arguments.connector == "apple-notes":
        return fetch_apple_notes(arguments.project, max_documents=arguments.max_documents)
    if arguments.connector == "buzz":
        return fetch_buzz(arguments.root, arguments.project, arguments.max_documents)
    if arguments.connector == "slack":
        return fetch_slack(
            arguments.channels,
            arguments.project,
            token_env=arguments.token_env,
            max_documents=arguments.max_documents,
            cache_dir=None if arguments.no_cache else arguments.cache_dir,
        )
    if arguments.connector == "discord":
        return fetch_discord(
            arguments.channels,
            arguments.project,
            arguments.token_env,
            cache_dir=None if arguments.no_cache else arguments.cache_dir,
            max_documents=arguments.max_documents,
        )
    token_path = _token_path(arguments)
    if arguments.connector == "google-drive":
        return fetch_drive(
            token_path,
            arguments.project,
            arguments.query,
            cache_dir=None if arguments.no_cache else arguments.cache_dir,
            max_content_chars=arguments.max_content_chars,
            max_documents=arguments.max_documents,
        )
    if arguments.connector == "gmail":
        return fetch_gmail(
            token_path,
            arguments.project,
            arguments.query,
            arguments.labels,
            cache_dir=None if arguments.no_cache else arguments.cache_dir,
            max_documents=arguments.max_documents,
        )
    if arguments.connector == "google-calendar":
        return fetch_calendar(
            token_path,
            arguments.project,
            arguments.query,
            max_documents=arguments.max_documents,
        )
    raise RuntimeError(f"unsupported connector: {arguments.connector}")


def _google_arguments(command: argparse.ArgumentParser) -> None:
    command.add_argument("--token", type=Path)
    command.add_argument("--token-env", default="CORTANA_GOOGLE_TOKEN")


def _token_path(arguments: argparse.Namespace) -> Path:
    path = arguments.token or os.environ.get(arguments.token_env)
    if not path:
        raise RuntimeError(f"--token or {arguments.token_env} is required")
    return validate_token_path(Path(path))


def entrypoint() -> int:
    try:
        return main()
    except (OSError, RuntimeError, httpx.HTTPError) as error:
        print(f"connector error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(entrypoint())
