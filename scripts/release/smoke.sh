#!/usr/bin/env bash
set -euo pipefail

binary="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/skillroster-release-smoke.XXXXXX")"
home_root="$fixture/home"
state_root="$fixture/state"
skill_root="$home_root/.codex/skills"
common=(--home "$home_root" --state-dir "$state_root" --json)

cleanup() {
  rm -rf -- "$fixture"
}
trap cleanup EXIT

run_json() {
  local output
  output="$($binary "${common[@]}" "$@")"
  [[ "$output" == *'"ok":true'* && "$output" == *'"schema_version":1'* ]] || {
    echo "invalid Agent envelope: $output" >&2
    return 1
  }
  printf '%s\n' "$output"
}

mkdir -p "$skill_root"
scan="$(run_json scan)"
[[ "$scan" == *'"skill_count":0'* ]] || {
  echo "synthetic home must start with zero Skills" >&2
  exit 1
}

setup="$(run_json setup)"
plan_id="$(printf '%s' "$setup" | sed -n 's/.*"plan_id":"\([^"]*\)".*/\1/p')"
[[ -n "$plan_id" && "$setup" == *'"state":"preview_ready"'* ]] || {
  echo "setup did not produce a preview Plan" >&2
  exit 1
}

apply="$(run_json apply "$plan_id")"
receipt_id="$(printf '%s' "$apply" | sed -n 's/.*"receipt_id":"\([^"]*\)".*/\1/p')"
[[ -n "$receipt_id" && "$apply" == *'"verification":"passed"'* && -f "$skill_root/skillroster/SKILL.md" ]] || {
  echo "release Apply did not verify the bootstrap Skill" >&2
  exit 1
}

undo="$(run_json undo "$receipt_id")"
[[ "$undo" == *'"verification":"passed"'* && ! -e "$skill_root/skillroster" ]] || {
  echo "release Undo did not restore the synthetic Agent root" >&2
  exit 1
}

status="$(run_json status)"
[[ "$status" == *'"recovery_state":"clear"'* ]] || {
  echo "release smoke left recovery required" >&2
  exit 1
}

# Keep this fixture synthetic and local: it recreates the retained-ID collision
# reported by real-home dogfood without reading or copying any user data.
source_root="$fixture/external-source"
linked_skill="$home_root/.claude/skills/retained-external"
entrypoint="$linked_skill/SKILL.md"
mkdir -p "$source_root"
mkdir -p "$(dirname "$linked_skill")"
cat > "$source_root/SKILL.md" <<'EOF'
---
name: retained-external
description: release smoke fixture
---
EOF
ln -s "$source_root" "$linked_skill"
if command -v sha256sum >/dev/null 2>&1; then
  skill_digest="$(printf 'unreadable-link:%s' "$entrypoint" | sha256sum | awk '{print $1}')"
else
  skill_digest="$(printf 'unreadable-link:%s' "$entrypoint" | shasum -a 256 | awk '{print $1}')"
fi
skill_id="skill_${skill_digest}"
python3 - "$state_root/skillroster.db" "$skill_id" "$linked_skill" <<'PY'
import sqlite3
import sys

database, skill_id, linked_skill = sys.argv[1:]
connection = sqlite3.connect(database)
connection.execute(
    """INSERT INTO skills
       (id, identity_key, name, description, declared_source, declared_revision,
        content_digest, digest_version, governance_state, canonical_path)
       VALUES (?, ?, ?, NULL, NULL, NULL, ?, 1, 'managed', ?)""",
    (skill_id, "content:retained-strong-identity", "retained-external",
     "retained-package-digest", linked_skill),
)
connection.commit()
connection.close()
PY

collision_scan="$(run_json scan)"
[[ "$collision_scan" == *'"skill_count":1'* ]] || {
  echo "retained-ID collision scan did not preserve the fixture Skill" >&2
  exit 1
}
python3 - "$state_root/skillroster.db" "$skill_id" <<'PY'
import sqlite3
import sys

database, skill_id = sys.argv[1:]
connection = sqlite3.connect(database)
row = connection.execute(
    "SELECT COUNT(*), identity_key, governance_state FROM skills WHERE id = ?",
    (skill_id,),
).fetchone()
connection.close()
assert row == (1, "content:retained-strong-identity", "managed"), row
PY

repeat_scan="$(run_json scan)"
[[ "$repeat_scan" == *'"skill_count":1'* ]] || {
  echo "repeated retained-ID collision scan did not remain stable" >&2
  exit 1
}

echo "release governance smoke passed"
