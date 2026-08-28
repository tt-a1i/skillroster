#!/usr/bin/env bash
set -euo pipefail

formula="Formula/skillroster.rb"
manifest="Cargo.toml"
bootstrap="skill/skillroster/SKILL.md"
installation="docs/installation.md"
website="website/index.html"

published_version="$(awk -F '"' '/^[[:space:]]*version "/ { print $2; exit }' "$formula")"
candidate_version="$(awk -F '"' '/^version = "/ { print $2; exit }' "$manifest")"
bootstrap_version="$(awk -F '"' '/^[[:space:]]*bootstrap-version:/ { print $2; exit }' "$bootstrap")"

[[ "$published_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "could not derive the published version from $formula" >&2
  exit 1
}
[[ "$candidate_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "could not derive the source candidate version from $manifest" >&2
  exit 1
}
[[ "$bootstrap_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "could not derive the bundled Bootstrap version from $bootstrap" >&2
  exit 1
}

require_literal() {
  local file="$1"
  local literal="$2"
  grep -Fq -- "$literal" "$file" || {
    echo "$file is missing installation-surface text: $literal" >&2
    exit 1
  }
}

reject_literal() {
  local file="$1"
  local literal="$2"
  if grep -Fq -- "$literal" "$file"; then
    echo "$file contains forbidden installation-surface text: $literal" >&2
    exit 1
  fi
}

require_literal "$installation" "The current public release is **v${published_version}**."
require_literal "$installation" "SKILLROSTER_VERSION=${published_version}"
require_literal "$installation" "--tag v${published_version} skillroster"
require_literal "$installation" "Published CLI v${published_version} bundles Bootstrap content version ${published_version}."
require_literal "$installation" "source-tree candidate is **v${candidate_version}**"
require_literal "$installation" "Its bundled Bootstrap content version is ${bootstrap_version}."

require_literal "$website" "CURRENT RELEASE v${published_version}"
require_literal "$website" "IMMUTABLE CURRENT RELEASE · v${published_version}"
require_literal "$website" "--tag v${published_version} skillroster"
require_literal "$website" 'data-copy="brew install tt-a1i/skillroster/skillroster"'
require_literal "$website" "SOURCE TREE v${candidate_version}"
reject_literal "$website" "brew tap tt-a1i/skillroster https://github.com/tt-a1i/skillroster.git"
