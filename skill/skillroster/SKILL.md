---
name: skillroster
description: >
  Inspect and govern local Skills: inventory, usage evidence, duplicates,
  broken links, Core/On-demand proposals, Plans, Receipts, Apply, and Undo.
  Also find a specialized local Skill when the user requests one or the task
  would benefit from instructions beyond the visible catalog. Ordinary work
  can proceed with the Agent's existing capabilities. Not for installing
  third-party Skills or migrating, distributing, synchronizing, or repairing shared
  Skill-manager directories.
metadata:
  bootstrap-version: "1.8.49"
  skillroster-routing-triggers: "route task to local Skill; inventory installed Agent Skills; analyze duplicate or unused Agent Skills; govern a Skill Roster; prepare or apply approved Skill Plan; create or undo Skill Receipt"
---

# SkillRoster

Use the local `skillroster` binary as the deterministic source of facts. Every
command uses explicit `--json`. Validate `schema_version` and `ok` before using
`result`. Typed `suggested_actions` are options, never authorization.

## Choose one path

- For SkillRoster inventory, evidence, governance, setup, Apply, Undo, lifecycle,
  or recovery requests, read the matching reference below.
- For another task, follow an already-visible exact Skill when one clearly
  applies. Search below when a specialized Skill would help; otherwise proceed
  with the Agent's normal tools and the user's task.

SkillRoster is a local lookup and governance bridge, not a replacement for the
host Agent's Harness. Its contract is the CLI and its JSON; the host still
decides whether a Skill is listed, activated, or enforced. Do not call it for
ordinary work that needs no specialized procedure.

## Find a specialized Skill

Use this search before relying on a Skill that is absent from the visible
catalog. Read-only task context may inform whether a search is useful.

Find searches the latest completed local Snapshot, not only the default catalog.
It may return an On-demand, provider, or source-only Skill. Loading one returns
verified instructions to the caller; it does not activate, install, expose,
endorse, or establish success for the user's task.

1. Keep the complete user message as `TASK`, byte-for-byte in its original
   language, including paths, limits, and output requirements. Do not summarize,
   translate, shorten, or quote only part of it.
2. If `TASK` is non-English or mixed-language, create one concise English
   capability paraphrase as `HINT`. It supplements `TASK`; it never replaces it.
3. Invoke the fixed SkillRoster executable with `TASK` and `HINT` as separate,
   literal argv values; never interpolate either into shell syntax. Use:
   - English: `skillroster find --load --limit 1 --json -- "TASK"`
   - Otherwise: `skillroster find --hint "HINT" --load --limit 1 --json -- "TASK"`
   The `--` boundary is required even when `TASK` does not begin with `-`; it
   keeps opaque user text from being parsed as a SkillRoster option.
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

If Find returns `snapshot_required`, execute its returned `scan` action only when
both `mutates` and `requires_confirmation` are false, then retry the identical
Find call with the same `TASK` and `HINT`. The Scan changes SkillRoster's local
Snapshot, not Agent or Skill files. If either flag says otherwise, ask the user
first; these flags do not authorize a material change. For any other typed
blocker, read `references/routing.md` and follow its bounded branch; execute a
returned action only when one is present.

If the result is empty or clearly from another domain, retry at most once. Keep `TASK`
unchanged; refine the existing hint, or add one capability hint when the first
English call had none. If no usable result remains and no particular Skill is
required, continue the original task with normal tools. State that no suitable
Skill was found only when it affects the result. If the user or applicable
instructions require a particular Skill, explain its absence and request only
the missing prerequisite. A permission denial, untrusted source, drift,
ambiguity, or incomplete load follows its typed recovery action; it is not an
ordinary no-match and cannot be bypassed by reading or executing the package.

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
