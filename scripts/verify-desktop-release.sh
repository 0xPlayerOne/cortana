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

core_archives=(
    "cortana-${tag}-aarch64-apple-darwin.tar.gz"
    "cortana-${tag}-x86_64-unknown-linux-gnu.tar.gz"
)
for asset in "${core_archives[@]}" "Cortana_aarch64.app.tar.gz"; do
    gh release download "$tag" --repo "$repo" --pattern "$asset" --dir "$staging" --clobber >/dev/null
done
for archive in "${core_archives[@]}"; do
    gh release download "$tag" --repo "$repo" --pattern "${archive}.sha256" --dir "$staging" --clobber >/dev/null
done

for archive in "${core_archives[@]}"; do
    (cd "$staging" && shasum -a 256 -c "${archive}.sha256")
done
echo "verified core archive checksums"

python3 - "$staging" "${core_archives[@]}" "Cortana_aarch64.app.tar.gz" <<'PY'
import os
import sys
import tarfile
from pathlib import PurePosixPath

root = sys.argv[1]
core_archives = sys.argv[2:4]
app_archive = sys.argv[4]


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
