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
- the maintained 40-task routing set reaches at least 95% Top-3 recall;
- a public `plan`/`apply` moves non-Core Skills to On-demand, `find` still returns readable paths, and Receipt-bounded `undo` restores the original Agent tree;
- small (5 Skills), large (120 Skills), and cross-Agent (12 Skills) duplicate scenarios preserve counts, traceable Finding evidence, and the four report metrics;
- plain 60-, 80-, and 120-column reports retain their core fields, no-change statement, and no ANSI bytes.

Top-3 routing and task success are separate checks:

- **routing hit:** the expected Skill appears in the first three public `find` results;
- **task success:** after routing, the evaluator opens a returned `SKILL.md` path and verifies its deterministic `CAPABILITY:` contract.

The governed run uses the public `scan`, `report`, `plan`, `apply`, `find`, and `undo` commands. It marks seven of ten Skills On-demand, then repeats both checks against paths returned after the real Apply. This validates deterministic fixture capability, not whether an external model completed a natural-language task.

Current local result (2026-08-22): **40/40 Top-3 hits before governance and 40/40 after governance**. Re-run the test above for release evidence; this recorded result is not a substitute for CI.

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

## Evidence boundaries

Automated fixtures validate adapters and presentation facts, but cannot certify a real Agent conversation, terminal rendering, or OS integration. Before declaring a release platform-supported, record these independently:

1. a real-environment run on macOS, Linux, and Windows using the release binary;
2. a human review of the first viewport for small, large, and cross-Agent scenarios;
3. an Agent conversation confirming one visible Plan, one confirmation, Apply verification, Receipt, and Undo;
4. accessibility checks in representative terminals, including narrow CJK output;
5. release-artifact checksums and installation smoke tests.

CI results are evidence for the automated layer only. A skipped or simulated platform run remains unexecuted acceptance.
