# Governance workflow

## Inspect

Run `skillroster scan --json`, then the bounded `skillroster report --json`.
Use `finding_rollups` for complete group scale and the selected findings for the
first decisions. Use `report --findings --limit 20 --json`, optionally narrowed
by one category or severity, only when enumeration is needed. Follow
`page.next_offset` while that decision remains open. Reserve `report --full`
for deliberate exhaustive export.

The initial diagnosis is complete when the bounded Report supplies the four
core metrics, selected Findings, complete rollups, coverage, and a primary next
action. Stop there. If one exact fact required to explain that primary action is
absent, make one `report --finding` call for it. Help, status, Finding
enumeration, other Finding drills, and full detail belong to a later user
question; they are not initial-diagnosis checks.

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

## Plan

Make semantic choices in conversation, then submit them to
`skillroster plan --stdin --json`. Every declarative request includes
`schema_version: 1`:

- `finding_roster_changes`: preferred for a large default Roster. Supply the
  Finding ID, per-Agent `core_budget` from 1 through 50, and optional protected
  Skill IDs. Protected, declared, and bootstrap Skills come first; target-Agent
  usage outranks exact-identity cross-Agent usage, then stable fallback. Missing
  usage never implies Explicit-only or Archived.
- `finding_library_changes`: preferred for exact duplicates. Supply the Finding
  ID, a listed canonical placement, and `managed` or `hosted`.
- Raw `roster_changes`, `library_changes`, and `source_updates` are advanced
  forms. Bind them to the latest Snapshot, complete scope, current Evidence,
  placement/source facts, revision, and fingerprint. For local source drift,
  require the user's explicit `retain_local`, `adopt_upstream`, or
  `preserve_both` choice.

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

For an escaping Skill link, show the observed target and obtain confirmation
before running
`skillroster source-root confirm --finding FINDING_ID --path ABSOLUTE_PATH --json`.
This records factual read access for that exact canonical directory and stable
filesystem identity only. It does not endorse content, raise Evidence quality,
or authorize Plan/Apply. Then rescan. Use `source-root inspect --json` to audit
active, revoked, or drifted permissions and `source-root revoke ID --json` to
revoke one. The temporary repeatable `--source-root` option remains a one-scan
alternative. Treat its drift facts as bounded accidental/persistent-drift
evidence, not proof against a malicious same-user ABA race. Never infer a
parent, sibling, descendant, alias, or wildcard.

## Present

Show one bounded viewport with a one-sentence diagnosis and exactly these four
core metrics: independent Skill count, placement count, default exposure, and
observed-use count. Show the three highest-priority Findings plus compact
rollups for the complete scale of every Finding group. Prioritize
recommendations and keep each typed count's field meaning and unit. A Finding
describes current affected scale: canonical candidates, physical sources,
logical placements, default exposure, and relinks are distinct facts. Do not
derive deletion or reduction counts from them. State measurable before/after
impact only from a validated Plan, and actual impact only from its Receipt.
Include uncertainties, evidence quality, safety risks, and whether confirmation
is required. Name one primary next action. For a proposed change, include its
measurable before/after impact, `change_summary`, operation groups, affected
facts, uncertainty, reversibility, canonical deletion count, and Plan ID. Use
`plan --show PLAN_ID --json` only when an exact operation, path, selection, or
complete ID list is required. State explicitly that inspection and planning
changed no Agent files.
