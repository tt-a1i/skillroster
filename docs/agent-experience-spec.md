# Agent Experience Specification

SkillRoster is primarily used through an Agent conversation. The CLI is the deterministic local engine; the bootstrap Skill tells the Agent how to gather facts, prepare a complete Plan, present it clearly, obtain one user confirmation, and execute it safely.

## 1. Primary promise

A person should be able to say:

> 检查一下我电脑上的 Skills，给我一个合理方案。

In the same Agent turn, when evidence is sufficient, the Agent should:

1. run a read-only Scan;
2. generate the evidence-backed report;
3. make the semantic governance decisions outside the CLI;
4. submit those decisions to `plan --stdin`;
5. present the validated Plan and one primary action: **应用这份方案**.

Plan generation is read-only, so it should not require a separate user round trip. Apply always requires confirmation after the Plan is visible.

## 2. One-confirmation Apply

“One-click Apply” is a conversation contract, not an unsafe CLI flag:

```text
User intent
  -> Agent runs scan/report/plan --json
  -> Agent shows exact impact, exclusions, risk, and reversibility
  -> User confirms once: “应用”
  -> Agent runs apply <plan-id> --json
  -> Agent reports verification, Receipt, and Undo
```

The Agent does not ask for confirmation per file or operation. It must not collapse Plan review and Apply into the same unreviewed action, pass a force flag, or apply a stale or blocked Plan.

## 3. Conversation states

The Agent selects one primary action from current state:

| State | Agent response | Primary action |
|---|---|---|
| Not initialized | Explain local setup targets | 安装 SkillRoster |
| No Snapshot | Run Scan automatically | 查看检查结果 |
| Healthy, no useful change | Say no governance is needed | 保持现状 |
| Findings, sufficient Evidence | Prepare and show a Plan | 应用这份方案 |
| Findings, insufficient Evidence | Explain the exact unknown | 补充信息 or 保持现状 |
| Plan blocked or stale | Explain the blocker | 重新扫描并生成方案 |
| Applied | Show verification and Receipt | 完成 |
| Undo available | Show reverse impact on request | 撤销上次应用 |
| Recovery required | Stop unrelated writes | 处理恢复 |
| Bootstrap modified | Show affected targets and ask retain/adopt | 保留本地 or 采用当前版 |
| Bootstrap target unsupported | Explain the preserved link/non-file | 检查目标 |

The Agent never manufactures a problem so that an Apply action exists.

## 4. First-screen information

The initial response should fit roughly one conversation viewport. It contains:

1. **One-sentence diagnosis** with only evidence-backed numbers.
2. **Four core metrics:** independent Skills, placements, default exposure, and observed-use coverage. Usage coverage names sampled, complete, limited, missing-root, and inaccessible Agent counts separately.
3. **Top three Findings:** fact, impact, and Evidence quality.
4. **Proposed change:** current → proposed counts and affected Agents.
5. **Safety boundary:** read-only so far, canonical deletion count, reversibility, and uncertainty. For semantic Roster Plans, include forced, target-Agent, cross-Agent, and stable-fallback Core counts plus the bounded named Core preview and its reasons. Cross-Agent selections name the Agent that supplied exact-identity evidence and are described as used elsewhere, not as target-Agent usage. Fallback- or cross-Agent-dominated selection remains a review-required proposal.
6. **One primary action:** the exact phrase the user can reply with.

Do not lead with raw paths, every Finding, an aggregate health score, a wall of JSON, or unsupported token/performance estimates.

When the user asks which Skills were observed, drill into `Five-stage usage
evidence`. Use `usage_overview` for the five typed stage totals, aggregate
coverage boundary, and bounded high-signal Skill preview; distinguish Exposed
placements from later-stage events. Keep `skill_name`, canonical `agent`,
stage, quality, event count, and observation window together; the opaque Skill
ID remains the stable identity, not the user-facing label. Incomplete coverage
still forbids usage percentages and “unused” conclusions.

## 5. Ready Plan response

```text
SkillRoster 检查完成

你有 137 个独立 Skill，在 8 个 Agent 中形成 212 个安装或链接；
当前默认暴露 68 个，使用证据覆盖 6/8 个 Agent。

主要问题
1. 74 个精确重复副本，其中 4 组已经版本分叉
2. 7 个软链接失效
3. Codex 默认暴露 68 个，观测窗口内 14 个有使用证据

建议方案 · plan_01K...
- 默认暴露：68 → 21
- 重复副本：74 → 0
- 修复链接：7
- 归档候选：9
- 删除 canonical Skill 内容：0

影响 8 个 Agent；方案可撤销，漂移检查已通过。
目前仍是只读状态，没有修改任何文件。

回复“应用”即可执行整份方案；我会在完成后验证并给出 Receipt 与撤销入口。
```

The precise wording adapts to the user's language and the host Agent, but the facts, Plan ID, change boundary, and confirmation state remain present.

## 6. Applied response

```text
方案已应用并验证通过

- Plan：plan_01K...
- 变更：19 个链接，9 个 Roster 状态
- 验证：目标、链接和索引均符合方案
- Receipt：rcpt_01M...
- Canonical Skill 删除：0

如需恢复，回复“撤销这次整理”。
```

If Apply failed but compensation succeeded, say that no planned state remains and identify the Receipt. If recovery is required, lead with that state, list the unresolved target, and do not offer another Apply.

## 7. Drill-down behavior

Follow-up questions resolve against the same report and Snapshot unless the user explicitly asks to rescan:

- “为什么认为它重复？” → Finding, digests, source, placements.
- “哪些没观察到使用？” → time window, observable sessions, missing Agents, evidence level.
- “会改哪些文件？” → Plan operations grouped by Agent and action.
- “只处理断链” → generate a new narrower Plan; never mutate the existing Plan.
- “这个 Skill 保留” → generate a new Plan reflecting the exception.
- “撤销” → preview the reverse Receipt impact, then confirm once.

The Agent uses stable IDs internally and displays them where they help follow-up. Full paths and raw Evidence remain on demand.

## 8. Bootstrap Skill rules

The single `skillroster` bootstrap Skill must instruct every supported Agent to:

- invoke commands with explicit `--json`;
- validate `schema_version` and `ok` before reading `result`;
- set `schema_version: 1` on every declarative Plan request;
- never parse human terminal output or TUI frames;
- preserve a non-English Find task verbatim and supply a positive English
  target-surface, object, operation, and state paraphrase through `--hint` when
  relevant metadata may be English or the native-language result is uncertain;
- when hints are present, require task/hint rank fusion and retain the channel
  ranks so a hint cannot silently replace the user's original task evidence;
  high-ranked hint evidence must outrank weak overlap that only appears in both
  lexical channels;
- treat `suggested_actions` as typed options, not authorization;
- automatically prepare a read-only Plan when evidence is sufficient;
- show the complete Plan impact before Apply;
- require one explicit user confirmation for Apply or Undo;
- never use `--force`, `--yes`, hidden shell writes, or direct filesystem substitutes;
- preserve the same report/Snapshot across follow-up questions;
- state whether files changed in every final response;
- stop when drift, ambiguity, unsupported scope, or recovery-required state appears.

## 9. CLI data needed by the Agent

Machine results should expose facts, not preformatted prose:

- `primary_metrics` with value, unit, coverage, and comparison baseline;
- Findings with stable ID, category, severity, Evidence quality, impact fields, and affected IDs;
- a bounded selector-free `report --json` first view (`--summary` is an explicit alias) with no more than three Findings, one primary Evidence reference per selected Finding, and complete `finding_rollups` grouped by category, severity, and title with deduplicated affected counts; explicit `report --full --json` for exhaustive diagnostics; a category/severity-filterable `report --findings --json` page for enumerating compact Finding summaries; and compact paged Evidence items from `report --finding ID --json`; exact complete records require `--full`, exact-duplicate detail exposes at most five ranked owned canonical candidates, and large-Roster detail exposes bounded per-Agent Core selection previews without requiring pagination;
- Find results that keep the original task, expose Agent-authored retrieval hints, name the ranking strategy, retain task and augmented channel ranks, and collapse same-name identities into one explicitly variant capability result; the linked divergent-content Finding groups readable paths, digests, Agents, providers, and governance facts by variant and requires a canonical-content choice before Plan;
- bounded Plan `change_summary`, `operation_groups`, affected-scope counts and `impact` deltas for current and proposed state; semantic Roster summaries expose at most five named Core selections with reasons per Agent, while `plan --show PLAN_ID` exposes the exact immutable Core selections; `diff_summary` contains at most three semantic Roster, Library, and filesystem items, or bounded line details for a source update; full operations are loaded only through `plan --show PLAN_ID` when needed;
- affected Agents, Skills, placements, operation groups, exclusions, and deletion count;
- risk, reversibility, drift, confirmation, and recovery state;
- typed `suggested_actions` containing argv, mutation status, confirmation requirement, and reason code.

The Agent decides wording and information order from these fields. The CLI must not send ANSI, Markdown, localized prose, TUI frames, or an opaque “health score” through JSON. The first complete release does not include a human TUI.

## 10. Acceptance scenarios

### Disorder found

The Agent discovers 100+ Skills, creates a Plan in the same read-only turn, makes the largest problems obvious, and offers one Apply action.

### Healthy setup

The Agent reports that no change is justified and does not create artificial archive or consolidation work.

### Bootstrap upgrade

After a CLI upgrade, the Agent runs `setup`. Exact official older bootstrap
content produces a recoverable upgrade Plan. Locally modified content produces
no Plan until the person explicitly chooses retain or adopt; unsupported
targets remain untouched.

### Partial Evidence

The Agent distinguishes “not observed” from “unused,” names missing coverage, and avoids a destructive recommendation.

### External canonical source

The Agent leaves an escaping link unread, shows its target, and asks whether
the source is intentional. A confirmed source is rescanned through
`--source-root` without increasing any Agent's default exposure; unconfirmed
sources remain excluded.

### One-confirmation Apply

After the user says “应用,” the Agent executes the exact visible Plan, verifies it, and returns a Receipt without additional per-operation prompts.

### Drift and recovery

The Agent refuses stale Plans, does not bypass preconditions, and makes recovery the only mutating next action when required.
