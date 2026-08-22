#!/usr/bin/env bash
set -euo pipefail

target="${1:?target is required}"
version="${2:?version is required}"

case "$target" in
  x86_64-unknown-linux-gnu|aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "unsupported release target: $target" >&2; exit 1 ;;
esac
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$ ]] || [[ "$version" == *..* ]]; then
  echo "version must be SemVer without build metadata" >&2
  exit 1
fi

name="skillroster-${version}-${target}"
repo_root="$(pwd -P)"
mkdir -p dist
dist_root="$(cd dist && pwd -P)"
if [[ "$dist_root" != "$repo_root/dist" ]]; then
  echo "dist must be a real directory inside the repository" >&2
  exit 1
fi
stage="${dist_root}/${name}"
archive="${dist_root}/${name}.tar.gz"
checksum="${archive}.sha256"
case "$stage" in
  "$dist_root"/*) ;;
  *) echo "staging path escapes dist" >&2; exit 1 ;;
esac
if [[ -e "$stage" || -L "$stage" || -e "$archive" || -L "$archive" || -e "$checksum" || -L "$checksum" ]]; then
  echo "refusing to overwrite an existing staging path or artifact" >&2
  exit 1
fi

mkdir -p "$stage"
install -m 0755 "target/${target}/release/skillroster" "$stage/skillroster"
cp README.md "$stage/README.md"
tar -C "$dist_root" -czf "$archive" "$name"
rm -rf "$stage"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$dist_root" && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256")
else
  (cd "$dist_root" && shasum -a 256 "${name}.tar.gz" > "${name}.tar.gz.sha256")
fi
