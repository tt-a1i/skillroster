#!/usr/bin/env bash
set -euo pipefail

ledger="docs/acceptance.md"
readme_zh="README.md"
readme_en="README.en.md"

row_values() {
  local label="$1"
  awk -F '|' -v label="$label" '
    {
      name = $2
      exposure = $3
      duplicates = $4
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", name)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", exposure)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", duplicates)
      if (name == label) {
        print exposure " " duplicates
        found = 1
        exit
      }
    }
    END { if (!found) exit 1 }
  ' "$ledger"
}

require_literal() {
  local file="$1"
  local literal="$2"
  grep -Fq -- "$literal" "$file" || {
    echo "$file is missing value-evidence text: $literal" >&2
    exit 1
  }
}

read -r unmanaged_exposure unmanaged_duplicates < <(row_values "Unmanaged")
read -r manual_exposure manual_duplicates < <(row_values "Careful manual")
read -r roster_exposure roster_duplicates < <(row_values "SkillRoster Apply")

require_literal "$readme_en" "| Unmanaged | $unmanaged_exposure | $unmanaged_duplicates |"
require_literal "$readme_en" "| Careful manual governance | $manual_exposure | $manual_duplicates |"
require_literal "$readme_en" "| After SkillRoster Apply | **$roster_exposure** | **$roster_duplicates** |"
require_literal "$readme_en" "Undo restored the Agent tree byte-for-byte"
require_literal "$readme_en" "counts do not measure token, labor or task-quality"
require_literal "$readme_en" "does not estimate runtime tokens"

require_literal "$readme_zh" "| 未治理 | $unmanaged_exposure | $unmanaged_duplicates |"
require_literal "$readme_zh" "| 谨慎人工治理 | $manual_exposure | $manual_duplicates |"
require_literal "$readme_zh" "| SkillRoster Apply 后 | **$roster_exposure** | **$roster_duplicates** |"
require_literal "$readme_zh" "Undo 按字节恢复 Agent tree"
require_literal "$readme_zh" "不代表实际 token、人工成本或任务质量收益"
require_literal "$readme_zh" "本地磁盘实测 UTF-8 字节数"
require_literal "$readme_zh" 'skillroster find --hint "review a Pull Request on GitHub" --load --limit 1 --json -- "审查这个 Pull Request"'

for readme in "$readme_zh" "$readme_en"; do
  require_literal "$readme" "skillroster check"
  require_literal "$readme" "skillroster init --agent codex"
  require_literal "$readme" "skillroster mcp"
done
