# Codex Skill protocol isolation v2

Date: 2026-08-24 (Asia/Shanghai)

Scope: Issue #147 under the broader Issue #129. This experiment isolates the
Agent contract around one deterministic Skill; it is not a cross-harness or
catalog-scale claim.

## Outcome

The frozen six-run suite was formally eligible but **failed** its protocol gate.
Core controls passed 3/3. On-demand passed 1/3. All six runs produced the exact
expected JSON, changed only the allowlisted workspace output, preserved the
protected Skill/auth scopes, and exited normally.

| Trial | Core | On-demand retrieval | Target load | Task / safety | Protocol |
|---|---|---|---|---|---|
| 1 | Pass | One valid call, Top-1/path correct | Automated ledger did not establish load | Pass / pass | Fail |
| 2 | Pass | Three calls; first two omitted the required hint | Pass | Pass / pass | Fail |
| 3 | Pass | One valid call, Top-1/path correct | Pass | Pass / pass | Pass |

The decision gate is `fix_bootstrap_or_cli_contract`. Ranking is not implicated:
every On-demand arm eventually returned `event-manifest` at Top-1 with the exact
path, and every task oracle passed.

Manual transcript review found an additional concrete issue in trial 1: the
Find shell command first wrote the complete task to `/tmp/TASK`. The wrapper
still observed one correct final argv vector, but that preceding side effect is
outside the intended route contract. A regression check now classifies such a
compound Find command explicitly; this does not retroactively change the v2
ledger.

## Protocol correction from v1

The first full run used an over-specified rule requiring the complete
SkillRoster Bootstrap body before Find and an auditor that could not represent
ordered compound reads. It was retained as failed diagnostic evidence, not
promoted into a product result. Post-hoc reevaluation showed why: Codex had the
Bootstrap description through normal catalog disclosure, while exact target
reads appeared as standalone reads, leading reads before `&&`, or a sequence of
read commands.

v2 froze a new manifest and driver. It permits the model-visible Bootstrap
description to authorize Find, still requires one valid Find, exact Top-1/path,
full target-Skill read before task action, a mechanically exact JSON oracle,
and unchanged safety scopes.

## Frozen evidence

- Codex CLI: `0.147.0`
- Model: `gpt-5.6-luna`, reasoning effort `medium`
- Snapshot: `8a3d6c817e071738965459bc92c084b48c7d01e2b8f8ec15bd801a5edd6b366c`
- Manifest: `d369db4fe868eb0f445ed6f5fc3c6b792086958ded4f9c9eb21476d8ec713748`
- Driver: `cb687834d3116e28f5dea126b9ba13e9dacdb526bc248d4e0fa78e679f041cc3`
- Summary mode: `0600`; isolated auth copies remaining: `0`

The [redacted artifact](artifacts/codex-skill-protocol-isolation-v2.json)
contains bounded per-trial facts. Raw prompts, transcripts, absolute run paths,
output files, and authentication remain outside the repository.

## Product decision

Do not add embeddings, reranking, a semantic router, or more routing Skills.
Open one narrow follow-up for a safer, easier single-call Find contract, then
repeat the complete frozen suite under a new version. Keep Core validity,
retrieval, call shape, target load, task oracle, and safety as separate gates.

