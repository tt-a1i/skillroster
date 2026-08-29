#!/usr/bin/env bash
set -euo pipefail

archive_readme="docs/release-archive/README.md"
release_version_pattern='(^|[^0-9])[vV]?[0-9]+\.[0-9]+\.[0-9]+([^0-9]|$)'

validate_regular_file_at_parent() {
  local candidate="$1"
  local expected_parent="$2"
  local actual_parent

  [[ -f "$candidate" && ! -L "$candidate" ]] || return 1
  actual_parent="$(cd "$(dirname "$candidate")" && pwd -P)" || return 1
  [[ "$actual_parent" == "$expected_parent" ]]
}

contains_release_version() {
  grep -Eq "$release_version_pattern"
}

if ! printf '%s\n' 'Current release: v1.8.33' | contains_release_version; then
  echo "release archive README version guard failed its positive control" >&2
  exit 1
fi
if printf '%s\n' 'Apache-2.0; run skillroster --version' | contains_release_version; then
  echo "release archive README version guard rejected its negative control" >&2
  exit 1
fi

path_test_root="$(mktemp -d "${TMPDIR:-/tmp}/skillroster-release-readme.XXXXXX")"
path_test_root="$(cd "$path_test_root" && pwd -P)"
trap 'rm -rf -- "$path_test_root"' EXIT
mkdir "$path_test_root/real"
printf '%s\n' 'version-neutral' > "$path_test_root/real/README.md"
validate_regular_file_at_parent \
  "$path_test_root/real/README.md" \
  "$path_test_root/real"
ln -s "$path_test_root/real/README.md" "$path_test_root/linked-readme.md"
if validate_regular_file_at_parent "$path_test_root/linked-readme.md" "$path_test_root"; then
  echo "release archive README path guard accepted a symlink" >&2
  exit 1
fi
ln -s "$path_test_root/real" "$path_test_root/linked-parent"
if validate_regular_file_at_parent \
  "$path_test_root/linked-parent/README.md" \
  "$path_test_root/linked-parent"; then
  echo "release archive README path guard accepted a linked parent" >&2
  exit 1
fi
rm -rf -- "$path_test_root"
trap - EXIT

repo_root="$(pwd -P)"
expected_archive_parent="$repo_root/docs/release-archive"
if ! validate_regular_file_at_parent "$archive_readme" "$expected_archive_parent"; then
  echo "release archive README must be a regular in-repository file without linked ancestors: $archive_readme" >&2
  exit 1
fi
if [[ "$(git check-attr eol -- "$archive_readme")" != "$archive_readme: eol: lf" ]]; then
  echo "release archive README must be checked out with LF on every platform" >&2
  exit 1
fi

if contains_release_version < "$archive_readme"; then
  echo "release archive README must not hard-code a release version" >&2
  exit 1
fi

for package_script in \
  scripts/release/package-unix.sh \
  scripts/release/package-windows.ps1; do
  if ! grep -Fq "$archive_readme" "$package_script"; then
    echo "release packager does not select the version-neutral README: $package_script" >&2
    exit 1
  fi
done
if ! grep -Fq 'https://github.com/tt-a1i/skillroster/releases' "$archive_readme"; then
  echo "release archive README does not link to public release evidence" >&2
  exit 1
fi
