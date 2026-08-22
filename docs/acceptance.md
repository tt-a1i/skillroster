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
- taking non-Core Skills off default exposure does not remove them from `find`, so the synthetic task-success count does not regress;
- small (5 Skills), large (120 Skills), and cross-Agent (12 Skills) duplicate scenarios preserve counts, traceable Finding evidence, and the four report metrics;
- plain 60-, 80-, and 120-column reports retain their core fields, no-change statement, and no ANSI bytes.

The routing baseline is defined as: a task succeeds when its expected Skill appears in the first three `find` results. The pre- and post-governance runs use the same Scan contents; the post-governance run reduces default exposure while keeping On-demand Skills searchable. This tests routing continuity, not whether an external model completed the downstream task.

Current local result (2026-08-22): **40/40 Top-3 hits before governance and 40/40 after governance**. Re-run the test above for release evidence; this recorded result is not a substitute for CI.

## Fixed value comparison

[`tests/fixtures/value-comparison.json`](../tests/fixtures/value-comparison.json) compares the same synthetic 120-Skill inventory:

| Approach | Default exposure | Duplicate placements left | Task successes | Receipt-backed |
|---|---:|---:|---:|---|
| Unmanaged | 120 | 80 | 40 | No |
| Careful manual | 54 | 10 | 40 | No |
| SkillRoster proposal | 36 | 0 | 40 | Yes |

This proves the calculation and the 50% exposure-reduction gate on controlled data. It does **not** claim measured labor savings, token savings, production performance, or superiority over every external manager.

## Evidence boundaries

Automated fixtures validate adapters and presentation facts, but cannot certify a real Agent conversation, terminal rendering, or OS integration. Before declaring a release platform-supported, record these independently:

1. a real-environment run on macOS, Linux, and Windows using the release binary;
2. a human review of the first viewport for small, large, and cross-Agent scenarios;
3. an Agent conversation confirming one visible Plan, one confirmation, Apply verification, Receipt, and Undo;
4. accessibility checks in representative terminals, including narrow CJK output;
5. release-artifact checksums and installation smoke tests.

CI results are evidence for the automated layer only. A skipped or simulated platform run remains unexecuted acceptance.
