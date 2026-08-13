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
staging="$(mktemp -d "${TMPDIR:-/tmp}/cortana-macos-package.XXXXXX")"
trap 'rm -rf -- "$staging"' EXIT

if ! gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name' \
  | grep -Fxq "$archive_name"; then
  echo "release $tag does not publish the requested macOS $release_arch app archive ($archive_name)" >&2
  echo "set CORTANA_MAC_ARCH to a published architecture or add that release artifact" >&2
  exit 1
fi

gh release download "$tag" --repo "$repo" --pattern "$archive_name" --dir "$staging"
tar -xzf "$staging/$archive_name" -C "$staging"
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

codesign --verify --deep --strict "$app"
echo "strict codesign verification passed: $app"

if spctl --assess --type execute "$app"; then
  echo "Gatekeeper assessment passed: Developer ID/notarization trust is available"
else
  echo "Gatekeeper assessment rejected: Developer ID/notarization is not configured" >&2
  if [[ "${CORTANA_REQUIRE_GATEKEEPER:-0}" == "1" ]]; then
    exit 1
  fi
fi

echo "verified macOS $release_arch package core for $tag without launching the GUI"
