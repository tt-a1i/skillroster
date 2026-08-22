#!/usr/bin/env bash
set -euo pipefail

archive="$1"
repo="$2"
extract_root="$(mktemp -d /tmp/skillroster-release.XXXXXX)"
trap 'rm -rf -- "$extract_root"' EXIT

tar -xzf "$archive" -C "$extract_root"
binary="$(find "$extract_root" -type f -name skillroster -print -quit)"
[[ -n "$binary" ]] || { echo "release archive has no skillroster binary" >&2; exit 1; }
chmod +x "$binary"
"$repo/scripts/release/smoke.sh" "$binary"
