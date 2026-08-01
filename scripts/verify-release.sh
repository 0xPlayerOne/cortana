#!/usr/bin/env bash
set -euo pipefail

archive="${1:-}"
checksum="${2:-${archive}.sha256}"

if [[ -z "$archive" || ! -f "$archive" ]]; then
  echo "usage: $0 ARCHIVE.tar.gz [ARCHIVE.tar.gz.sha256]" >&2
  exit 2
fi
if [[ ! -f "$checksum" ]]; then
  echo "checksum file is missing: $checksum" >&2
  exit 1
fi

sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "neither shasum nor sha256sum is available" >&2
    return 1
  fi
}

expected="$(awk 'NF { print $1; exit }' "$checksum")"
actual="$(sha256 "$archive")"
expected_lower="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
actual_lower="$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')"
if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ || "$expected_lower" != "$actual_lower" ]]; then
  echo "release checksum mismatch for $archive" >&2
  exit 1
fi

listing="$(tar -tzf "$archive")"
root="$(printf '%s\n' "$listing" | awk -F/ 'NF { print $1; exit }')"
if [[ -z "$root" || "$root" == .* || "$root" == /* ]]; then
  echo "release archive has no safe top-level directory" >&2
  exit 1
fi
while IFS= read -r entry; do
  [[ -n "$entry" ]] || continue
  if [[ "$entry" == /* || "$entry" == *'../'* || "$entry" == '../'* || "$entry" == *'/..' ]]; then
    echo "release archive contains an unsafe path: $entry" >&2
    exit 1
  fi
  if [[ "$entry" != "$root" && "$entry" != "$root/"* ]]; then
    echo "release archive contains multiple top-level roots" >&2
    exit 1
  fi
done <<< "$listing"

staging="$(mktemp -d "${TMPDIR:-/tmp}/cortana-release.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
tar -xzf "$archive" -C "$staging"
package="$staging/$root"

for required in \
  "$package/bin/cortana" \
  "$package/install.sh" \
  "$package/share/cortana/web/index.html" \
  "$package/config.example.toml" \
  "$package/skills/cortana/SKILL.md"; do
  [[ -f "$required" ]] || {
    echo "release archive is missing: ${required#"$package/"}" >&2
    exit 1
  }
done
[[ -x "$package/bin/cortana" ]] || {
  echo "release binary is not executable" >&2
  exit 1
}
wheel="$(find "$package/dist" -maxdepth 1 -type f -name '*.whl' -print -quit)"
[[ -n "$wheel" ]] || {
  echo "release archive has no connector wheel" >&2
  exit 1
}

host="$(uname -s)"
run_binary=false
case "$root" in
  *-unknown-linux-gnu) [[ "$host" == "Linux" ]] && run_binary=true ;;
  *-apple-darwin) [[ "$host" == "Darwin" ]] && run_binary=true ;;
esac
if "$run_binary"; then
  "$package/bin/cortana" --version >/dev/null
else
  echo "Verified archive structure and checksum; skipped cross-platform binary execution"
fi
echo "Verified Cortana release archive: $archive"
