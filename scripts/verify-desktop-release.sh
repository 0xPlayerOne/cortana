#!/usr/bin/env bash
set -euo pipefail

tag="${1:-${RELEASE_TAG:-}}"
repo="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
if [[ -z "$tag" || -z "$repo" ]]; then
  echo "usage: GH_REPO=owner/repo $0 TAG" >&2
  exit 2
fi

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
    "Cortana_aarch64.app.tar.gz",
    "Cortana_aarch64.app.tar.gz.sig",
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
gh release download "$tag" --repo "$repo" --pattern latest.json --dir "$staging" --clobber >/dev/null
python3 - "$staging/latest.json" "$tag" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
tag = sys.argv[2]
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
for platform in required:
    entry = platforms[platform]
    if not isinstance(entry, dict) or not entry.get("url") or not entry.get("signature"):
        raise SystemExit(f"updater manifest entry is incomplete: {platform}")
    if f"/download/{tag}/" not in entry["url"]:
        raise SystemExit(f"updater URL points at the wrong release: {platform}")
print(f"verified updater manifest for {tag}")
PY
