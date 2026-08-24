# Real Agent Skill governance dogfood — Issue #163

Date: 2026-08-24 (Asia/Shanghai)
Scope: one read-only dogfood against the current local multi-Agent Skill estate.
This is evidence for the Agent-first facts/presentation boundary, not a routing
benchmark or a governance mutation run.

## First view for Codex

```text
SkillRoster · local read-only diagnosis
Snapshot  scan_…b8782be8                         Recovery  clear

Estate       251 independent Skills · 887 placements · 521 default exposures
Coverage     5/8 Skill roots present · usage observed on 3 Agents
Usage        Loaded 39 observed · denominator unreliable for all 8 Agents
Mutation     CLI files_changed=false · no Apply · no Setup · no Undo

Highest-signal findings
  HIGH   16 placements have links escaping an approved root
  MED    4 large default Rosters: Codex 135 · Claude 90 · Pi 125 · Hermes 129
  MED    115 duplicate Findings / 670 placements; example group: 9 / 3 sources

Governance preview (review only)
  Core budget 50; proposed On-demand: Codex 78 · Claude 34 · Pi 68 · Hermes 72
  Fallback-heavy selections and 27 blocked changes require explicit review
  Explicit-only / Archived: unknown; missing usage is not negative evidence

Next action: inspect Findings and resolve variant/source blockers; do not Apply.
```

The displayed IDs and evidence are intentionally shortened only for the first
view. The machine artifact retains stable Finding/Evidence/Skill identities.

## Run and boundary

The run used source commit `209e24a`, the release binary `skillroster 1.8.24`,
one isolated state directory, and one external raw-evidence directory. The CLI
ran one Scan, Status, bounded and filtered Reports, three semantic Find
queries, one selected Finding drill-down, a read-only roster Plan surface, and
a final Status check. Fourteen command invocations were recorded; no retry was
needed. The initial Scan took approximately 11.5 seconds by the tool wall-clock
measurement; subsequent read-only commands completed in under one second each.

The mutation-observation fingerprint covers the 21 Skill roots actually
included by the Scan (including the shared local Skill root and
provider-managed plugin Skill roots), their file metadata and symlink
identities, plus content digests for explicitly listed configuration/roster
files. Dynamic session, log, cache, auth, repository, and isolated state paths
are excluded. Before and after manifests both contain 3,973 lines and the same
SHA-256. The final CLI Status also reports `files_changed=false`,
`pending_plan_count=0`, and recovery `clear`.

This evidence does **not** prove byte-for-byte immutability of every ordinary
Skill file: the pre-run manifest recorded size and modification time for those
files, not their content digests, and linked targets outside the listed roots
were not content-frozen. It proves no observed metadata, symlink-identity, or
listed-config change and corroborates the CLI's read-only report. A future
strict zero-change gate must hash regular-file content before and after.

## Three user intents

### Inventory and observed usage

The Agent received a bounded report and usage Finding. It can state that 521
placements are exposed and 39 Loaded events were observed, with observed use
across 3 Agents. It must also state that all 8 Agents lack a reliable complete
session denominator: 5 roots are limited and 3 are missing. Therefore the
result cannot classify a Skill as “never used”. The usage Finding and its
Evidence ID remain the traceable source for these statements.

### Duplicate, exposure, source, path, and layout diagnosis

The report surfaced 16 escaping-link placements, a 479-placement large-Roster
Finding across four Agents, and 115 exact-duplicate Findings affecting 670
placements. The first view also shows one redacted representative duplicate
group with 9 placements across 3 physical sources. Same-name divergent
variants return stable variant Findings rather than silently choosing a
package. These are separate structural facts: duplicate content does not imply
reduced exposure, and an escaping source is not an implicit permission to read
or mutate it.

### Core / On-demand / Explicit-only / Archived summary

The selected large-Roster Finding produced a review-only Core budget-50
preview. It named positive signals and separately counted cross-Agent and
stable-fallback candidates. Fallbacks dominate the preview for every affected
Agent (38, 43, 38, 38 respectively), so the Agent must label the proposal
uncertain. The CLI explicitly reports that missing usage does not imply an
Explicit-only or Archived decision.

Submitting that Finding to the read-only `plan --stdin` surface failed closed
with `roster_package_fingerprint_variants`: multiple complete package
fingerprints require explicit preservation before a canonical roster mutation
can be planned. No partial Plan was stored and no real Agent/Skill filesystem
mutation was attempted; the CLI wrote only its isolated state. The blocker is
useful product evidence, not a failed safety test.

## Acceptance decision

**Partial for Issue #163's full evidence gate.** The functional dogfood passed:
the Agent produced a useful first-view diagnosis from deterministic facts,
preserved the distinction between observed/inferred/unknown, exposed fallback
and variant uncertainty, and stopped safely at a typed governance blocker. The
strict zero-change acceptance requirement did not pass because ordinary Skill
file bytes lacked a pre-run content digest. The run does not prove semantic
intent quality, Core-vs-On-demand superiority, task-success improvement, labor
savings, a complete usage denominator, or byte-for-byte estate immutability.

The Agent-authored, inferred follow-up is narrow: strengthen the zero-change
ledger with regular-file content digests, and make incomplete usage coverage,
fallback-dominated selections, and variant/source blockers prominent in the
first view and next action. This is a model recommendation, not a CLI fact or
governance authorization. No embedding, reranker, intent model, Skill graph,
MCP, daemon, cloud service, TUI, or second routing Skill is justified.

The redacted machine record is
[`real-agent-dogfood-issue-163.json`](artifacts/real-agent-dogfood-issue-163.json).
Raw command output and local manifests remain outside the repository.
