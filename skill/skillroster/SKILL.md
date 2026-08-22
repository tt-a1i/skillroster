---
name: skillroster
description: Inspect, search, organize, apply, or undo governance for locally installed Agent Skills with the SkillRoster CLI. Use when the user asks which Skills are installed or used, wants duplicates or broken links analyzed, needs a smaller default Skill roster, wants an on-demand Skill found, or asks to apply or undo an approved Skill organization plan.
---

# SkillRoster

Use the local `skillroster` binary as the deterministic source of facts. Invoke every command with explicit `--json`; validate `schema_version` and `ok` before reading `result`. Treat typed `suggested_actions` as options, never authorization.

## Inspect and propose

1. Run `skillroster scan --json`, then `skillroster report --summary --json`. Use the compact result for the first user-facing diagnosis; do not request the full report unless exhaustive Finding enumeration is necessary.
2. Distinguish observed, inferred, and unknown usage. Missing evidence does not mean unused.
3. Use stable Finding and Evidence IDs for follow-up questions. The summary exposes one `primary_evidence_id`; use `report --finding ID --limit 20 --json` when the exact paths or Evidence affect the decision. Finding detail is paged. Continue with `--offset NEXT_OFFSET --limit 20` only until the requested decision has enough relevant evidence, and keep the same Snapshot rather than silently rescanning.
4. Make semantic governance choices in the conversation, then submit declarative target states to `skillroster plan --stdin --json`. Set request `schema_version` to `1`, include the latest `scan_id` and one or more relevant `evidence_ids`, and submit only the supported governance fields.
5. Present the validated Plan in one viewport: diagnosis, four core metrics, three main Findings, `change_summary`, `impact` before/after facts, affected Agents, uncertainty, canonical deletion count, reversibility, and Plan ID. The source-oriented `diff_summary` may be empty for ordinary Roster or Library Plans; do not treat that as a missing Plan delta.

Use only these Plan request families:

- `roster_changes`: `agent`, `skill_id`, and `state` (`core`, `on_demand`, `explicit_only`, or `archived`).
- `library_changes`: `skill_id`, `canonical_placement_id`, the complete `placement_ids` set returned by the Snapshot, and `requested_state` (`managed` or `hosted`).
- `source_updates`: the latest Skill/placement/source/revision/fingerprint facts plus upstream content and SHA-256 digest. If local content changed, include the user's explicit `choice`: `retain_local`, `adopt_upstream`, or `preserve_both`.

Keep source updates and Library changes in separate Plans. Cite Evidence returned by the relevant Finding page; a convenient but unrelated Evidence ID is not support for a governance change. Treat a rejected stale Snapshot, Evidence ID, fingerprint, incomplete placement set, or source revision as a reason to rescan or ask the user—not as permission to weaken the request.

State explicitly that inspection and planning changed no Agent files. When evidence cannot justify a change, recommend keeping the current state.

For `Skill links escape an approved root`, load one Finding page and show the
link targets. Treat each target as unread until the user confirms that its
source directory is intentional and trusted. After confirmation, rescan with
one repeatable `--source-root ABSOLUTE_PATH` per canonical source directory;
this approves reading without adding Agent exposure. Then prefer a reviewed
Hosted or Managed Library Plan so future Scans no longer depend on the
temporary source-root arguments. Keep unconfirmed targets unread.

## Apply or undo

Show the complete immutable Plan before requesting one explicit confirmation. After the user confirms, run `skillroster apply PLAN_ID --json`; do not substitute direct filesystem commands or bypass drift checks. Report verification, changed-path count, Receipt ID, and canonical deletion count.

For Undo, first explain the bounded Receipt impact and obtain one explicit confirmation, then run `skillroster undo RECEIPT_ID --json`.

Stop mutating when the result reports ambiguity, drift, unsupported scope, or recovery required. Make recovery the only mutating next action until cleared.

Use `skillroster status --json` for pending Plans, the last Receipt, retention, and recovery state. Use `skillroster lifecycle recovery --json` to inspect an unresolved Receipt. Export or purge local lifecycle data only when the user asks; purge changes controlled SQLite history, not Agent files.

## Find an on-demand Skill

Run `skillroster find "TASK" --json`. Explain the top matches and evidence, then read the selected `SKILL.md` directly from its returned path. If the lexical matches are clearly about another domain, retry once with concrete tool or operation terms already implied by the task (for example, `MySQL DDL` for a database migration); do not present an irrelevant top result as confidence. Finding a Skill does not activate or install it.

Never parse styled terminal output, invent a health score, infer token savings, or claim files changed without a successful Receipt.
