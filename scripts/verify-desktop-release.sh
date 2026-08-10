#!/usr/bin/env bash
set -euo pipefail

tag="${1:-${RELEASE_TAG:-}}"
repo="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
if [[ -z "$tag" || -z "$repo" ]]; then
  echo "usage: GH_REPO=owner/repo $0 TAG" >&2
  exit 2
fi
version="${tag#v}"
app_archive="Cortana_${version}_aarch64.app.tar.gz"

assets_json="$(gh release view "$tag" --repo "$repo" --json assets)"
python3 - "$assets_json" "$tag" <<'PY'
import json
import sys

payload = json.loads(sys.argv[1])
tag = sys.argv[2]
version = tag[1:] if tag.startswith("v") else tag
assets = {
    asset["name"]: asset
    for asset in payload.get("assets", [])
    if isinstance(asset, dict) and isinstance(asset.get("name"), str)
}

required = {
    f"cortana-{tag}-aarch64-apple-darwin.tar.gz",
    f"cortana-{tag}-aarch64-apple-darwin.tar.gz.sha256",
    f"cortana-{tag}-x86_64-unknown-linux-gnu.tar.gz",
    f"cortana-{tag}-x86_64-unknown-linux-gnu.tar.gz.sha256",
    f"Cortana_{version}_aarch64.dmg",
    f"Cortana_{version}_aarch64.app.tar.gz",
    f"Cortana_{version}_aarch64.app.tar.gz.sig",
    f"Cortana_{version}_amd64.AppImage",
    f"Cortana_{version}_amd64.AppImage.sig",
    f"Cortana_{version}_amd64.deb",
    f"Cortana_{version}_amd64.deb.sig",
    f"Cortana-{version}-1.x86_64.rpm",
    f"Cortana-{version}-1.x86_64.rpm.sig",
    f"Cortana_{version}_x64-setup.exe",
    f"Cortana_{version}_x64-setup.exe.sig",
    f"Cortana_{version}_x64_en-US.msi",
    f"Cortana_{version}_x64_en-US.msi.sig",
    "latest.json",
}
missing = sorted(required.difference(assets))
if missing:
    raise SystemExit("release is missing assets: " + ", ".join(missing))

for name, asset in assets.items():
    if name.endswith((".sig", ".sha256")) and asset.get("size", 0) <= 0:
        raise SystemExit(f"release signature/checksum is empty: {name}")

print(f"verified {len(required)} Cortana release assets for {tag}")
PY

staging="$(mktemp -d "${TMPDIR:-/tmp}/cortana-release-assets.XXXXXX")"
trap 'rm -rf "$staging"' EXIT

minisign_bin="${CORTANA_MINISIGN_BIN:-minisign}"
require_minisign="${CORTANA_REQUIRE_MINISIGN:-0}"
case "$require_minisign" in
    0|1) ;;
    *)
        echo "CORTANA_REQUIRE_MINISIGN must be 0 or 1" >&2
        exit 2
        ;;
esac

signed_archives=(
    "$app_archive"
    "Cortana_${tag#v}_amd64.AppImage"
    "Cortana_${tag#v}_amd64.deb"
    "Cortana-${tag#v}-1.x86_64.rpm"
    "Cortana_${tag#v}_x64-setup.exe"
    "Cortana_${tag#v}_x64_en-US.msi"
)

# Bounded retry wrapper around `gh release download`: a single transient
# network failure must not fail the whole verification run. This is the real
# v0.29.39 incident: all 18 assets were present and healthy, but one download
# hit a connection reset. The attempt budget is small and configurable, and
# CORTANA_DOWNLOAD_RETRY_DELAY=0 lets tests exercise retries without sleeping.
# Every attempt also has a hard timeout so a wedged GitHub CLI cannot stall the
# release gate indefinitely.
# Failures still exit nonzero once the budget is exhausted, so missing or
# invalid assets are never hidden by the retry.
download_attempts="${CORTANA_DOWNLOAD_ATTEMPTS:-3}"
download_retry_delay="${CORTANA_DOWNLOAD_RETRY_DELAY:-2}"
download_timeout="${CORTANA_DOWNLOAD_TIMEOUT_SECONDS:-120}"
if ! [[ "$download_attempts" =~ ^[1-9][0-9]*$ ]]; then
    echo "CORTANA_DOWNLOAD_ATTEMPTS must be a positive integer no greater than 5" >&2
    exit 2
fi
if ((download_attempts > 5)); then
    echo "CORTANA_DOWNLOAD_ATTEMPTS must be a positive integer no greater than 5" >&2
    exit 2
fi
if ! [[ "$download_retry_delay" =~ ^[0-9]+$ ]]; then
    echo "CORTANA_DOWNLOAD_RETRY_DELAY must be a non-negative integer no greater than 60" >&2
    exit 2
fi
if ((download_retry_delay > 60)); then
    echo "CORTANA_DOWNLOAD_RETRY_DELAY must be a non-negative integer no greater than 60" >&2
    exit 2
fi
if ! [[ "$download_timeout" =~ ^[1-9][0-9]*$ ]]; then
    echo "CORTANA_DOWNLOAD_TIMEOUT_SECONDS must be a positive integer no greater than 600" >&2
    exit 2
fi
if ((download_timeout > 600)); then
    echo "CORTANA_DOWNLOAD_TIMEOUT_SECONDS must be a positive integer no greater than 600" >&2
    exit 2
fi

run_gh_download() {
    local timeout_seconds="$1"
    shift
    python3 - "$timeout_seconds" "$@" <<'PY'
import subprocess
import sys

timeout = float(sys.argv[1])
command = ["gh", *sys.argv[2:]]
try:
    result = subprocess.run(command, stdout=subprocess.DEVNULL, check=False, timeout=timeout)
except FileNotFoundError:
    print("gh release download failed: gh is not installed", file=sys.stderr)
    raise SystemExit(127)
except subprocess.TimeoutExpired:
    print(
        f"gh release download timed out after {sys.argv[1]}s: {' '.join(command)}",
        file=sys.stderr,
    )
    raise SystemExit(124)
raise SystemExit(result.returncode)
PY
}

gh_download() {
    local attempt
    for ((attempt = 1; attempt <= download_attempts; attempt++)); do
        if run_gh_download "$download_timeout" release download "$@"; then
            return 0
        fi
        if ((attempt < download_attempts)); then
            echo "gh release download failed (attempt ${attempt}/${download_attempts}); retrying in ${download_retry_delay}s" >&2
            sleep "$download_retry_delay"
        fi
    done
    echo "gh release download failed after ${download_attempts} attempts: $*" >&2
    return 1
}

core_archives=(
    "cortana-${tag}-aarch64-apple-darwin.tar.gz"
    "cortana-${tag}-x86_64-unknown-linux-gnu.tar.gz"
)
for asset in "${core_archives[@]}" "${signed_archives[@]}"; do
    gh_download "$tag" --repo "$repo" --pattern "$asset" --dir "$staging" --clobber
done
for archive in "${signed_archives[@]}"; do
    gh_download "$tag" --repo "$repo" --pattern "${archive}.sig" --dir "$staging" --clobber
done
for archive in "${core_archives[@]}"; do
    gh_download "$tag" --repo "$repo" --pattern "${archive}.sha256" --dir "$staging" --clobber
done

for archive in "${core_archives[@]}"; do
    (cd "$staging" && shasum -a 256 -c "${archive}.sha256")
done
echo "verified core archive checksums"

python3 - "$staging" "${core_archives[@]}" "${signed_archives[@]}" <<'PY'
import os
import sys
import tarfile
from pathlib import PurePosixPath

root = sys.argv[1]
core_archives = sys.argv[2:4]
signed_archives = sys.argv[4:]
app_archive = signed_archives[0]


def path_matches(name, expected):
    name = name.strip("/")
    expected = expected.strip("/")
    return (
        name == expected
        or name.endswith("/" + expected)
        or name.startswith(expected + "/")
        or ("/" + expected + "/") in ("/" + name + "/")
    )


def verify_archive(path, label, required):
    with tarfile.open(path, "r:gz") as archive:
        members = []
        for member in archive.getmembers():
            normalized = PurePosixPath(member.name)
            if (
                normalized.is_absolute()
                or ".." in normalized.parts
                or "\\" in member.name
                or member.issym()
                or member.islnk()
            ):
                raise SystemExit(f"{label} contains an unsafe archive member: {member.name}")
            members.append((normalized.as_posix().lstrip("./"), member))

    for expected, kind in required:
        matches = [
            member for name, member in members if path_matches(name, expected)
        ]
        if not matches:
            raise SystemExit(f"{label} is missing {expected}")
        if kind == "file" and not any(member.isfile() for member in matches):
            raise SystemExit(f"{label} does not contain a regular file for {expected}")
        if kind == "executable" and not any(
            member.isfile() and member.mode & 0o111 for member in matches
        ):
            raise SystemExit(f"{label} contains a non-executable {expected}")


for filename in core_archives:
    verify_archive(
        os.path.join(root, filename),
        filename,
        (
            ("bin/cortana", "executable"),
            ("install.sh", "executable"),
            ("config.example.toml", "file"),
            ("share/cortana/web/index.html", "file"),
            ("dist", "prefix"),
            ("scripts/install-release.sh", "file"),
            ("scripts/verify-release.sh", "file"),
        ),
    )

verify_archive(
    os.path.join(root, app_archive),
    app_archive,
    (
        ("Contents/MacOS/cortana", "executable"),
        ("Contents/MacOS/cortana-desktop", "executable"),
        ("Contents/Resources/resources/cortana-connectors", "prefix"),
    ),
)
print("verified core archives and macOS sidecar/resources")
PY

if command -v "$minisign_bin" >/dev/null 2>&1; then
    updater_config="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)/apps/desktop/src-tauri/tauri.conf.json"
    updater_public_key="$staging/tauri-updater.pub"
    python3 - "$updater_config" "$updater_public_key" "$staging" "${signed_archives[@]}" <<'PY'
import base64
import json
import sys
from pathlib import Path

config_path = Path(sys.argv[1])
public_key_path = Path(sys.argv[2])
staging_path = Path(sys.argv[3])
signed_archives = sys.argv[4:]
try:
    config = json.loads(config_path.read_text(encoding="utf-8"))
    encoded_public_key = config["plugins"]["updater"]["pubkey"]
    if not isinstance(encoded_public_key, str):
        raise TypeError("updater pubkey must be a string")
    public_key = base64.b64decode(encoded_public_key, validate=True)
except (OSError, UnicodeError, KeyError, TypeError, ValueError) as error:
    raise SystemExit(f"invalid Tauri updater key encoding: {error}") from error

if not public_key.startswith(b"untrusted comment: minisign public key: "):
    raise SystemExit("Tauri updater pubkey is not a minisign public key")
public_key_path.write_bytes(public_key.rstrip(b"\n") + b"\n")
for archive in signed_archives:
    signature_path = staging_path / f"{archive}.sig"
    decoded_signature_path = staging_path / f"{archive}.sig.minisig"
    try:
        encoded_signature = signature_path.read_bytes().strip()
        signature = base64.b64decode(encoded_signature, validate=True)
    except (OSError, UnicodeError, ValueError) as error:
        raise SystemExit(f"invalid Tauri updater signature encoding for {archive}: {error}") from error
    if not signature.startswith(b"untrusted comment: signature from tauri secret key\n"):
        raise SystemExit(f"Tauri updater signature is not a minisign signature: {archive}")
    decoded_signature_path.write_bytes(signature.rstrip(b"\n") + b"\n")
PY
    for archive in "${signed_archives[@]}"; do
        if ! "$minisign_bin" -Vm "$staging/$archive" \
            -x "$staging/$archive.sig.minisig" -p "$updater_public_key"; then
            echo "Tauri updater signature verification failed: $archive" >&2
            exit 1
        fi
        echo "verified Tauri updater signature: $archive"
    done
elif [[ "$require_minisign" == "1" ]]; then
    echo "CORTANA_REQUIRE_MINISIGN=1 but minisign verifier is unavailable" >&2
    exit 1
else
    echo "skipped Tauri updater signature verification: minisign verifier unavailable"
fi

gh_download "$tag" --repo "$repo" --pattern latest.json --dir "$staging" --clobber
python3 - "$staging/latest.json" "$tag" "$assets_json" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
tag = sys.argv[2]
release_assets = json.loads(sys.argv[3]).get("assets", [])
version = tag[1:] if tag.startswith("v") else tag
if manifest.get("version") != version:
    raise SystemExit(
        f"updater manifest version mismatch: expected {version}, got {manifest.get('version')}"
    )
platforms = manifest.get("platforms")
required = {
    "darwin-aarch64",
    "darwin-aarch64-app",
    "linux-x86_64",
    "linux-x86_64-appimage",
    "linux-x86_64-deb",
    "linux-x86_64-rpm",
    "windows-x86_64",
    "windows-x86_64-msi",
    "windows-x86_64-nsis",
}
if not isinstance(platforms, dict) or not required.issubset(platforms):
    missing = sorted(required.difference(platforms or {}))
    raise SystemExit("updater manifest is missing platforms: " + ", ".join(missing))
known_asset_urls = {
    url
    for asset in release_assets
    if isinstance(asset, dict)
    for url in (asset.get("url"), asset.get("apiUrl"))
    if isinstance(url, str) and url
}
for platform in required:
    entry = platforms[platform]
    if not isinstance(entry, dict) or not entry.get("url") or not entry.get("signature"):
        raise SystemExit(f"updater manifest entry is incomplete: {platform}")
    url = entry["url"]
    if f"/download/{tag}/" not in url and url not in known_asset_urls:
        raise SystemExit(f"updater URL points at the wrong release: {platform}")
print(f"verified updater manifest for {tag}")
PY

# Final installed-vs-published version gate: execute the published Linux core
# binary and assert that its own --version output matches the release tag, so
# a stale checkout build, an unpromoted version, or a mis-uploaded archive can
# never pass the published-asset gate. The verifier cannot run foreign-OS
# binaries, so non-Linux hosts skip execution exactly like verify-release.sh.
if [[ "$(uname -s)" == "Linux" ]]; then
  linux_archive="cortana-${tag}-x86_64-unknown-linux-gnu.tar.gz"
  linux_stage="$staging/linux"
  mkdir -p "$linux_stage"
  tar -xzf "$staging/$linux_archive" -C "$linux_stage"
  binary="$(find "$linux_stage" -type f -path '*/bin/cortana' -print -quit)"
  if [[ -z "$binary" || ! -x "$binary" ]]; then
    echo "published Linux core archive is missing an executable bin/cortana" >&2
    exit 1
  fi
  reported="$("$binary" --version)"
  expected="cortana ${version}"
  if [[ "$reported" != "$expected" ]]; then
    echo "published Linux binary version mismatch: expected '$expected', got '$reported'" >&2
    exit 1
  fi
  echo "verified published Linux binary version matches ${tag}"
else
  echo "skipped published binary execution on non-Linux host"
fi
