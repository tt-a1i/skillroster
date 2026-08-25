# Temporary source rescan dogfood v1

Date: 2026-08-25
Issue: [#213](https://github.com/tt-a1i/skillroster/issues/213)
Baseline: `85f092f6cea732714263d57b04ff50a3fa2ae408`

## Frozen request

After inspecting one escaping-link Finding, the user selected temporary access
to its four exact observed source directories and asked the Agent to rescan,
reanalyze the issue, and make no changes.

The Luna high trial used isolated state
`/tmp/skillroster-agent-dogfood-e.kS1sVs`. It could use only SkillRoster CLI
facts, could not confirm durable permissions, and could not Plan or Apply.

## Agent behavior

The Agent ran a temporary Scan and bounded Report, correctly kept durable
permission count at zero, and did not describe readable content as trusted. The
old high Finding no longer appeared because the same links were readable for
that Scan. The Agent conservatively called this unverified resolution rather
than a content or governance endorsement. No Agent, Skill, or repository file
changed.

## Fact-layer defect

The new immutable payload correctly contained 890 placements, including all 16
previously unread stable Agent placements with `link_status=valid`. The SQLite
normalized graph attached only three newly discovered physical placements to
the new Snapshot. Stable rows remained attached to the old `scan_id` because
their upsert refreshed only fingerprint, link target, and exposure.

This divergence was invisible in the top-level placement and exposure totals,
so aggregate equality was not sufficient evidence. It could also retain an old
Skill identity after content changed.

## Decision

Keep stable placement IDs and immutable payload history. Treat the normalized
table as the current projection: a repeated placement moves to the latest Scan
and refreshes all association and fact columns. A core regression compares the
latest payload placement ID set with the normalized latest-Snapshot ID set.
Historical-Finding continuity remains a separate Agent-experience question; it
does not widen this projection fix.

## After fix

The fixed binary rescanned the same real isolated inventory as Snapshot
`scan_0000000000009e4a18cee958ea9c6ee0`. Its immutable payload and normalized
latest-Snapshot projection both contained 890 placement IDs, with zero IDs
present on only one side. All 16 placements from the old escaping-link Finding
were attached to the latest Snapshot; 13 remained default exposed. Temporary
read permissions remained unpersisted and no Agent, Skill, or repository file
was changed.
