---
name: skillroster
description: >
  Inspect and govern local Skills: inventory, usage evidence, duplicates,
  broken links, Core/On-demand proposals, Plans, Receipts, Apply, and Undo.
  Also find a specialized local Skill when the user requests one or the task
  would benefit from instructions beyond the visible catalog. Ordinary work
  can proceed with the Agent's existing capabilities. Explicit shared Skill
  adoption uses a reviewed SkillRoster Plan. Not for third-party installation
  or general shared-directory synchronization and repair.
metadata:
  bootstrap-version: "1.8.54"
  skillroster-routing-triggers: "route task to local Skill; inventory installed Agent Skills; analyze duplicate or unused Agent Skills; govern a Skill Roster; prepare or apply approved Skill Plan; create or undo Skill Receipt"
---

# SkillRoster

Use the local `skillroster` binary as the deterministic source of facts. Every
command uses explicit `--json`. Validate `schema_version` and `ok` before using
`result`. Typed `suggested_actions` are options, never authorization.

## Choose one path

- Inventory, usage, duplicates or Roster proposals: read `references/governance.md`.
- Setup, Apply, Undo, lifecycle or recovery: read `references/mutation.md`.
- Another task: use an already-visible exact Skill when applicable. Search
  below when specialized instructions would help; otherwise use normal tools.

The host Agent controls activation. SkillRoster returns local facts and verified
instructions; retrieval does not install, expose, endorse or authorize a Skill.

## Find a specialized Skill

Find searches the latest Snapshot, including On-demand, provider and source-only
Skills. Read-only task context may inform the search.

1. Keep the complete user message as `TASK`, byte-for-byte in its original
   language, including paths, limits, and output requirements. Do not summarize,
   translate, shorten, or quote only part of it.
2. If `TASK` is non-English or mixed-language, create one concise English
   capability paraphrase as `HINT`. It supplements `TASK`; it never replaces it.
3. Invoke the fixed SkillRoster executable with `TASK` and `HINT` as separate,
   literal argv values; never interpolate either into shell syntax. For a user-required
   named Skill, perform the required-name check below first. Otherwise use:
   - English: `skillroster find --load --limit 1 --json -- "TASK"`
   - Otherwise: `skillroster find --hint "HINT" --load --limit 1 --json -- "TASK"`
   The `--` boundary is required even when `TASK` does not begin with `-`; it
   keeps opaque user text from being parsed as a SkillRoster option.
   **Required-name check:** first use the corresponding Find above without `--load`
   and with `--limit 5`. Match the user's required name to returned `name`, keeping
   `skill_id`, `rank`, `paths`, any `providers`, and `result.snapshot_id` together.
   Resolve any user-specified provider or path from those same public facts.
   Once the intended identity is unambiguous and rank 1, repeat the same Find with
   `--load --limit 1 --require-snapshot SNAPSHOT_ID`, preserving executable, context,
   `TASK`, and hints. Require `loaded_skill.selection.skill_id` to equal that ID
   and its `snapshot_id` to equal the inspected Snapshot before using the content.
   A name missing from these bounded results remains unresolved. For that case,
   lower rank, or ambiguity, follow the bounded recovery below and
   `references/routing.md`; keep the required identity unresolved until established.
   Use variant selectors only within that intended Top-1 same-name group.
   A semantically similar candidate does not satisfy the user's required name.
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

Empty, wrong-domain, weak-evidence and unresolved required-name results share
one hint-refinement retry. Follow `references/routing.md`, keeping `TASK` intact.
After that, continue with normal tools if the Skill is optional; for a required
Skill, request its missing prerequisite. Permission denial, untrusted source,
drift, ambiguity and incomplete loads retain their typed recovery branches.
They cannot be bypassed by reading or executing the package.

Inspection and Plan preparation change no Agent files. Before Apply or Undo,
show complete bounded impact and obtain one explicit confirmation, following
`references/mutation.md`. Use CLI setup/confirmation/Apply even when read-only
MCP tools are available; human-only `init` is not an Agent execution path.

Never parse styled terminal output, invent a health score, infer token savings,
claim unobserved usage, or weaken ambiguity, drift, trust, and recovery checks.
In the final response, state whether files changed; claim changes only from a
verified Receipt or from task tools actually used after routing.
