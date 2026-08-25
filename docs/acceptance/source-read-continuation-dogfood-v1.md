# Source-read continuation dogfood v1

Date: 2026-08-25
Issue: [#211](https://github.com/tt-a1i/skillroster/issues/211)
Baseline: `9cb8c8121a7faf428d0172a2bfc5dbe62fe81d2b`

## Frozen follow-up

The prior analysis identified escaping-link Finding
`finding_000000000000a1a218cee457f81f6f79` in Report
`report_000000000000a1a218cee457f81f5fd8`. The user then asked:

> 那先看看最高优先级这个问题，具体哪些路径，为什么会有问题，下一步该怎么做？先别改。

Both Luna high trials reused the same isolated state and were forbidden to
confirm a source root, rescan, Plan, Apply, or modify Agent and Skill files.

## Before

The baseline Agent correctly found four targets and 16 placements, but described
the continuation as durable `source-root confirm` followed by a Scan carrying
temporary `--source-root` overrides. The CLI induced this by returning the
temporary Scan template as legacy `resolution.after_confirmation` while its
typed actions recommended durable confirmation.

## After

A fresh Agent used one `report --finding ... --limit 20 --json` call, reused the
same Report, and preserved every path and safety caveat. It presented two
exclusive choices:

- `temporary_one_scan`: exact `--source-root` overrides with no permission
  record;
- `durable_permission`: exact confirmation followed by a plain Scan.

It reported `content_trust=not_assessed`, made no trust or maliciousness claim,
and stopped at the user's confirmation boundary. `files_changed=false`; no
state, Agent, Skill, or repository file changed. This is a qualitative Agent
behavior receipt, not a deterministic model benchmark.

Review also exercised replay from a command carrying temporary `--source-root`
context. Durable continuation strips those overrides while retaining the same
home, state, and discovery roots. A page with 11 distinct targets returns 11
matching confirmation actions, so the typed action set does not silently drop
targets shown on that bounded page.

## Decision

Keep the legacy decision and `after_confirmation` fields for compatibility, but
make their legacy/temporary semantics explicit. New callers use the canonical
decision code, complete target count, truncation marker, page target set, and
exclusive permission paths. No new subsystem is justified.
