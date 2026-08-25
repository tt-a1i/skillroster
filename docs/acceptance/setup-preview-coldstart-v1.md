# Bootstrap setup preview cold start

Observed on 2026-08-25 on macOS arm64. The before run used the public v1.8.27
release binary. The after run used a debug build from branch
`codex/issue-218-setup-bounded-summary` at base commit `43c5932`. Byte counts
include the trailing newline emitted by the CLI. They are observations for the
recorded temporary path lengths, not portable byte limits.

## Scenario

A fresh HOME exposed all eight supported Agent roots. Codex, Claude Code, and
Pi shared one physical Skill root; the other Agents used separate roots. Each
root contained one harmless fixture Skill. Public v1.8.27 performed Scan and
Bootstrap Setup without Apply.

## Before

- `setup --json`: 5,959 bytes, 8 logical targets, 6 physical targets, 36 planned
  operations.
- The response omitted ordinary Plan impact, operation groups, risk, and
  reversibility.
- Explaining the preview required `plan --show`, which returned 87,054 bytes,
  including 77,940 bytes of repeated Bootstrap file content.

No Agent files changed, one Plan remained pending, and no Receipt existed.

## Acceptance

`setup --json` now projects the immutable Plan's bounded `change_summary`,
`operation_groups`, `affected`, `diff_summary`, `impact`, `risk`,
`reversible`, and `detail` facts. It excludes complete operations, file bodies,
and fingerprints. An Agent can explain an ordinary Bootstrap preview from the
Setup response alone and reserve `plan --show` for exact-detail questions.

Setup remains preview-only: confirmation is required, `files_changed` is
false, and Apply/Undo semantics are unchanged.

The same isolated eight-Agent fixture returned a 6,985-byte Setup response
with all 36 operations summarized as 12 directory creations and 24 file
writes. It reported `filesystem_change`, `reversible: true`, and the exact
detail command without embedding `operations`. Status showed one pending Plan,
zero Receipts, and clear recovery. The ordinary explanation therefore needs
one 6,985-byte response instead of Setup plus an additional 87,054-byte detail
response.

## Reproduction

The isolated HOME contained the eight standard Agent roots. Codex, Claude
Code, and Pi linked to one shared physical root; OpenCode, Hermes, Cursor,
Gemini CLI, and GitHub Copilot used separate roots. With a fresh state
directory, the measured command sequence was:

```text
skillroster --home HOME --state-dir STATE --json scan
skillroster --home HOME --state-dir STATE --json setup
skillroster --home HOME --state-dir STATE --json status
```

The CLI regression test
`setup_deduplicates_shared_agent_roots_and_undo_restores_each_physical_root`
reconstructs the same 8/6/36 topology and verifies the bounded response,
pending Plan, absent Receipt, Apply, and Undo. A separate legacy-summary test
proves that an incomplete stored Bootstrap Plan is not reused.
