# Finding continuity dogfood

## Question

When a newer Scan and Report exist, can an Agent take an older Finding and
identify which of its original placements are still in the current inventory,
what current Findings affect them, and which paths are only historical?

The join is intentionally factual: stable `placement_id` is the only join
key. The command does not rescan, read Skill contents, infer semantic
resolution, create a Plan, or mutate Agent/Skill files.

## Before

The real continuity investigation required five CLI calls: the historical
Finding, the current Report summary, the exhaustive current Report, and two
pages of the executable-script Finding. The full Report was 776,585 bytes.
The Agent then had to manually join stable placement IDs and paths. This was
usable for a careful one-off audit, but made omission, truncation, and
“missing means resolved” mistakes easy.

## After

The existing read-only command remains the entry point:

```text
skillroster report --finding FINDING_ID --limit 20 --json
```

When the latest completed Snapshot has a current persisted Report, the result
contains bounded `current_continuity` facts. It identifies both historical
and current Report/Snapshot IDs, complete historical/current/matching counts,
bounded current Finding references with matched placement IDs, and current
placement facts for the requested historical page. Each current placement
also includes its bounded current Finding IDs so an Agent can group paths
without another export. Missing historical placements have a separate count
and bounded ID preview. Every preview has an explicit truncation flag.

If the latest Report is absent, stale, or malformed, continuity is returned as
`status: unavailable`; the historical Finding detail still succeeds. A zero
intersection is explicitly not a resolution claim.

## Verification

- Real local-state dogfood used historical Finding
  `finding_000000000000a1a218cee457f81f6f79` against the latest persisted
  Report. One 33,502-byte compact response returned all 16 current paths and
  three overlaps: 12 for `Large default Rosters need review`, 16 for
  `Management state needs review`, and 16 for
  `Skill packages contain executable scripts`. The command reported
  `files_changed: false`.
- Core test covers stable-ID intersection, missing placement accounting,
  a current-only placement, current Finding membership, and page placement
  facts.
- CLI acceptance creates a 51-placement historical Finding, adds one current
  placement, and verifies the 20-item requested page, current paths, Finding
  IDs, complete overlap count, and zero missing placements.
- Continuity performs no Scan, new Skill-content read, Plan, or Apply, and does
  not change Agent or Skill files.
