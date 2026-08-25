"""Regression tests for release updater-manifest generation."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "scripts" / "generate-updater-manifest.py"


def _fake_gh(tmp_path: Path, assets: list[dict[str, str]]) -> Path:
    release = json.dumps(
        {"assets": assets, "body": "release notes", "publishedAt": "2026-08-25T17:27:31Z"}
    )
    script = tmp_path / "gh"
    script.write_text(
        "#!/usr/bin/env python3\n"
        "import json, sys\n"
        f"release = json.loads({release!r})\n"
        "if sys.argv[1:3] == ['release', 'view']:\n"
        "    print(json.dumps(release))\n"
        "elif sys.argv[1:2] == ['api']:\n"
        "    print('signature')\n"
        "else:\n"
        "    raise SystemExit(f'unsupported fake gh command: {sys.argv!r}')\n",
        encoding="utf-8",
    )
    script.chmod(0o755)
    return script


def _assets(version: str) -> list[dict[str, str]]:
    names = [
        f"Cortana_{version}_amd64.AppImage",
        f"Cortana_{version}_amd64.deb",
        f"Cortana-{version}-1.x86_64.rpm",
        f"Cortana_{version}_x64-setup.exe",
        f"Cortana_{version}_x64_en-US.msi",
    ]
    return [
        {
            "name": name,
            "url": f"https://uploads.example/{name}",
            "apiUrl": f"https://api.example/{name}",
        }
        for archive in names
        for name in (archive, f"{archive}.sig")
    ]


def test_partial_manifest_omits_missing_platforms(tmp_path: Path) -> None:
    version = "0.39.0"
    _fake_gh(tmp_path, _assets(version))
    output = tmp_path / "latest.json"
    env = {
        **os.environ,
        "GH_REPO": "0xPlayerOne/cortana",
        "PATH": f"{tmp_path}:{os.environ['PATH']}",
    }

    result = subprocess.run(
        [sys.executable, str(GENERATOR), "v0.39.0", "--allow-partial", "--output", str(output)],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    manifest = json.loads(output.read_text(encoding="utf-8"))
    assert set(manifest["platforms"]) == {
        "linux-x86_64",
        "linux-x86_64-appimage",
        "linux-x86_64-deb",
        "linux-x86_64-rpm",
        "windows-x86_64",
        "windows-x86_64-msi",
        "windows-x86_64-nsis",
    }
    assert not any(platform.startswith("darwin-") for platform in manifest["platforms"])
    assert "partial" in result.stdout


def test_strict_manifest_rejects_missing_platforms(tmp_path: Path) -> None:
    _fake_gh(tmp_path, _assets("0.39.0"))
    env = {
        **os.environ,
        "GH_REPO": "0xPlayerOne/cortana",
        "PATH": f"{tmp_path}:{os.environ['PATH']}",
    }

    result = subprocess.run(
        [sys.executable, str(GENERATOR), "v0.39.0", "--output", str(tmp_path / "latest.json")],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode != 0
    assert "release is missing updater assets" in result.stderr
