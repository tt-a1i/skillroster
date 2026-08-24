# Codex symmetric transfer gate v1

Date: 2026-08-24 (Asia/Shanghai)

Scope: Issue #161 under Issue #129. This is one frozen four-run Codex suite at
`gpt-5.6-luna` with medium reasoning: one Core/On-demand pair for each of two
fresh capability families. It is a protocol experiment, not a catalog-scale
benchmark and does not extrapolate to other Agent harnesses.

## Outcome

The complete schedule ran exactly once from clean source commit
`36556bfe5b6375e6a31862e5330405cf7bca0120`. Source and Codex executable
identities remained stable. The overall result is **failed** and
`formal_gate_eligible: false`; it must not be used as product evidence for
cold-routing superiority.

| Family | Core | On-demand | Pair interpretation |
|---|---|---|---|
| Symmetric Chinese rewriting | Invalid Core control: no complete target load before task work; task oracle also failed | Exact Top-1 Find, complete verified load, route and safety passed; task oracle failed | `invalid_core_control` |
| Symmetric reference extraction | Invalid Core control: no complete target load before task work; task oracle passed | Exact Top-1 Find, complete verified load, task oracle, route and safety passed | `invalid_core_control` |

Aggregate: Core `0/2`, On-demand `1/2`; both family gates failed because the
Core control was invalid. All four runs had stable visible Skill surfaces,
valid transcript envelopes, and passed protected-scope/workspace safety checks.
The two On-demand runs each made one exact Find call and loaded the exact
complete target content in the verified JSON result. No ranking, embedding,
reranker, semantic infrastructure, or product claim is justified by this run.

## Frozen identity and evidence boundary

The run froze the source tree, manifest, driver, Bootstrap, CLI, target package
bytes, Codex executable/version, model, reasoning effort, timeout, sandbox,
arm schedule, and pair invariants. The run used `workspace-write` with
`exclude_tmpdir_env_var=true`, `exclude_slash_tmp=true`, and one unique
run-owned temporary grant. The explicit auth source was copied only into
isolated run homes; the driver reported zero remaining auth copies.

The redacted [artifact](artifacts/codex-symmetric-transfer-v1.json) records
the complete schedule and each dimension without prompts, transcripts,
generated content, credentials, or absolute run paths. Raw evidence remains
outside the repository. The frozen source/fixture/driver/Skill inputs were not
changed before or after execution, and no diagnostic run or second trial was
performed.

## Decision

Per Issue #161, stop at this evidence. The rewrite Core control is unsuitable
for attribution in its observed form; do not tune the control based on this
result inside the execution issue. The reference extraction On-demand path is
clean in this single trial, but the family pair cannot pass while its Core
control is invalid. Any follow-up must be a separately reviewed preparation or
control-contract issue before another execution issue is created.

See the parent [Issue #129](https://github.com/tt-a1i/skillroster/issues/129).
