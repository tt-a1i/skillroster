# Codex On-demand transfer gate v1

Date: 2026-08-25 (Asia/Shanghai)

Scope: Issue #191 under #129. This is the single frozen execution of the
preparation merged in PR #190: three capability families, three fresh
On-demand trials per family, Codex CLI 0.147.0, `gpt-5.6-luna`, and medium
reasoning. It tests bounded Codex transfer of the existing one-call Find/load
seam; it is not a Core comparison or a universal cross-Agent claim.

## Outcome

The complete nine-run schedule executed once. The suite **failed** its frozen
all-runs gate: 8/9 runs were accepted, and `formal_gate_eligible` is false.
Source, Codex executable, and every frozen input identity stayed stable.

| Capability family | Accepted | Result |
|---|---:|---|
| Deterministic JSON artifact | 3/3 | passed |
| Reference-backed structured extraction | 3/3 | passed |
| Script-backed multi-file artifact | 2/3 | failed |

The failed trial first called Find with the complete task but without the
required hint/load shape. It then recovered with one exact canonical call,
received the correct Top-1 path and complete verified Skill content, and
passed the task oracle. The frozen scorer nevertheless correctly retained the
exclusive stage `retrieval_wrong`, because the protocol required exactly one
valid call.

The same trial was also marked unsafe by the transcript-side external-write
auditor. A double-quoted shell command redirected discovery noise to the null
sink; the parser retained the closing quote in the target spelling and
reported it as external. This is likely an audit-parser false positive, not
proof of an OS-level external write. The OS sandbox remained the actual write
boundary, workspace and protected scopes passed, and only the two allowlisted
outputs changed. Because the scorer was frozen, this result is not repaired,
reinterpreted, or rerun here.

The raw field `task_succeeded_without_loaded_skill: true` also needs careful
reading: exact complete content was loaded on the successful retry. In this
schema the flag means task success without satisfying the entire accepted
one-call load contract, not literal absence of loaded Skill bytes.

## Decision

Retain the current deterministic ranking and one-call mechanism. Eight of nine
runs were accepted; the remaining run failed both the one-call retrieval
contract and the frozen external-write audit, even though its recovery call,
exact load, and task oracle passed. That mixed result does not justify a new
ranker, model, or routing subsystem. Open only a narrow harness-correctness
follow-up for quoted redirection classification and the misleading bypass
field semantics. Do not rerun this frozen suite after that fix.

This run does not satisfy the original universal wording of #129. It does
provide a stopping result: bounded Codex transfer is useful but not perfectly
reliable, and further control-arm tuning is unlikely to change the current
product decision.

## Evidence boundary

- Source commit: `91424a38036a6d2414c4dcbccd04f7d044327d4b`.
- Raw summary SHA-256:
  `207367c999e22dd0c3bf140cac0921d8bd612000c5f85d9921e0af2d5178314b`.
- Out-of-band operator verification, separate from the raw summary, recorded
  that the first command was rejected before any run root or model invocation
  because the system temporary directory had a linked ancestor. The complete
  schedule was then invoked once under its canonical isolated root.
- Raw runs, transcripts, prompts, outputs, absolute paths, and authentication
  remain outside the repository.
- Out-of-band verification counted nine run roots and zero remaining copied
  `auth.json` files. The failed trial's second Find was an in-trial recovery
  call, not a suite-level rerun; no post-schedule rerun was performed.
- The public artifact is a bounded redacted projection; no real Agent or Skill
  directory was modified.

See the [redacted artifact](artifacts/codex-on-demand-transfer-gate-v1.json).
