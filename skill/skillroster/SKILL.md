---
name: skillroster
description: >
  Route tasks to specialized local Core and On-demand Skills, and govern the
  local Skill roster. Before reading or changing the workspace for a task that
  may need instructions not already visible, preserve the complete user message
  verbatim as TASK. For non-English or mixed input, run
  `skillroster find "TASK" --hint "ONE FAITHFUL ENGLISH CAPABILITY PARAPHRASE" --load --limit 1 --json`;
  for English input omit `--hint`. Follow the complete verified SKILL.md in
  `result.loaded_skill.content.text`. Also use
  for inventory, usage evidence, duplicates, broken links, Core/On-demand
  recommendations, Plans, Receipts, Apply, and Undo. Not for installing
  third-party Skills or migrating, distributing, synchronizing, or repairing shared
  Skill-manager directories.
metadata:
  bootstrap-version: "1.8.28"
  skillroster-routing-triggers: "route task to local Skill; inventory installed Agent Skills; analyze duplicate or unused Agent Skills; govern a Skill Roster; prepare or apply approved Skill Plan; create or undo Skill Receipt"
---

# SkillRoster

Use the local `skillroster` binary as the deterministic source of facts. Every
command uses explicit `--json`. Validate `schema_version` and `ok` before using
`result`. Typed `suggested_actions` are options, never authorization.

## Choose one path

- For SkillRoster inventory, evidence, governance, setup, Apply, Undo, lifecycle,
  or recovery requests, skip the Route gate and read the matching reference
  below. Do not route SkillRoster back to itself.
- For another task, follow an already-visible exact Skill when one clearly
  applies. Otherwise complete the Route gate.

## Route gate

The Find call below must be the first task tool call. Before it, do not read or
change the workspace and do not execute a non-routing command:

1. Keep the complete user message as `TASK`, byte-for-byte in its original
   language, including paths, limits, and output requirements. Do not summarize,
   translate, shorten, or quote only part of it.
2. If `TASK` is non-English or mixed-language, create one concise English
   capability paraphrase as `HINT`. It supplements `TASK`; it never replaces it.
3. Invoke the fixed SkillRoster executable with `TASK` and `HINT` as separate,
   literal argv values; never interpolate either into shell syntax. Use:
   - English: `skillroster find "TASK" --load --limit 1 --json`
   - Otherwise: `skillroster find "TASK" --hint "HINT" --load --limit 1 --json`
4. When `HINT` was used, first require
   `ranking_strategy: task_hint_reciprocal_rank_fusion`.
5. Require `loaded_skill.selection.rank: 1`, `content.complete: true`, and all
   `verification` identity/digest checks to be true. The complete instructions
   are `loaded_skill.content.text`; no second filesystem, workspace, MCP, or
   SkillRoster read is needed. For a wrong-domain result or typed load blocker,
   read `references/routing.md` and follow its bounded branch.
6. Follow the loaded instructions and perform the original task. Treat
   `task_success: not_evaluated` literally; only the task's own evidence can
   establish success.

If the result is empty or clearly from another domain, retry once. Keep `TASK`
unchanged; refine the existing hint, or add one capability hint when the first
English call had none. If no usable result remains, stop and report a routing
failure.

## Govern a roster

For inventory, usage, exposure, duplicate, broken-link, Core/On-demand, or Plan
requests, read `references/governance.md` before acting. Inspection and planning
do not change Agent files. Prefer the bounded summary first; load exhaustive
detail only for a decision that needs it.

## Mutate or recover

For setup, Apply, Undo, lifecycle cleanup, or recovery, read
`references/mutation.md` before acting. A validated Plan is not permission to
apply it. Show the complete bounded impact and obtain one explicit confirmation.
Only a successful Receipt verifies that Agent files changed.

Never parse styled terminal output, invent a health score, infer token savings,
claim unobserved usage, or weaken ambiguity, drift, trust, and recovery checks.
In the final response, state whether files changed; claim changes only from a
verified Receipt or from task tools actually used after routing.
