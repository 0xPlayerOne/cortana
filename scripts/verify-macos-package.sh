#!/usr/bin/env bash
set -euo pipefail

tag="${1:-${RELEASE_TAG:-}}"
repo="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
if [[ -z "$tag" || -z "$repo" ]]; then
  echo "usage: GH_REPO=owner/repo $0 TAG" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS package verification requires macOS (codesign/spctl unavailable)" >&2
  exit 2
fi

version="${tag#v}"
requested_arch="${CORTANA_MAC_ARCH:-$(uname -m)}"
case "$requested_arch" in
  arm64|aarch64)
    release_arch="aarch64"
    ;;
  x86_64|amd64)
    release_arch="x86_64"
    ;;
  *)
    echo "unsupported macOS architecture: $requested_arch (use arm64/aarch64 or x86_64)" >&2
    exit 2
    ;;
esac

archive_name="Cortana_${version}_${release_arch}.app.tar.gz"
signature_name="${archive_name}.sig"
staging="$(mktemp -d "${TMPDIR:-/tmp}/cortana-macos-package.XXXXXX")"
trap 'rm -rf -- "$staging"' EXIT

release_assets="$(gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name')"
if ! grep -Fxq "$archive_name" <<<"$release_assets"; then
  echo "release $tag does not publish the requested macOS $release_arch app archive ($archive_name)" >&2
  echo "set CORTANA_MAC_ARCH to a published architecture or add that release artifact" >&2
  exit 1
fi
if ! grep -Fxq "$signature_name" <<<"$release_assets"; then
  echo "release $tag is missing the macOS package signature ($signature_name)" >&2
  exit 1
fi

gh release download "$tag" --repo "$repo" --pattern "$archive_name" --pattern "$signature_name" --dir "$staging"

minisign_bin="${CORTANA_MINISIGN_BIN:-minisign}"
# Published updater signatures are part of the production package contract.
# Keep an explicit opt-out for offline fixture work, but fail closed by default
# when the verifier is unavailable.
require_minisign="${CORTANA_REQUIRE_MINISIGN:-1}"
case "$require_minisign" in
  0|1) ;;
  *)
    echo "CORTANA_REQUIRE_MINISIGN must be 0 or 1" >&2
    exit 2
    ;;
esac
if command -v "$minisign_bin" >/dev/null 2>&1; then
  updater_config="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)/apps/desktop/src-tauri/tauri.conf.json"
  python3 - "$updater_config" "$staging/$signature_name" "$staging/$signature_name.minisig" <<'PY'
import base64
import json
import sys
from pathlib import Path

config_path, signature_path, decoded_path = map(Path, sys.argv[1:])
try:
    config = json.loads(config_path.read_text(encoding="utf-8"))
    encoded_signature = signature_path.read_bytes().strip()
    signature = base64.b64decode(encoded_signature, validate=True)
except (OSError, UnicodeError, ValueError, TypeError, json.JSONDecodeError) as error:
    raise SystemExit(f"invalid macOS package signature encoding: {error}") from error

if not signature.startswith(b"untrusted comment: signature from tauri secret key\n"):
    raise SystemExit("macOS package signature is not a minisign signature")
decoded_path.write_bytes(signature.rstrip(b"\n") + b"\n")
try:
    encoded_key = config["plugins"]["updater"]["pubkey"]
    public_key = base64.b64decode(encoded_key, validate=True)
except (KeyError, TypeError, ValueError) as error:
    raise SystemExit(f"invalid Tauri updater public key: {error}") from error
if not public_key.startswith(b"untrusted comment: minisign public key: "):
    raise SystemExit("Tauri updater public key is not a minisign public key")
(decoded_path.parent / "tauri-updater.pub").write_bytes(public_key.rstrip(b"\n") + b"\n")
PY
  "$minisign_bin" -Vm "$staging/$archive_name" \
    -x "$staging/$signature_name.minisig" \
    -p "$staging/tauri-updater.pub"
  echo "verified Tauri updater signature: $archive_name"
elif [[ "$require_minisign" == "1" ]]; then
  echo "CORTANA_REQUIRE_MINISIGN=1 but minisign verifier is unavailable" >&2
  exit 1
else
  echo "skipped Tauri updater signature verification: minisign verifier unavailable" >&2
fi

python3 - "$staging/$archive_name" "$staging" <<'PY'
import sys
import tarfile
from pathlib import PurePosixPath

archive_path, destination = sys.argv[1:]
with tarfile.open(archive_path, "r:gz") as archive:
    for member in archive.getmembers():
        normalized = PurePosixPath(member.name)
        if (
            normalized.is_absolute()
            or ".." in normalized.parts
            or "\\" in member.name
            or member.issym()
            or member.islnk()
        ):
            raise SystemExit(f"macOS package contains an unsafe archive member: {member.name}")
    archive.extractall(destination)
PY
app="$staging/Cortana.app"
if [[ ! -d "$app" ]]; then
  echo "release archive did not contain Cortana.app" >&2
  exit 1
fi

bundle_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
[[ "$bundle_version" == "$version" ]] || {
  echo "bundle version mismatch: expected $version, got $bundle_version" >&2
  exit 1
}

core="$app/Contents/MacOS/cortana"
core_version="$("$core" --version)"
[[ "$core_version" == "cortana $version" ]] || {
  echo "bundled core version mismatch: expected cortana $version, got $core_version" >&2
  exit 1
}

"$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/verify-packaged-core.sh" "$core"

codesign --verify --deep --strict "$app"
echo "strict codesign verification passed: $app"

require_gatekeeper="${CORTANA_REQUIRE_GATEKEEPER:-0}"
case "$require_gatekeeper" in
  0|1) ;;
  *)
    echo "CORTANA_REQUIRE_GATEKEEPER must be 0 or 1" >&2
    exit 2
    ;;
esac

if spctl --assess --type execute "$app"; then
  echo "Gatekeeper assessment passed: Developer ID/notarization trust is available"
else
  echo "Gatekeeper assessment rejected: Developer ID/notarization is not configured" >&2
  if [[ "$require_gatekeeper" == "1" ]]; then
    exit 1
  fi
fi

echo "verified macOS $release_arch package core for $tag without launching the GUI"
