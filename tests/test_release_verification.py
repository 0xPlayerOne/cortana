"""Regression tests for the release verification scripts.

Covers the installed-vs-checkout version-skew gate: ``verify-release.sh`` must
reject an archive whose packaged ``bin/cortana --version`` output does not
match the version encoded in the archive name, and the published-asset gate
(``verify-desktop-release.sh``) must execute the published Linux core binary
and assert its version against the release tag on Linux hosts.

The desktop verifier normally talks to GitHub through ``gh``; the tests
substitute a fake ``gh`` and ``minisign`` on ``PATH`` that serve a synthetic
release, so the whole gate runs offline against locally built archives. The
shim asserts the exact Minisign invocation and key/signature decoding; the
published-release workflow exercises the real cryptographic verifier.
"""

import base64
import hashlib
import json
import os
import plistlib
import subprocess
import sys
import tarfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
VERIFY_RELEASE = ROOT / "scripts" / "verify-release.sh"
VERIFY_DESKTOP = ROOT / "scripts" / "verify-desktop-release.sh"
UPDATER_PUBLIC_KEY = base64.b64decode(
    json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())["plugins"]["updater"][
        "pubkey"
    ]
)

TAG = "v9.9.9"
VERSION = "9.9.9"
APP_ARCHIVE = f"Cortana_{VERSION}_aarch64.app.tar.gz"
SIGNED_ARCHIVES = (
    APP_ARCHIVE,
    f"Cortana_{VERSION}_amd64.AppImage",
    f"Cortana_{VERSION}_amd64.deb",
    f"Cortana-{VERSION}-1.x86_64.rpm",
    f"Cortana_{VERSION}_x64-setup.exe",
    f"Cortana_{VERSION}_x64_en-US.msi",
)
MINISIGN_SIGNATURE = (
    b"untrusted comment: signature from tauri secret key\n"
    b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
)

if sys.platform == "linux":
    HOST_SUFFIX = "unknown-linux-gnu"
elif sys.platform == "darwin":
    HOST_SUFFIX = "apple-darwin"
else:
    HOST_SUFFIX = None

requires_shell = pytest.mark.skipif(
    HOST_SUFFIX is None,
    reason="release verification scripts require bash, tar, and shasum",
)


def fake_binary(reported_version: str) -> bytes:
    return f'#!/bin/sh\necho "cortana {reported_version}"\n'.encode()


def write_executable(path: Path, content: bytes) -> None:
    path.write_bytes(content)
    path.chmod(0o755)


def build_core_tree(directory: Path, version: str, suffix: str | None = None) -> Path:
    """Create a syntactically valid core release tree under ``directory``."""
    suffix = suffix or HOST_SUFFIX
    root = directory / f"cortana-v{version}-{suffix}"
    (root / "bin").mkdir(parents=True)
    (root / "share/cortana/web").mkdir(parents=True)
    (root / "skills/cortana").mkdir(parents=True)
    (root / "dist").mkdir(parents=True)
    (root / "scripts").mkdir(parents=True)
    write_executable(root / "bin/cortana", fake_binary(version))
    write_executable(root / "install.sh", b"#!/bin/sh\n")
    (root / "share/cortana/web/index.html").write_text("<html></html>\n")
    (root / "config.example.toml").write_text("[query]\n")
    (root / "skills/cortana/SKILL.md").write_text("skill\n")
    (root / "dist/cortana_brain-9.9.9-py3-none-any.whl").write_bytes(b"wheel")
    (root / "scripts/install-release.sh").write_text("#!/bin/sh\n")
    (root / "scripts/verify-release.sh").write_text("#!/bin/sh\n")
    return root


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    digest.update(path.read_bytes())
    return digest.hexdigest()


def build_core_archive(
    directory: Path, version: str, binary_version: str, suffix: str | None = None
) -> Path:
    """Build ``cortana-v{version}-{suffix}.tar.gz`` plus its sidecar.

    The packaged fake binary reports ``cortana {binary_version}`` so tests can
    stage a matching release or a version-skewed one.
    """
    root = build_core_tree(directory, version, suffix)
    for path in root.rglob("*"):
        if path.is_file() and path.name == "cortana" and path.parent.name == "bin":
            write_executable(path, fake_binary(binary_version))
    archive = directory / f"{root.name}.tar.gz"
    with tarfile.open(archive, "w:gz") as handle:
        handle.add(root, arcname=root.name)
    sidecar = directory / f"{archive.name}.sha256"
    sidecar.write_text(f"{sha256_of(archive)}  {archive.name}\n")
    return archive


def build_app_archive(directory: Path) -> Path:
    """Build the macOS application archive the desktop verifier inspects."""
    app = directory / "Cortana.app"
    (app / "Contents/MacOS").mkdir(parents=True)
    (app / "Contents/Resources/resources/cortana-connectors").mkdir(parents=True)
    write_executable(app / "Contents/MacOS/cortana", fake_binary(VERSION))
    write_executable(app / "Contents/MacOS/cortana-desktop", b"#!/bin/sh\n")
    (app / "Contents/Info.plist").write_bytes(
        plistlib.dumps(
            {
                "CFBundleIdentifier": "ai.cortana.desktop",
                "CFBundlePackageType": "APPL",
                "CFBundleExecutable": "cortana-desktop",
                "CFBundleShortVersionString": VERSION,
                "CFBundleVersion": VERSION,
            }
        )
    )
    (app / "Contents/Resources/resources/cortana-connectors/__init__.py").write_text("")
    archive = directory / APP_ARCHIVE
    with tarfile.open(archive, "w:gz") as handle:
        handle.add(app, arcname=app.name)
    return archive


def latest_manifest() -> bytes:
    platform_assets = {
        "darwin-aarch64": f"Cortana_{VERSION}_aarch64.dmg",
        "darwin-aarch64-app": APP_ARCHIVE,
        "linux-x86_64": f"cortana-{TAG}-x86_64-unknown-linux-gnu.tar.gz",
        "linux-x86_64-appimage": f"Cortana_{VERSION}_amd64.AppImage",
        "linux-x86_64-deb": f"Cortana_{VERSION}_amd64.deb",
        "linux-x86_64-rpm": f"Cortana-{VERSION}-1.x86_64.rpm",
        "windows-x86_64": f"Cortana_{VERSION}_x64-setup.exe",
        "windows-x86_64-msi": f"Cortana_{VERSION}_x64_en-US.msi",
        "windows-x86_64-nsis": f"Cortana_{VERSION}_x64-setup.exe",
    }
    manifest = {
        "version": VERSION,
        "platforms": {
            platform: {
                "url": f"https://github.com/test/repo/releases/download/{TAG}/{asset}",
                "signature": "test-signature",
            }
            for platform, asset in platform_assets.items()
        },
    }
    return json.dumps(manifest).encode()


def required_asset_names() -> list[str]:
    return [
        f"cortana-{TAG}-aarch64-apple-darwin.tar.gz",
        f"cortana-{TAG}-aarch64-apple-darwin.tar.gz.sha256",
        f"cortana-{TAG}-x86_64-unknown-linux-gnu.tar.gz",
        f"cortana-{TAG}-x86_64-unknown-linux-gnu.tar.gz.sha256",
        f"Cortana_{VERSION}_aarch64.dmg",
        APP_ARCHIVE,
        f"{APP_ARCHIVE}.sig",
        f"Cortana_{VERSION}_amd64.AppImage",
        f"Cortana_{VERSION}_amd64.AppImage.sig",
        f"Cortana_{VERSION}_amd64.deb",
        f"Cortana_{VERSION}_amd64.deb.sig",
        f"Cortana-{VERSION}-1.x86_64.rpm",
        f"Cortana-{VERSION}-1.x86_64.rpm.sig",
        f"Cortana_{VERSION}_x64-setup.exe",
        f"Cortana_{VERSION}_x64-setup.exe.sig",
        f"Cortana_{VERSION}_x64_en-US.msi",
        f"Cortana_{VERSION}_x64_en-US.msi.sig",
        "latest.json",
    ]


def build_desktop_assets(directory: Path, linux_binary_version: str) -> dict[str, bytes]:
    """Build every artifact the desktop verifier downloads or inspects."""
    assets: dict[str, bytes] = {}
    for suffix in ("aarch64-apple-darwin", "x86_64-unknown-linux-gnu"):
        binary_version = linux_binary_version if suffix == "x86_64-unknown-linux-gnu" else VERSION
        archive = build_core_archive(directory, VERSION, binary_version, suffix)
        assets[archive.name] = archive.read_bytes()
        assets[archive.name + ".sha256"] = (directory / f"{archive.name}.sha256").read_bytes()
    app = build_app_archive(directory)
    assets[app.name] = app.read_bytes()
    for archive in SIGNED_ARCHIVES:
        assets.setdefault(archive, f"synthetic {archive}\n".encode())
        assets[f"{archive}.sig"] = base64.b64encode(MINISIGN_SIGNATURE)
    assets["latest.json"] = latest_manifest()
    for name in required_asset_names():
        assets.setdefault(name, b"synthetic release asset\n")
    return assets


def fake_gh(bin_dir: Path, assets: dict[str, bytes]) -> None:
    """Write a fake ``gh`` that serves the synthetic release from ``assets``."""
    assets_dir = bin_dir / "assets"
    assets_dir.mkdir(parents=True)
    view = {
        "assets": [
            {
                "name": name,
                "size": len(content),
            }
            for name, content in sorted(assets.items())
        ]
    }
    (assets_dir / "release-view.json").write_text(json.dumps(view))
    for name, content in assets.items():
        (assets_dir / name).write_bytes(content)
    script = bin_dir / "gh"
    script.write_text(
        f"""#!/usr/bin/env bash
set -euo pipefail
ROOT='{assets_dir}'
if [[ "${{1:-}}" == "release" && "${{2:-}}" == "view" ]]; then
  cat "$ROOT/release-view.json"
  exit 0
fi
if [[ "${{1:-}}" == "release" && "${{2:-}}" == "download" ]]; then
  pattern=""
  dir=""
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --pattern) pattern="$2"; shift 2 ;;
      --dir) dir="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  attempts_file="$ROOT/.download-attempts"
  attempts=0
  if [[ -f "$attempts_file" ]]; then
    attempts="$(cat "$attempts_file")"
  fi
  attempts=$((attempts + 1))
  printf '%s\n' "$attempts" > "$attempts_file"
  if [[ -n "${{FAKE_GH_SLEEP_DOWNLOAD:-}}" ]]; then
    sleep "$FAKE_GH_SLEEP_DOWNLOAD"
  fi
  if [[ "${{FAKE_GH_ALWAYS_FAIL_DOWNLOAD:-0}}" == "1" ||
        "$attempts" -le "${{FAKE_GH_FAIL_DOWNLOADS:-0}}" ]]; then
    echo "synthetic release download failure for $pattern" >&2
    exit 75
  fi
  mkdir -p "$dir"
  if [[ -f "$ROOT/$pattern" ]]; then
    cp "$ROOT/$pattern" "$dir/$pattern"
  else
    : > "$dir/$pattern"
  fi
  exit 0
fi
echo "unexpected gh invocation: $*" >&2
exit 1
""",
    )
    script.chmod(0o755)


def fake_minisign(bin_dir: Path, valid: bool = True, invalid_archive: str | None = None) -> None:
    """Provide a deterministic verifier shim without committing a private key."""
    expected = repr(MINISIGN_SIGNATURE)
    expected_public_key = repr(UPDATER_PUBLIC_KEY.rstrip(b"\n") + b"\n")
    selected_archive = repr(invalid_archive)
    outcome = "0" if valid else "1"
    script = bin_dir / "minisign"
    script.write_text(
        f"""#!/usr/bin/env python3
from pathlib import Path
import sys

args = sys.argv[1:]
try:
    if len(args) != 6 or args[0] != '-Vm' or args[2] != '-x' or args[4] != '-p':
        raise ValueError('unexpected minisign invocation')
    archive = Path(args[1])
    signature = Path(args[3]).read_bytes()
    public_key = Path(args[5]).read_bytes()
    archive.stat()
except (ValueError, IndexError, OSError):
    raise SystemExit(1)
if signature.rstrip(b'\\n') != {expected}.rstrip(b'\\n') or public_key != {expected_public_key}:
    raise SystemExit(1)
if {selected_archive} is not None and archive.name == {selected_archive}:
    raise SystemExit(1)
raise SystemExit({outcome})
"""
    )
    script.chmod(0o755)


def run_script(
    script: Path, args: list[str], env_extra: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    if env_extra:
        env.update(env_extra)
    return subprocess.run(
        ["bash", str(script), *args],
        capture_output=True,
        text=True,
        env=env,
        timeout=120,
    )


def run_desktop_verify(
    tmp_path: Path,
    assets: dict[str, bytes],
    force_linux: bool = False,
    minisign_mode: str | None = "valid",
    require_minisign: bool = False,
    invalid_archive: str | None = None,
    fail_downloads: int = 0,
    always_fail_download: bool = False,
    download_attempts: str | None = None,
    download_timeout: str | None = None,
    download_sleep: str | None = None,
) -> subprocess.CompletedProcess[str]:
    bin_dir = tmp_path / "bin"
    fake_gh(bin_dir, assets)
    if minisign_mode in {"valid", "invalid", "invalid_archive"}:
        if minisign_mode == "invalid_archive" and invalid_archive is None:
            invalid_archive = f"Cortana_{VERSION}_amd64.AppImage"
        fake_minisign(
            bin_dir,
            valid=minisign_mode != "invalid",
            invalid_archive=invalid_archive if minisign_mode == "invalid_archive" else None,
        )
    if force_linux:
        # Emulate a Linux verifier host so the published-binary execution gate
        # runs on every platform; the fixture binary is a POSIX shell script
        # and executes anywhere.
        uname = bin_dir / "uname"
        uname.write_text("#!/usr/bin/env bash\nprintf '%s\\n' Linux\n")
        uname.chmod(0o755)
    env = {
        "PATH": f"{bin_dir}:{os.environ['PATH']}",
        "GH_REPO": "test/repo",
        "CORTANA_DOWNLOAD_RETRY_DELAY": "0",
        "CORTANA_DOWNLOAD_TIMEOUT_SECONDS": "10",
    }
    if fail_downloads:
        env["FAKE_GH_FAIL_DOWNLOADS"] = str(fail_downloads)
    if always_fail_download:
        env["FAKE_GH_ALWAYS_FAIL_DOWNLOAD"] = "1"
    if download_attempts is not None:
        env["CORTANA_DOWNLOAD_ATTEMPTS"] = download_attempts
    if download_timeout is not None:
        env["CORTANA_DOWNLOAD_TIMEOUT_SECONDS"] = download_timeout
    if download_sleep is not None:
        env["FAKE_GH_SLEEP_DOWNLOAD"] = download_sleep
    if minisign_mode is None:
        env["CORTANA_MINISIGN_BIN"] = "missing-test-minisign"
    if require_minisign:
        env["CORTANA_REQUIRE_MINISIGN"] = "1"
    return run_script(VERIFY_DESKTOP, [TAG], env_extra=env)


@requires_shell
def test_verify_release_accepts_matching_packaged_version(tmp_path: Path) -> None:
    archive = build_core_archive(tmp_path, VERSION, VERSION)

    result = run_script(VERIFY_RELEASE, [str(archive)])

    assert result.returncode == 0, result.stderr
    assert f"Verified packaged binary version {VERSION}" in result.stdout


@requires_shell
def test_verify_release_rejects_installed_vs_checkout_version_skew(tmp_path: Path) -> None:
    # The archive claims v9.9.9 but the packaged binary reports 1.0.0: the
    # release gate must fail instead of shipping a stale or mislabeled binary.
    archive = build_core_archive(tmp_path, VERSION, "1.0.0")

    result = run_script(VERIFY_RELEASE, [str(archive)])

    assert result.returncode == 1
    assert "release binary version mismatch" in result.stderr
    assert "expected 'cortana 9.9.9', got 'cortana 1.0.0'" in result.stderr


@requires_shell
def test_verify_release_fails_closed_without_a_versioned_archive_name(tmp_path: Path) -> None:
    # A name that cannot be matched to a plain semver release cannot be
    # verified, so the verifier refuses instead of skipping the version gate.
    root = build_core_tree(tmp_path, VERSION)
    archive = tmp_path / "release.tar.gz"
    with tarfile.open(archive, "w:gz") as handle:
        handle.add(root, arcname=root.name)
    (tmp_path / "release.tar.gz.sha256").write_text(f"{sha256_of(archive)}  release.tar.gz\n")

    result = run_script(VERIFY_RELEASE, [str(archive)])

    assert result.returncode == 1
    assert "cannot derive the expected release version" in result.stderr


@requires_shell
def test_verify_release_still_rejects_checksum_mismatch(tmp_path: Path) -> None:
    archive = build_core_archive(tmp_path, VERSION, VERSION)
    sidecar = tmp_path / f"{archive.name}.sha256"
    sidecar.write_text(f"{'0' * 64}  {archive.name}\n")

    result = run_script(VERIFY_RELEASE, [str(archive)])

    assert result.returncode == 1
    assert "release checksum mismatch" in result.stderr


@requires_shell
def test_desktop_verify_accepts_matching_published_binary(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(tmp_path, assets)

    assert result.returncode == 0, result.stderr
    if sys.platform == "linux":
        assert "verified published Linux binary version matches v9.9.9" in result.stdout
    else:
        assert "skipped published binary execution on non-Linux host" in result.stdout


@requires_shell
def test_desktop_verify_retries_transient_release_download(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(tmp_path, assets, fail_downloads=1)

    assert result.returncode == 0, result.stderr
    assert "retrying in 0s" in result.stderr


@requires_shell
def test_desktop_verify_fails_after_bounded_release_download_retries(
    tmp_path: Path,
) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(tmp_path, assets, always_fail_download=True)

    assert result.returncode == 1
    assert "failed after 3 attempts" in result.stderr


@requires_shell
def test_desktop_verify_rejects_unbounded_retry_budget(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(tmp_path, assets, download_attempts="6")

    assert result.returncode == 2
    assert "no greater than 5" in result.stderr


@requires_shell
def test_desktop_verify_rejects_unbounded_download_timeout(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(tmp_path, assets, download_timeout="601")

    assert result.returncode == 2
    assert "no greater than 600" in result.stderr


@requires_shell
def test_desktop_verify_times_out_a_wedged_download(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(
        tmp_path,
        assets,
        download_timeout="1",
        download_sleep="2",
    )

    assert result.returncode == 1
    assert "timed out after 1s" in result.stderr


@requires_shell
def test_desktop_verify_rejects_missing_required_asset(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)
    missing = f"Cortana_{VERSION}_amd64.AppImage.sig"
    assets.pop(missing)

    result = run_desktop_verify(tmp_path, assets)

    assert result.returncode == 1
    assert "release is missing assets" in result.stderr
    assert missing in result.stderr


@requires_shell
def test_desktop_verify_rejects_published_version_skew(tmp_path: Path) -> None:
    # The published Linux core archive is structurally valid and checksummed,
    # but its binary reports the wrong version; the final asset gate must fail.
    # A fake `uname` emulates the Linux verifier host so this code path is
    # exercised on every platform, not only in Linux CI.
    assets = build_desktop_assets(tmp_path, "1.0.0")

    result = run_desktop_verify(tmp_path, assets, force_linux=True)

    assert result.returncode == 1
    assert "published Linux binary version mismatch" in result.stderr
    assert "expected 'cortana 9.9.9', got 'cortana 1.0.0'" in result.stderr


def rewrite_app_archive_with_plist(tmp_path: Path, **updates: str) -> dict[str, bytes]:
    assets = build_desktop_assets(tmp_path, VERSION)
    app_archive = tmp_path / APP_ARCHIVE
    app = tmp_path / "rewritten.app"
    with tarfile.open(app_archive, "r:gz") as source:
        source.extractall(app)
    plist_path = app / "Cortana.app/Contents/Info.plist"
    plist = plistlib.loads(plist_path.read_bytes())
    plist.update(updates)
    plist_path.write_bytes(plistlib.dumps(plist))
    with tarfile.open(app_archive, "w:gz") as archive:
        archive.add(app / "Cortana.app", arcname="Cortana.app")
    assets[APP_ARCHIVE] = app_archive.read_bytes()
    return assets


@requires_shell
def test_desktop_verify_rejects_macos_bundle_version_skew(tmp_path: Path) -> None:
    assets = rewrite_app_archive_with_plist(tmp_path, CFBundleShortVersionString="1.0.0")

    result = run_desktop_verify(tmp_path, assets)

    assert result.returncode == 1
    assert "CFBundleShortVersionString mismatch" in result.stderr
    assert "expected '9.9.9', got '1.0.0'" in result.stderr


@requires_shell
def test_desktop_verify_rejects_macos_bundle_identity_skew(tmp_path: Path) -> None:
    assets = rewrite_app_archive_with_plist(tmp_path, CFBundleIdentifier="com.example.wrong")

    result = run_desktop_verify(tmp_path, assets)

    assert result.returncode == 1
    assert "CFBundleIdentifier mismatch" in result.stderr
    assert "expected 'ai.cortana.desktop', got 'com.example.wrong'" in result.stderr


@requires_shell
def test_desktop_verify_rejects_invalid_tauri_signature(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(tmp_path, assets, minisign_mode="invalid")

    assert result.returncode == 1
    assert "Tauri updater signature verification failed" in result.stderr


@requires_shell
def test_desktop_verify_rejects_invalid_non_first_signed_archive(tmp_path: Path) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)
    invalid_archive = f"Cortana_{VERSION}_amd64.AppImage"

    result = run_desktop_verify(
        tmp_path,
        assets,
        minisign_mode="invalid_archive",
        invalid_archive=invalid_archive,
    )

    assert result.returncode == 1
    assert f"verified Tauri updater signature: {APP_ARCHIVE}" in result.stdout
    assert f"Tauri updater signature verification failed: {invalid_archive}" in result.stderr


@requires_shell
def test_desktop_verify_fails_closed_when_minisign_is_required_but_unavailable(
    tmp_path: Path,
) -> None:
    assets = build_desktop_assets(tmp_path, VERSION)

    result = run_desktop_verify(
        tmp_path,
        assets,
        minisign_mode=None,
        require_minisign=True,
    )

    assert result.returncode == 1
    assert "CORTANA_REQUIRE_MINISIGN=1" in result.stderr
    assert "unavailable" in result.stderr
