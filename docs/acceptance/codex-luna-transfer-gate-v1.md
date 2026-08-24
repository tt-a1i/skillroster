# Codex Luna cold-routing transfer gate

Date: 2026-08-24 (Asia/Shanghai)

Scope: the bounded execution requested by Issue #145 and evidence toward Issue #129.
This is not completion of the broader cross-harness recovery goal.

## Outcome

The single frozen four-arm run **failed**. All four Codex processes exited
normally, prompt surfaces were correct, protected scopes stayed unchanged, and
workspace safety passed. No arm was retried.

| Task | Arm | Find | Exact load / order | Oracle | Result |
|---|---|---|---|---|---|
| Humanizer | Core | N/A | No / no | Failed | Control invalid |
| Humanizer | On-demand | Correct Top-1 after two invalid calls | No / no | Passed | Rejected |
| Architecture | Core | N/A | Yes / yes | Failed | Control invalid |
| Architecture | On-demand | Correct Top-1 on first call | Yes / yes | Failed | Rejected |

Both On-demand arms eventually retrieved the correct Skill and exact path. The
Architecture arm proved the intended Bootstrap -> Find -> exact load -> task
sequence. Humanizer recovered from two malformed Find invocations, but the
audit did not establish a full Skill read and classified the retries as a
contract violation.

The paired inference is deliberately `invalid_core_control` for both tasks.
Humanizer Core did not load the visible Skill before acting and missed the
minimum-length oracle. Architecture Core loaded its Skill in order but, like
On-demand, failed the self-contained HTML, topology, and Archify receipt gates.
These facts do not support blaming cold routing for either task failure.

## Frozen evidence

The run used Codex CLI `0.147.0`, model `gpt-5.6-luna`, revision `2623693`, and
the immutable `codex-transfer-replication-v1` manifest. The suite snapshot is
`17b524c365f87cea88c9f902802813c332bd3b6bea1323e0ba6869aaee6192ef`.
Execution ran from 13:17:28 to 13:24:22 +08:00.

The external summary was created once with mode `0600`; its SHA-256 is
`6ff164659c25c4bb2420d12c38559c69d30eb8fdda9dfd2a3ba607e14c3a3298`.
The retained private run tree has aggregate SHA-256
`a6365dbe5a5438816a3eb91ebe990c880308939fd7474c5b8b0e07b627acea44`.
No isolated authentication copy remained after completion.

The committed [redacted artifact](artifacts/codex-luna-transfer-gate-v1.json)
contains the reviewable facts and frozen digests. Raw prompts, transcripts,
absolute temporary paths, receipts, and credentials remain uncommitted.

## Product implication

This run supports the deterministic retrieval layer more than the end-to-end
Agent contract: both cold queries ultimately ranked the correct Skill first,
but only one Agent run followed the complete route cleanly. The next
investigation should focus on making Bootstrap, Find arguments, and exact Skill
loading easier for an Agent to execute, while keeping semantic intent and task
judgment in the model. More ranking heuristics are not justified by this run.
