# SkillRoster 1.0 Acceptance

This document separates deterministic, synthetic acceptance from release checks that require a person or a real operating system. Fixtures contain no user Skills, sessions, prompts, credentials, or governance state.

## Automated acceptance

Run the core suite with:

```bash
cargo test --test acceptance
```

The suite proves:

- all eight direct adapters discover their independent filesystem fixture;
- each adapter conservatively normalizes Exposed, Matched, Loaded, Applied, and Outcome evidence, while prose-only mentions create no observed stage;
- the maintained 47-task routing set, including Agent-hinted Chinese tasks, reaches at least 95% Top-3 recall;
- a public `plan`/`apply` moves non-Core Skills to On-demand, `find` still returns readable paths, and Receipt-bounded `undo` restores the original Agent tree;
- small (5 Skills), large (120 Skills), and cross-Agent (12 Skills) duplicate scenarios preserve counts, traceable Finding evidence, and the four report metrics;
- plain 60-, 80-, and 120-column reports retain their core fields, no-change statement, and no ANSI bytes.

Top-3 routing and task success are separate checks:

- **routing hit:** the expected Skill appears in the first three public `find` results;
- **task success:** after routing, the evaluator opens a returned `SKILL.md` path and verifies its deterministic `CAPABILITY:` contract.

The governed run uses the public `scan`, `report`, `plan`, `apply`, `find`, and `undo` commands. It marks seven of ten Skills On-demand, then repeats both checks against paths returned after the real Apply. This validates deterministic fixture capability, not whether an external model completed a natural-language task.

Current local result (2026-08-22): **47/47 Top-3 hits before governance and 47/47 after governance**. Re-run the test above for release evidence; this recorded result is not a substitute for CI.

## Executed three-arm value comparison

The acceptance test creates three isolated temporary homes from the same deterministic 120-Skill inventory. Results are calculated from actual Scans and Reports; no result fixture is loaded.

- **Unmanaged:** leaves 120 Codex placements plus 80 Claude Code copies untouched.
- **Careful manual:** moves canonical packages into `.agents_skills`, uses a declared scope budget to resolve 70 of 80 cross-Agent copies, creates 54 hard-linked Core placements, and records the 10 unresolved copies in `manual-roster.json`.
- **SkillRoster:** runs the public CLI, submits 200 Roster changes, applies the validated Plan, measures the resulting filesystem, then undoes the Receipt and compares the restored Agent tree byte-for-byte.

Current local result (2026-08-22):

| Approach | Default exposure | Duplicate placements left | Receipt-backed |
|---|---:|---:|---|
| Unmanaged | 200 | 80 | No |
| Careful manual | 64 | 10 | No |
| SkillRoster Apply | 36 | 0 | Yes; Undo restored 200/80 |

“Duplicate placements” counts additional same-content non-symlink placements across Agent roots; shared-library canonicals are not double-counted. The manual arm's 10 is verified by scanning the filesystem after its bounded procedure, not inserted as a result fixture. The run proves the 50% exposure-reduction gate, duplicate reduction, readable On-demand routing, and restoration on controlled data. It does **not** claim measured labor savings, token savings, production performance, or superiority over every external manager.

The [relevant-manager execution run](acceptance/manager-comparison-run.md)
separately installs the same synthetic Skills with Vercel `skills` 1.5.23,
then proves SkillRoster can scan, govern, route, and exactly restore that
manager-owned symlink layout.

## Executed local environment checks

On the reference macOS machine, a release build scanned 180 independent Skills
and 676 placements across all eight adapters in 3.37 seconds. It changed no
Agent files, left schema v8 recovery clear, and remained below the five-minute
gate. A real PTY run also verified:

- Apply and Undo show the Plan or Receipt impact before reading input;
- cancellation exits successfully, creates no Receipt, and changes no paths;
- progress starts only after confirmation;
- SIGINT exits 130 with cursor restoration when styled and a truthful static
  summary in `TERM=dumb`;
- confirmed Apply and Undo both verify and leave recovery clear.

## Release-candidate platform evidence

The 2026-08-22 release gates use actual hosted operating systems and the packaged
release binaries, always against synthetic temporary homes:

- [four-platform CI](https://github.com/tt-a1i/skillroster/actions/runs/32548259913)
  passed Rust 1.85 formatting, Clippy, and 79 core/high-risk tests on Linux x86_64,
  Windows x86_64, macOS arm64, and macOS x86_64;
- [release candidate 8](https://github.com/tt-a1i/skillroster/actions/runs/32548259787)
  built all four archives and ran `--version`, `--help`, Scan, Setup preview,
  Apply verification, Receipt-bounded Undo, and recovery-clear Status on every
  corresponding operating system;
- all four candidate checksums were downloaded and independently verified; each
  archive contained only the binary and README, and the macOS arm64 archive
  passed the same governance smoke after extraction;
- `cargo install --locked --path .` into an isolated prefix passed version and
  governance smoke checks.

WSL is tested as Linux in the release workflow by importing a checksum-pinned
Canonical Ubuntu 24.04 image into WSL1 on the Windows runner, then executing the
packaged Linux binary through the same governance loop. The final successful
candidate and official tag run are the release record; a failed or skipped WSL
job does not count as support.

## Human and Agent presentation review

A human review of plain first viewports found the intended hierarchy intact:

- the eight-Agent fixture at 60 and 120 columns showed the four key metrics,
  top Findings, category totals, read-only boundary, and next action without
  wrapping or ANSI noise;
- the real 180-Skill/676-placement environment at 80 columns surfaced 548
  default exposures and the escaping-link and exact-duplicate Findings in the
  first viewport, with no Agent-file changes;
- automated 60/80/120-column, `NO_COLOR`, `TERM=dumb`, non-TTY, CJK-width, and
  styled/plain semantic-equivalence checks cover the remaining repeatable cases;
- the PTY acceptance above covers visible impact, confirmation, cancellation,
  SIGINT, progress timing, verification, Receipt, Undo, and recovery states.

In this Agent-led release run, the Agent consumed only JSON, inspected the Setup
Plan preview and mutation boundary, used the user's explicit execution
authorization for the synthetic scope, then verified Apply, Receipt, Undo, and
recovery-clear Status. The bootstrap Skill separately requires an explicit user
confirmation before any real Apply or Undo and never asks per-file questions.

## Agent-led Finding drilldown evidence

Before the 1.3 release, the Agent reused one immutable read-only Snapshot from
the reference macOS home: 180 independent Skills, 676 placements, 548 default
exposures, and 189 Findings. It changed no Agent files. On that same Snapshot:

- the unsafe-link Finding response fell from 24,817 to 10,410 bytes (58.1%) in
  default compact mode while retaining all 16 paged Evidence items;
- an exact-duplicate Finding fell from 11,814 to 6,807 bytes (42.4%) while
  retaining seven traceable Evidence items and its planning action;
- `--full` preserved the complete IDs, placements, and Evidence records on
  explicit request;
- unsafe links no longer suggest a generic Plan. They return a typed trust
  decision, observed targets, and a repeatable `--source-root` rescan template;
- plain 60-, 80-, and 120-column Finding views preserve the issue, counts,
  Evidence paths, and next decision without exceeding the requested width.

## Agent-led Finding enumeration evidence

The 1.4 discovery run started with the public 1.3.0 macOS arm64 binary and a
fresh isolated state directory. Its read-only Scan found the same 180 Skills,
676 placements, 548 default exposures, and 189 Findings in 9.41 seconds, with
no Agent-file changes. The Agent then measured a 3,179-byte three-Finding
summary against a 645,511-byte exhaustive report and confirmed there was no
bounded enumeration path.

On the same immutable Snapshot, the new paged interface returned 20 complete
Finding summaries in 15,590 bytes. A `usage` category filter selected exactly
two Findings; combined `overlap` and `medium` filters selected 125. The summary
now suggests `list_findings`, a partial page preserves its filters in
`list_more_findings`, and the exhaustive diagnostic export suggests no generic
Plan. Selected IDs still lead to compact Evidence drilldown. Automated tests
cover pagination, filtering, action argv, invalid flag combinations, and plain
60-, 80-, and 120-column rendering.

## Evidence boundary

These checks establish deterministic routing and safe local governance; they do
not claim model quality, token or labor savings, production performance, or
access to arbitrary future Agent log formats. Final support additionally
requires the official release tag workflow, published artifact checksums, and
the installation checks recorded in the Release notes.
