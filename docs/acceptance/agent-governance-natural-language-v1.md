# Natural-language Agent governance dogfood v1

Date: 2026-08-25
Issue: [#209](https://github.com/tt-a1i/skillroster/issues/209)

## Frozen task

> 帮我分析这台电脑当前安装的 Agent Skills：主要有什么问题，哪些确实有证据，哪些不能下结论，并给出治理优先级。先只读，不要改任何 Skill 或 Agent 配置。结果要一眼能看懂，不要把一堆命令和原始 JSON 扔给我。

Each Luna high trial scanned `/Users/tushaokun` with a separate temporary state
directory. Trials could read the Bootstrap and governance reference only when
assigned to the Bootstrap arm. No trial could Plan, Apply, confirm a source
root, or mutate Agent and Skill files.

## Evidence

| Trial | Bootstrap | SkillRoster calls | Result |
| --- | --- | ---: | --- |
| A | no | 9 | Safe facts, but paged Findings, full detail, and three unrelated Find calls |
| B | old | 7 | Clear summary, but inferred `9 - 3 = 6` removable placements without a Plan |
| C | first correction | 7 | Unsupported reduction closed; unnecessary help, drills, and enumeration remained |
| D | final correction | 2 | Scan and bounded Report only; no unsupported impact claim |

This table is a qualitative behavior ledger, not a deterministic model test.
Trials A–C preserve the observed failure progression; only Trial D is the
accepted run and is bound to the Snapshot and Report below. The baseline source
revision was `fd01ea7418dcc83d07d593735239c85821f55aa8`.

All four trials agreed on 251 independent Skills, 887 placements, 521 default
exposure placements, and three Agents with positive usage observations. Trial D
also preserved the three selected Findings, every Finding rollup, session and
root coverage limitations, and the read-only boundary. It correctly described
the exact-duplicate Finding as current affected scale and deferred canonical
ownership, before/after impact, and mutation to a later decision and validated
Plan.

Trial D wrote only its isolated database and captured JSON under
`/tmp/skillroster-agent-dogfood-d`. It changed no repository, Agent, or Skill
file. Snapshot `scan_000000000000982418cee44ffe47ac88` and Report
`report_000000000000a1a218cee457f81f5fd8` identify the accepted run.
Its receipt records `files_changed=false`. It did not inspect or confirm any
source root: exact targets remained unretrieved, the sole next action was a
read-only drill into the high-severity Finding, and exact source-root permission
remained a later explicit user decision rather than a trust claim.

## Decision

The failure was an Agent instruction boundary, not evidence for a new Rust
analysis subsystem. Keep the CLI contract unchanged. Finding counts describe
current scale; only a validated Plan may state proposed before/after impact,
and only a Receipt may state observed mutation impact. Initial diagnosis is
complete at the bounded Report unless one exact primary-decision fact is absent.
