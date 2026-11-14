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
published_revision="$(awk -F '"' '/^[[:space:]]*revision:/ { print $2; exit }' "$formula")"

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
[[ "$published_revision" =~ ^[0-9a-f]{40}$ ]] || {
  echo "could not derive the published revision from $formula" >&2
  exit 1
}
git cat-file -e "$published_revision^{commit}" 2>/dev/null || {
  echo "$formula revision $published_revision is unavailable; fetch repository history" >&2
  exit 1
}
published_bootstrap_version="$(
  git show "$published_revision:$bootstrap" |
    awk -F '"' '/^[[:space:]]*bootstrap-version:/ { print $2; exit }'
)"
[[ "$published_bootstrap_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "could not derive the published Bootstrap version at $published_revision" >&2
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
require_literal "$installation" "Published CLI v${published_version} bundles Bootstrap content version ${published_bootstrap_version}."
require_literal "$installation" "The source tree is **v${candidate_version}**."
require_literal "$installation" "Its bundled Bootstrap content version is ${bootstrap_version}."

require_literal "$website" "CURRENT RELEASE v${published_version}"
require_literal "$website" "IMMUTABLE CURRENT RELEASE · v${published_version}"
require_literal "$website" "--tag v${published_version} skillroster"
require_literal "$website" 'data-copy="brew install tt-a1i/skillroster/skillroster"'
require_literal "$website" "SOURCE TREE v${candidate_version}"
if [[ "$published_version" != "$candidate_version" ]]; then
  require_literal "$website" "SOURCE CANDIDATE v${candidate_version} · NOT YET PUBLISHED · BOOTSTRAP ${bootstrap_version}"
  require_literal README.md "v${candidate_version} 源码候选体验"
  require_literal README.en.md "v${candidate_version} source-candidate features"
else
  reject_literal "$website" "SOURCE CANDIDATE v${candidate_version} · NOT YET PUBLISHED"
fi
require_literal "$website" "PUBLIC: CLI v${published_version} / BOOTSTRAP ${published_bootstrap_version}"
for npm_manifest in npm/package.json npm/platforms/*/package.json; do
  require_literal "$npm_manifest" "\"version\": \"${candidate_version}\""
done
reject_literal "$website" "brew tap tt-a1i/skillroster https://github.com/tt-a1i/skillroster.git"

# Entry paths and npm packaging instructions must stay visible.
require_literal "$installation" 'skillroster check'
require_literal "$installation" 'skillroster init --agent codex'
require_literal "$installation" 'claude mcp add --transport stdio skillroster -- skillroster mcp'
require_literal "$installation" 'codex mcp add skillroster -- skillroster mcp'
require_literal "$installation" 'scripts/release/build-npm.py'
require_literal 'README.md' 'skillroster check'
require_literal 'README.en.md' 'skillroster check'
