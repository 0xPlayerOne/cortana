#!/usr/bin/env python3
"""Reject Release Please pull requests that would move versions backwards."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

SEMVER = re.compile(r"(?<![0-9])([0-9]+)\.([0-9]+)\.([0-9]+)(?![0-9])")
VERSION_LINE = re.compile(r'^version\s*=\s*["\']([^"\']+)["\']', re.MULTILINE)


def git_show(ref: str, path: str) -> str:
    return subprocess.run(
        ["git", "show", f"{ref}:{path}"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def parse_version(text: str, path: str) -> tuple[int, int, int]:
    candidates = VERSION_LINE.findall(text) if path != "git tag" else [text.removeprefix("v")]
    for candidate in candidates:
        match = SEMVER.fullmatch(candidate)
        if match is not None:
            return tuple(int(part) for part in match.groups())
    raise ValueError(f"{path} does not contain a declared semantic version")


def version_at(ref: str) -> tuple[int, int, int]:
    return parse_version(git_show(ref, "Cargo.toml"), "Cargo.toml")


def highest_published_version() -> tuple[int, int, int] | None:
    tags = subprocess.run(
        ["git", "tag", "--list", "v[0-9]*", "--sort=-version:refname"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    versions = []
    for tag in tags:
        try:
            versions.append(parse_version(tag, "git tag"))
        except ValueError:
            continue
    return max(versions) if versions else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True, help="base commit SHA")
    parser.add_argument("--head", required=True, help="pull-request head commit SHA")
    parser.add_argument("--head-ref", required=True, help="pull-request head branch")
    args = parser.parse_args()

    if not args.head_ref.startswith("release-please--branches--main"):
        print(f"release version guard: non-release branch {args.head_ref}; pass")
        return 0

    base = version_at(args.base)
    head = version_at(args.head)
    published = highest_published_version()
    floor = max(base, published) if published is not None else base
    if head <= floor:
        print(
            "release version guard: refusing non-increasing release "
            f"{head[0]}.{head[1]}.{head[2]} from base "
            f"{base[0]}.{base[1]}.{base[2]} or published tag "
            f"{floor[0]}.{floor[1]}.{floor[2]}",
            file=sys.stderr,
        )
        return 1

    print(
        "release version guard: accepted "
        f"{base[0]}.{base[1]}.{base[2]} -> "
        f"{head[0]}.{head[1]}.{head[2]}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
