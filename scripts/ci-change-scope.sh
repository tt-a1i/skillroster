#!/usr/bin/env bash
set -euo pipefail

classify_paths() {
  local full=false
  local saw_path=false
  local path
  while IFS= read -r path; do
    [[ -z "$path" ]] && continue
    saw_path=true
    case "$path" in
      skill/skillroster/*)
        full=true
        ;;
      LICENSE|*.md)
        ;;
      *)
        full=true
        ;;
    esac
  done
  if [[ "$saw_path" == false ]]; then
    full=true
  fi
  printf '%s\n' "$full"
}

self_test() {
  local actual
  actual="$(printf '%s\n' 'README.md' 'docs/product-spec.md' 'LICENSE' | classify_paths)"
  [[ "$actual" == false ]]

  for runtime_path in \
    'skill/skillroster/SKILL.md' \
    'skill/skillroster/references/routing.md' \
    'skill/skillroster/manifest.json' \
    'src/bootstrap.rs'; do
    actual="$(printf '%s\n' "$runtime_path" | classify_paths)"
    [[ "$actual" == true ]]
  done

  actual="$(printf '%s\n' 'README.md' 'src/lib.rs' | classify_paths)"
  [[ "$actual" == true ]]
  actual="$(printf '' | classify_paths)"
  [[ "$actual" == true ]]
}

if [[ "${1:-}" == '--self-test' ]]; then
  self_test
  exit 0
fi

classify_paths
