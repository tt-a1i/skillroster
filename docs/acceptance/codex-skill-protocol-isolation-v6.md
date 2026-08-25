# Codex Skill protocol isolation v6

Date: 2026-08-24 (Asia/Shanghai)

Scope: Issue #147 under Issue #129. This is a frozen, deterministic comparison
of one Skill exposed as Core versus governed as On-demand. It does not claim
catalog-scale routing quality or cross-harness generality.

## Outcome

The six-run experiment is **formally eligible and failed its protocol gate**.
Core passed 3/3; On-demand passed 1/3. The frozen decision is
`fix_bootstrap_or_cli_contract`.

All three On-demand trials made one audited Find call, returned
`event-manifest` at Top-1 with the exact path, loaded the complete target Skill,
produced the exact expected JSON, changed only the allowlisted workspace output,
preserved protected Skill/auth scopes, and exited normally. Ranking and task
execution therefore passed 3/3. The two rejected trials failed only the ordered
Agent contract:

| Trial | Core | On-demand protocol | Task / safety |
|---|---|---|---|
| 1 | Pass | Fail: attempted a workspace-opening MCP call after Find and before target load; the call was cancelled | Pass / pass |
| 2 | Pass | Fail: valid Find argv was wrapped in an unsafe shell shape | Pass / pass |
| 3 | Pass | Pass | Pass / pass |

## Efficiency and score-difference analysis

The retained raw `turn.completed` records allow a post-hoc token comparison.
Cache utilization is defined as `cached_input_tokens / input_tokens`; it is a
cost/context-reuse measure, not a task-quality score. Precise wall-clock latency
was not captured by the v6 runner and cannot be reconstructed honestly.

| Trial | Arm | Input | Cached input | Uncached input | Cache utilization | Output | Reasoning output | Successful commands |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Core | 39,484 | 25,088 | 14,396 | 63.54% | 495 | 165 | 2 |
| 1 | On-demand | 161,768 | 137,216 | 24,552 | 84.82% | 1,608 | 719 | 5 |
| 2 | Core | 39,353 | 25,088 | 14,265 | 63.75% | 430 | 104 | 2 |
| 2 | On-demand | 69,937 | 49,152 | 20,785 | 70.28% | 898 | 260 | 4 |
| 3 | Core | 39,329 | 22,016 | 17,313 | 55.98% | 458 | 128 | 2 |
| 3 | On-demand | 88,523 | 53,248 | 35,275 | 60.15% | 1,206 | 368 | 5 |
| **Total / weighted** | **Core** | **118,166** | **72,192** | **45,974** | **61.09%** | **1,383** | **397** | **6** |
| **Total / weighted** | **On-demand** | **320,228** | **239,616** | **80,612** | **74.83%** | **3,712** | **1,347** | **14** |

On-demand used 202,062 more input tokens (+171.0%), including 34,638 more
uncached input tokens (+75.3%), and its weighted cache utilization was 13.73
percentage points higher. The higher cache share did not make it cheaper: most
of the extra cached tokens came from a longer repeated prefix and additional
tool-turn context, while the additional Find/load/order work increased both
fresh context and output/reasoning tokens.

The protocol score gap (Core 3/3 versus On-demand 1/3) was not a task-quality or
ranking gap. Retrieval, exact load, task oracle, and safety were 3/3 in both
arms. Two On-demand trials lost only the ordered-contract point: one attempted
two MCP actions before target load, and one wrapped a valid Find argv in an
unsafe shell shape. The extra 2-3 successful commands per On-demand trial are
consistent with the larger token footprint, but six runs are too few to claim a
stable causal cost multiplier. The v7 one-call contract removed the protocol
score gap; its raw usage evidence was intentionally deleted, so no v7 token or
cache comparison is claimed.

The result does not justify embeddings, reranking, a semantic router, or more
routing Skills. It supports a narrower follow-up: make the Bootstrap/CLI seam
easier for an Agent to execute as one safe, observable operation while keeping
semantic intent with the model.

## Evidence boundary

The run froze source commit/tree, clean-worktree state, CLI/Bootstrap/target
digests, Codex version and executable content, model, reasoning effort, timeout,
sandbox, arm schedule, and pair invariants. The post-run source and executable
identities matched. Preflight used the same model configuration in a fresh
config-free `CODEX_HOME`; Codex `debug prompt-input` does not accept the
execution-only `--ignore-user-config`, sandbox, or ephemeral flags, and that
difference is explicitly recorded in the snapshot.

Full load requires exact content echo from a standalone read, a same-target
read sequence, or a compound command whose suffixes are all conservatively
classified as read-only. A successful read command without content echo is not
promoted to load evidence. The deterministic fixture also declares this
standalone-load contract in Skill metadata; the natural user prompt does not
mention SkillRoster, Find, capability search, or the target Skill.

v2 remained diagnostic after independent review invalidated its frozen identity.
v3 through v5 were never retroactively promoted: they exposed formal-eligibility,
shell-audit, binary-identity, preflight, and Core-control gaps that v6 closes.

The [redacted artifact](artifacts/codex-skill-protocol-isolation-v6.json) is a
bounded projection that preserves the machine violation strings alongside
human-normalized labels. Raw prompts, transcripts, absolute run paths,
outputs, and authentication remain outside the repository; isolated auth copies
remaining after the run: `0`.
