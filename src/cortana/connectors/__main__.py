from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Iterable
from pathlib import Path

from .apple_notes import fetch as fetch_apple_notes
from .buzz import fetch as fetch_buzz
from .chat import fetch_discord, fetch_slack
from .google import fetch_drive, fetch_gmail
from .model import Document, emit


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="python -m cortana.connectors")
    root.add_argument("--project", default="default")
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

    gmail = commands.add_parser("gmail")
    _google_arguments(gmail)
    gmail.add_argument("--query", default="")
    gmail.add_argument("--label", action="append", dest="labels")
    return root


def main(argv: list[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    documents = _documents(arguments)
    count = emit(documents, sys.stdout)
    print(f"connector={arguments.connector} emitted={count}", file=sys.stderr)
    return 0


def _documents(arguments: argparse.Namespace) -> Iterable[Document]:
    if arguments.connector == "apple-notes":
        return fetch_apple_notes(arguments.project)
    if arguments.connector == "buzz":
        return fetch_buzz(arguments.root, arguments.project)
    if arguments.connector == "slack":
        return fetch_slack(arguments.channels, arguments.project, arguments.token_env)
    if arguments.connector == "discord":
        return fetch_discord(arguments.channels, arguments.project, arguments.token_env)
    token_path = _token_path(arguments)
    if arguments.connector == "google-drive":
        return fetch_drive(token_path, arguments.project, arguments.query)
    if arguments.connector == "gmail":
        return fetch_gmail(token_path, arguments.project, arguments.query, arguments.labels)
    raise RuntimeError(f"unsupported connector: {arguments.connector}")


def _google_arguments(command: argparse.ArgumentParser) -> None:
    command.add_argument("--token", type=Path)
    command.add_argument("--token-env", default="CORTANA_GOOGLE_TOKEN")


def _token_path(arguments: argparse.Namespace) -> Path:
    path = arguments.token or os.environ.get(arguments.token_env)
    if not path:
        raise RuntimeError(f"--token or {arguments.token_env} is required")
    token_path = Path(path).expanduser()
    if not token_path.is_file():
        raise RuntimeError(f"Google token file does not exist: {token_path}")
    return token_path


if __name__ == "__main__":
    raise SystemExit(main())
