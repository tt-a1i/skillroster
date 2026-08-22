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

echo "release governance smoke passed"
