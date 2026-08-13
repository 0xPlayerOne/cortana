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
archive_name="Cortana_${version}_aarch64.app.tar.gz"
staging="$(mktemp -d "${TMPDIR:-/tmp}/cortana-macos-package.XXXXXX")"
trap 'rm -rf -- "$staging"' EXIT

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

echo "verified macOS package core for $tag without launching the GUI"
