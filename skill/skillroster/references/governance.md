# Governance workflow

## Inspect

Run `skillroster scan --summary --json`, then the bounded
`skillroster report --json`. Use full `scan --json` only for deliberate root or
coverage diagnostics.
Use `finding_rollups` for group scale and the selected Findings for the first
decisions. Read `overview.state`: `inventory_empty` means no Skills were found
in the checked roots; `no_review_items` means only informational Findings remain.
In either case, finish the inspection with its scope and usage limits and offer
to keep the current setup. Show at most three relevant review items, never fill
empty slots with unknown usage or archive status. Informational Findings remain
available through `report --findings --limit 20 --json`, optionally narrowed by
category or severity, only when enumeration is needed. Follow `page.next_offset`
while that decision remains open. Reserve `report --full` for deliberate
exhaustive export; do not make extra calls just to satisfy a fixed presentation
template.

For each selected Finding, pass through these CLI facts without widening their
scope: `affected_agents` is the observed Agent set, `actionability` describes
what the CLI can support now, and `reversibility` describes the recovery path
of a possible applied change. They constrain the next safe action; they are not
a value score, and `reversibility` is not permission to Apply. Use the user's
goal and the available evidence to decide what deserves attention.

Usage evidence is five-stage and conservative. Read `session_coverage` before
describing it. `sampled_agents` support bounded observed events;
`complete_agents` alone support a complete observable-session denominator;
`missing_root_agents` and `inaccessible_agents` are unknown, not unused.
`Exposed` counts placements; later stages count events. Keep Agent, stage,
quality, event count, and observation window together.

Use stable Finding and Evidence IDs. For path or decision facts, call
`report --finding ID --limit 20 --json`. Treat
`planning.uncertainty.review_required` as review-required, not evidence-backed.

Keep the same Report and Snapshot for follow-up pages and decisions; do not
silently rescan between them. In `selection_evidence`, `target_agent` means the
named Agent used that exact Skill identity. `cross_agent` means only the Agents
listed in `evidence_agents` supplied evidence; describe it as “used elsewhere,”
never as target-Agent usage. Similar names and paths do not transfer evidence.
Every `stable_fallback` selection follows name and stable-ID order; it has no
usage evidence and says nothing about usefulness. Explain this even when only
one Core entry is a fallback. Review upcoming work and rare but important Skills
with the user; use the existing `protected_skill_ids` for their choices. Keeping
the current Roster is a valid outcome when the evidence is insufficient.

## Plan

First bind the proposal to the user's goal. A request to inspect ends with the
findings; it does not create a governance Plan. A request to deduplicate copies
stays a Library change and preserves default exposure. Reducing the default
Roster is a separate choice. When that is the goal and positive usage evidence
is absent, ask which capabilities must stay available for upcoming work before
using a Core budget or fallback ordering. Fifty is a supported budget ceiling,
not a recommendation. If the user prefers the current Roster, finish there.
An explicitly requested name-ordered draft may be previewed, with its fallback
basis stated plainly and no claim of better usefulness.

Make semantic choices in conversation, then submit them to
`skillroster plan --stdin --json`. Every declarative request includes
`schema_version: 1`:

- `finding_roster_changes`: preferred for a large default Roster. Supply the
  Finding ID, per-Agent `core_budget` from 1 through 50, and optional protected
  Skill IDs. Protected, declared, and bootstrap Skills come first; target-Agent
  usage outranks exact-identity cross-Agent usage, then stable fallback. Missing
  usage never implies Explicit-only or Archived. If planning reports
  `same_name_library_target_conflict`, inspect the linked divergent-content
  Finding. Do not submit the default request or choose one variant by rank.
  After confirmation, replay the exact validated
  `protect_library_target_claimants_as_core` request when it is available. A
  normalized-name collision is distinct from a same-name Finding; open full
  detail and preserve every listed claimant unless the user resolves the names.
  Multiple target groups can emerge during validation; a compact incomplete
  closure has no executable template. Treat names differing only by case as
  one portable Library target.
- `finding_library_changes`: preferred for exact duplicates. Supply the Finding
  ID, a listed canonical placement, and `managed` or `hosted`.

Choose the state by the user's physical-location intent. `managed` keeps the
selected canonical package at its current path and links matching placements
to it. `hosted` explicitly migrates it into SkillRoster's Library root. A request
to retain an existing source in place calls for `managed`; hosting requires
migration intent. Select the listed canonical candidate separately from this
state choice. Before Apply, compare `impact.items[].before.canonical_path` and
`after.canonical_path` with that intent; unchanged default exposure does not
mean the canonical package stayed put.

- Raw `roster_changes`, `library_changes`, and `source_updates` are advanced
  forms. Bind them to the latest Snapshot, complete scope, current Evidence,
  placement/source facts, revision, and fingerprint. For local source drift,
  require the user's explicit `retain_local`, `adopt_upstream`, or
  `preserve_both` choice.

For a raw source update, use this request shape with observed values:
`schema_version: 1`, `scan_id`, `evidence_ids`, and `source_updates: [...]`.
Each `source_updates` item has these exact fields:

- `skill_id`, `placement_id`: the exact target's current public Evidence
  bindings or an already verified exact Find selection; never infer by name.
- `source`: the installed target's declared source, from authorized metadata.
- `current_revision`: its declared `version`, otherwise `revision`.
- `current_fingerprint`: that placement's complete **package** fingerprint,
  from current placement-bound digest Evidence or verified Find's
  `verification.package_fingerprint`; never substitute a SKILL.md file hash
  or a routing-content identity digest. Require complete coverage and bindings.
- `upstream_revision`: the supplied candidate's different declared revision.
- `upstream_content`: exact UTF-8 entrypoint content supplied for this update.
- `upstream_digest`: SHA-256 of those exact entrypoint bytes, not its package.
- `choice`: `retain_local`, `adopt_upstream`, or `preserve_both`, matching the
  user's decision. The key is `choice`, not `decision`. First-observed/untrusted
  baselines require an explicit choice even without known local modification;
  modified trusted baselines also require it. Do not invent user intent.
- Optional `base_digest`: an already established immutable baseline's
  entrypoint SHA-256. Omit it if that baseline is unknown; a hash of today's
  file does not establish or replace a baseline and never promotes trust.

Take `scan_id` and relevant skill/placement-bound `evidence_ids` from the same
latest Snapshot's public Report/Finding facts. Open the exact Finding detail
when compact Evidence omits a required digest; keep each ID/digest together.
Only read installed/candidate metadata or hash bytes inside the authorized
scope; do not bypass a source/read blocker or read private SQLite to fill gaps.
The candidate's source/revision must match the submitted source/revision;
source updates consume supplied bytes, not an automatic Git fetch or endorsement.
Target only the authorized mutable regular placement. Do not choose canonical
content, send raw filesystem `operations`, or mix Roster/Library changes here.
Preview the validated Plan and its exact targets; Apply needs authorization.
Follow the existing mutation reference for Receipt verification and Undo.

Keep Roster, source, and Library changes in separate Plans. A stale Snapshot,
Evidence ID, fingerprint, incomplete scope, or revision requires rescan or user
input, never weaker validation.

If planning reports `trusted_canonical_sources_required`, treat that stable ID
only as a request for an exact local read permission, never as a claim that the
directory or its content is trustworthy. Use only the typed blocked Skills and
observed source targets. A truncated result
points to a SkillRoster-owned JSON detail file; validate its schema before
using the complete identities and argv. Do not inspect or trust targets
independently, synthesize broader parents, or submit a partial Plan.

If planning reports `mutation_scope_blocks_roster_change` or
`package_fingerprint_variants_block_roster_change`, read all `blocked_reasons`
and its typed `resolution_choices`. Package differences require preservation;
read permission does not resolve them. A compact
`protect_blocked_skills_as_core` choice with
`protected_skill_ids_complete: false` is not executable; open the same Finding
with `--full`. Only after the user confirms a full choice with
`plan_request_template_available: true` may you replay its exact Plan request.
That choice keeps the blocked Skills Core while mutable placements are
right-sized; it never makes provider or durable-read-only placements mutable.
When a selection reason is `shared_physical_forced_core`, explain that several
Agent placements share one physical Skill. It is a consistency constraint, not
usage evidence for every Agent. Trust the reconciled Core counts. If Plan
returns `roster_shared_core_budget_exceeded`, ask for a higher budget or fewer
forced Core Skills; do not submit incompatible direct states to bypass it.
If Report or Plan returns `roster_physical_identity_rescan_required`, run the
provided read-only Scan action. Never reconstruct the missing Snapshot fact
from current paths.

For an escaping Skill link, show the observed target, obtain confirmation, and
choose exactly one typed `permission_paths` continuation:

- `durable_permission`: run
  `skillroster source-root confirm --finding FINDING_ID --path ABSOLUTE_PATH --json`,
  then follow its plain Scan action. Do not add the confirmed path as a
  `--source-root` override.
- `temporary_one_scan`: skip `source-root confirm` and run one Scan with the
  repeatable exact `--source-root` option. This creates no durable permission.

Both paths grant factual read access only. They do not assess or endorse
content, raise Evidence quality, or authorize Plan/Apply. Treat the legacy
`confirm_trusted_source_roots` decision as an opaque compatibility alias; use
`decision_code` and `decision_semantics` for meaning. Use `source-root inspect
--json` to audit active, revoked, or drifted durable permissions and
`source-root revoke ID --json` to revoke one. Treat drift facts as bounded
accidental/persistent-drift evidence, not proof against a malicious same-user
ABA race. When `observed_link_targets_truncated` is true, follow Finding pages;
each page's exact confirmation actions cover its `page_observed_link_targets`.
Never infer a parent, sibling, descendant, alias, or wildcard.

## Present

For an exact-duplicate proposal, explain the selected Skill by name: which
copies match, which exact source the user chose to retain (or host), which paths
become links, which Agents keep the same exposure, and how the Receipt enables
Undo. Use the validated Plan's `impact.items` and totals for these facts. Show
the bounded paths first; open full Plan detail if a needed path is omitted.
Offer previewing that choice and keeping the current setup. Same-name variants
with different content still require their own explicit choice; candidate or
action array order never chooses a source or expresses business priority.

Present a compact, evidence-backed first view using the report's available
metrics, selected Findings, rollups, and typed safety facts. Include
`affected_agents`, `actionability`, and `reversibility` when they are present,
and keep each count's field meaning and unit. Do not derive deletion or
reduction counts from current Finding scale. State measurable before/after
impact only from a validated Plan, and actual impact only from its Receipt.
Include uncertainty, evidence quality, safety risks, and whether confirmation is
required. Choose a primary next action when the evidence supports one; expand
the view when the user's question or an unresolved decision requires it. For a
proposed change, include its measurable before/after impact,
`change_summary`, operation groups, affected facts, uncertainty,
reversibility, canonical deletion count, and Plan ID. Use `plan --show PLAN_ID
--json` only when an exact operation, path, selection, or complete ID list is
required. State explicitly that inspection and planning changed no Agent files.
