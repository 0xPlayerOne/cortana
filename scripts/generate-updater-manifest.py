#!/usr/bin/env python3
"""Build one complete Tauri updater manifest from published release assets.

Tauri's matrix action can upload a manifest from each platform concurrently.
The last writer then wins and silently drops the other platforms.  This helper
is intentionally release-asset driven: all platform bundles and signatures
must already exist before the single manifest upload is performed.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PLATFORM_ASSETS = {
    "darwin-aarch64": "Cortana_{version}_aarch64.app.tar.gz",
    "darwin-aarch64-app": "Cortana_{version}_aarch64.app.tar.gz",
    "linux-x86_64": "Cortana_{version}_amd64.AppImage",
    "linux-x86_64-appimage": "Cortana_{version}_amd64.AppImage",
    "linux-x86_64-deb": "Cortana_{version}_amd64.deb",
    "linux-x86_64-rpm": "Cortana-{version}-1.x86_64.rpm",
    "windows-x86_64": "Cortana_{version}_x64_en-US.msi",
    "windows-x86_64-msi": "Cortana_{version}_x64_en-US.msi",
    "windows-x86_64-nsis": "Cortana_{version}_x64-setup.exe",
}


def run(*args: str) -> bytes:
    return subprocess.run(args, check=True, stdout=subprocess.PIPE).stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("tag", help="published Git tag, for example v0.35.0")
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("latest.json"),
        help="manifest output path (default: latest.json)",
    )
    parser.add_argument(
        "--allow-partial",
        action="store_true",
        help=(
            "publish a valid manifest for the signed updater targets that exist; "
            "the strict release verifier still rejects incomplete releases"
        ),
    )
    args = parser.parse_args()

    repo = os.environ.get("GH_REPO") or os.environ.get("GITHUB_REPOSITORY")
    if not repo:
        parser.error("GH_REPO or GITHUB_REPOSITORY is required")

    release = json.loads(
        run("gh", "release", "view", args.tag, "--repo", repo, "--json", "assets,body,publishedAt")
    )
    version = args.tag.removeprefix("v")
    assets: dict[str, dict[str, Any]] = {
        asset["name"]: asset
        for asset in release.get("assets", [])
        if isinstance(asset, dict) and isinstance(asset.get("name"), str)
    }
    missing = sorted(
        {
            name
            for name in PLATFORM_ASSETS.values()
            for name in (name.format(version=version), f"{name.format(version=version)}.sig")
            if name not in assets
        }
    )
    if missing and not args.allow_partial:
        raise SystemExit("release is missing updater assets: " + ", ".join(missing))

    signatures: dict[str, str] = {}

    def signature_for(name: str) -> str:
        if name not in signatures:
            asset = assets[name]
            url = asset.get("url") or asset.get("apiUrl")
            if not isinstance(url, str) or not url:
                raise SystemExit(f"release asset has no API URL: {name}")
            signatures[name] = (
                run("gh", "api", url, "--header", "Accept: application/octet-stream")
                .decode("utf-8")
                .strip()
            )
            if not signatures[name]:
                raise SystemExit(f"release signature is empty: {name}")
        return signatures[name]

    platforms: dict[str, dict[str, str]] = {}
    for platform, template in PLATFORM_ASSETS.items():
        archive = template.format(version=version)
        signature_name = f"{archive}.sig"
        if archive not in assets or signature_name not in assets:
            continue
        asset = assets[archive]
        url = asset.get("url") or asset.get("apiUrl")
        if not isinstance(url, str) or not url:
            raise SystemExit(f"release asset has no API URL: {archive}")
        platforms[platform] = {"signature": signature_for(signature_name), "url": url}

    if not platforms:
        raise SystemExit("release has no signed updater assets")

    published_at = release.get("publishedAt")
    if not isinstance(published_at, str) or not published_at:
        published_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    manifest = {
        "version": version,
        "notes": release.get("body") or "",
        "pub_date": published_at,
        "platforms": platforms,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    mode = " (partial)" if missing else ""
    print(f"generated updater manifest for {args.tag} with {len(platforms)} platforms{mode}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
