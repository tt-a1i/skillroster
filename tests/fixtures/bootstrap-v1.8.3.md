---
name: skillroster
description: Inspect, search, organize, apply, or undo governance for locally installed Agent Skills with the SkillRoster CLI. Use when the user asks which Skills are installed or used, wants duplicates or broken links analyzed, needs a smaller default Skill roster, wants an on-demand Skill found, or asks to apply or undo an approved Skill organization plan.
metadata:
  bootstrap-version: "1.8.3"
---

# SkillRoster

Use the local `skillroster` binary as the deterministic source of facts. Invoke every command with explicit `--json`; validate `schema_version` and `ok` before reading `result`. Treat typed `suggested_actions` as options, never authorization.

## Inspect and propose

1. Run `skillroster scan --json`, then `skillroster report --summary --json`. Use the compact result for the first user-facing diagnosis.
2. Distinguish observed, inferred, and unknown usage. Read `session_coverage` before presenting usage: `sampled_agents` have bounded local evidence, `complete_agents` alone support a complete observable-session denominator, `missing_root_agents` have no discovered session directory, and `inaccessible_agents` have a configured directory that could not be read. Limited samples may support observed event counts and stages, never a usage percentage or an “unused” claim. When the three-item summary is insufficient, use `report --findings --limit 20 --json`; narrow it with one `--category` or `--severity` from the summary totals, and follow `page.next_offset` only while that category still affects the decision. This paged list is the enumeration path; keep the exhaustive report out of the Agent context.
3. Use stable Finding and Evidence IDs for follow-up questions. The summary and paged list expose one `primary_evidence_id`; use `report --finding ID --limit 20 --json` when paths or Evidence affect the decision. Its compact `items` combine the Evidence ID, subject, path, quality, and decision facts without repeating complete internal collections. Use `--full` only when an exact complete ID or record is needed. Exact-duplicate and large-Roster details include bounded `planning` choices independently of pagination. Continue with `--offset NEXT_OFFSET --limit 20` only when another decision still needs more evidence, and keep the same Snapshot rather than silently rescanning.
4. Make semantic governance choices in the conversation, then submit them to `skillroster plan --stdin --json`. For exact duplicates or a large default Roster, send `schema_version` plus the corresponding Finding request below; SkillRoster derives the current Snapshot, Evidence, Skills, placements, and complete changes. Other request families include the latest `scan_id` and relevant `evidence_ids`.
5. Present the validated summary Plan in one viewport: diagnosis, four core metrics, three main Findings, `change_summary`, `operation_groups`, bounded `affected` facts, `impact` before/after facts, the semantic `diff_summary`, uncertainty, canonical deletion count, reversibility, and Plan ID. The diff contains at most three Roster, Library, and filesystem items, or bounded line facts for a source update. The full immutable representation stays in local state; use `skillroster plan --show PLAN_ID --json` only when an exact operation, path, or complete ID list is needed to answer the user. Do not load it by default.

Use only these Plan request families:

- `finding_roster_changes` (preferred for `Large default Rosters need review`): `finding_id`, a per-Agent `core_budget` from 1 through 50, and optional `protected_skill_ids`. Review `planning.agents`, especially positive-signal and fallback counts, before choosing the budget. SkillRoster preserves requested, declared, and bootstrap Core Skills, ranks positive usage evidence, and uses stable ordering only as a fallback. Remaining affected Skills become On-demand; this request never implies Explicit-only or Archived. If `planning.supported` is false, follow its typed `decision`: confirm reported source roots or resolve dependent source links, then rescan. Do not submit a partial Plan.
- `finding_library_changes` (preferred for exact duplicates): `finding_id`, a `canonical_placement_id` chosen from `planning.canonical_candidates`, and `requested_state` (`managed` or `hosted`). Do not copy paged placement or Evidence IDs into this request.
- `roster_changes`: the advanced raw form with `agent`, `skill_id`, and `state` (`core`, `on_demand`, `explicit_only`, or `archived`). Use it for deliberate exceptions or when no supported Finding can bind the complete scope.
- `library_changes`: the advanced raw form with `skill_id`, `canonical_placement_id`, the complete `placement_ids` set, and `requested_state`. Use it only when there is no exact-duplicate Finding to bind.
- `source_updates`: the latest Skill/placement/source/revision/fingerprint facts plus upstream content and SHA-256 digest. If local content changed, include the user's explicit `choice`: `retain_local`, `adopt_upstream`, or `preserve_both`.

Keep Roster, source-update, and Library changes in separate Plans. Semantic Finding requests derive their Evidence internally. For raw requests, cite Evidence returned by the relevant Finding page; a convenient but unrelated Evidence ID is not support for a governance change. Treat a rejected stale Snapshot, Evidence ID, fingerprint, incomplete scope, or source revision as a reason to rescan or ask the user—not as permission to weaken the request.

State explicitly that inspection and planning changed no Agent files. When evidence cannot justify a change, recommend keeping the current state.

For `Skill links escape an approved root`, load one compact Finding page and
show `resolution.observed_link_targets`. Treat each target as unread until the
user confirms that its canonical source directory is intentional and trusted.
Do not follow a generic Plan action for this Finding. After confirmation,
rescan with one repeatable `--source-root ABSOLUTE_PATH` per canonical source
directory; this approves reading without adding Agent exposure. Then prefer a
reviewed Hosted or Managed Library Plan so future Scans no longer depend on
the temporary source-root arguments. Keep unconfirmed targets unread.

## Apply or undo

Run `skillroster setup --json` when installing the Bootstrap Skill or after upgrading the CLI. An exact official older copy produces a normal upgrade Plan. If `state` is `modified_choice_required`, show the affected Agent targets and ask whether to `retain-local` or `adopt-current`; do not choose for the user. Adopting still only prepares a recoverable Plan and requires the usual Apply confirmation. Treat `unsupported_targets` as blocked rather than replacing links or non-files.

Show the complete immutable Plan before requesting one explicit confirmation. After the user confirms, run `skillroster apply PLAN_ID --json`; do not substitute direct filesystem commands or bypass drift checks. Report verification, changed-path count, Receipt ID, and canonical deletion count.

“Complete” means the summary accounts for every affected Agent, Skill, placement, operation group, exclusion, risk, and before/after delta by count; it does not require copying every internal operation into the conversation. If the user asks for an exact path or operation, load the stored detail with `plan --show` before confirmation.

For Undo, first explain the bounded Receipt impact and obtain one explicit confirmation, then run `skillroster undo RECEIPT_ID --json`.

Stop mutating when the result reports ambiguity, drift, unsupported scope, or recovery required. Make recovery the only mutating next action until cleared.

Use `skillroster status --json` for pending Plans, the last Receipt, retention, and recovery state. Use `skillroster lifecycle recovery --json` to inspect an unresolved Receipt. Export or purge local lifecycle data only when the user asks; purge changes controlled SQLite history, not Agent files.

## Find an on-demand Skill

Run `skillroster find "TASK" --json`, keeping the user's task verbatim. For a non-English or mixed-language task, include one concise English capability paraphrase through `--hint "TEXT"` on the first call. Build the hint entirely from the desired target surface, object, operation, and state—for example, `control existing logged-in Chrome tabs` or `analyze standalone spreadsheet file workbook data`. Use terms implied by the task rather than a guessed Skill name. Retry once with a refined target description only when the result is empty or clearly about another domain. Explain the top matches and evidence. A `variant_count` above one is unresolved same-name content ambiguity: keep each returned variant's path and provider facts together, respect `variants_truncated`, show the warning, and inspect the corresponding layout Finding before choosing content. Otherwise read the selected `SKILL.md` directly from its returned path. Finding a Skill does not activate or install it.

Treat `providers` with `governable: false` as enabled provider-managed Skills: they are valid read-only search results, but must not be moved, updated, consolidated, or added to a governance Plan.

Never parse styled terminal output, invent a health score, infer token savings, or claim files changed without a successful Receipt.
