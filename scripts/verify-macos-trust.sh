#!/usr/bin/env bash
set -euo pipefail

app="${1:-}"
expected_version="${2:-}"
expected_arch="${3:-}"

if [[ -z "$app" || -z "$expected_version" ]]; then
  echo "usage: $0 /path/to/Cortana.app VERSION [arm64|x86_64]" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS trust verification requires Darwin" >&2
  exit 2
fi
if [[ ! -d "$app" || "${app##*.}" != "app" ]]; then
  echo "application bundle is missing or not a .app directory: $app" >&2
  exit 1
fi

info_plist="$app/Contents/Info.plist"
desktop_binary="$app/Contents/MacOS/cortana-desktop"
[[ -f "$info_plist" ]] || { echo "Info.plist is missing: $info_plist" >&2; exit 1; }
[[ -x "$desktop_binary" ]] || {
  echo "packaged desktop executable is missing or not executable: $desktop_binary" >&2
  exit 1
}

bundle_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$info_plist")"
[[ "$bundle_version" == "$expected_version" ]] || {
  echo "bundle version mismatch: expected $expected_version, got $bundle_version" >&2
  exit 1
}

if [[ -n "$expected_arch" ]]; then
  case "$expected_arch" in
    arm64|aarch64) arch_pattern='arm64' ;;
    x86_64|amd64) arch_pattern='x86_64' ;;
    *) echo "unsupported expected macOS architecture: $expected_arch" >&2; exit 2 ;;
  esac
  file -b "$desktop_binary" | grep -Eq "$arch_pattern" || {
    echo "desktop executable architecture does not match expected $expected_arch" >&2
    exit 1
  }
fi

codesign_details="$(codesign -dv --verbose=4 "$app" 2>&1)"
printf '%s\n' "$codesign_details" | grep -Fq 'Authority=Developer ID Application:' || {
  echo "macOS bundle is not signed with a Developer ID Application identity" >&2
  exit 1
}
printf '%s\n' "$codesign_details" | grep -Eq 'flags=.*runtime' || {
  echo "macOS bundle is missing the hardened runtime code-signing flag" >&2
  exit 1
}
printf '%s\n' "$codesign_details" | grep -Fq 'Timestamp=' || {
  echo "macOS bundle signature is not timestamped" >&2
  exit 1
}

codesign --verify --deep --strict --verbose=2 "$app"
spctl --assess --type execute --strict --verbose=4 "$app"
xcrun stapler validate "$app"

echo "verified Developer ID, hardened runtime, timestamp, Gatekeeper, and stapling: $app"
