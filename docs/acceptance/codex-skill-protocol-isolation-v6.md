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

The [redacted artifact](artifacts/codex-skill-protocol-isolation-v6.json)
contains bounded facts only. Raw prompts, transcripts, absolute run paths,
outputs, and authentication remain outside the repository; isolated auth copies
remaining after the run: `0`.
