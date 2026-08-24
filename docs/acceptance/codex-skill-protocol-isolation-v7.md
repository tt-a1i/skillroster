# Codex Skill protocol isolation v7

Date: 2026-08-24 (Asia/Shanghai)

Scope: Issue #149 under Issue #129. This frozen six-run comparison tests one
Skill exposed as Core against the same Skill governed as On-demand and activated
through one verified `find --load` result. It does not claim catalog-scale
routing quality or cross-harness generality.

## Outcome

The experiment is **formally eligible and passed its protocol gate**. Core
passed 3/3; On-demand passed 3/3. The frozen decision is
`retain_current_design`.

Every On-demand trial made exactly one audited Find call with the complete task
and one non-empty hint, returned `event-manifest` at Top-1 with its exact path,
and received the exact complete Skill content in the same verified JSON result.
No trial read the target `SKILL.md` again. All six trials produced the expected
JSON, changed only `outputs/events.json`, preserved the target Skill, exposed
Bootstrap, and authentication scopes, and completed with a valid transcript.

| Trial | Core | On-demand | Task / safety |
|---|---|---|---|
| 1 | Pass | Pass: one-call verified activation | Pass / pass |
| 2 | Pass | Pass: one-call verified activation | Pass / pass |
| 3 | Pass | Pass: one-call verified activation | Pass / pass |

The result supports retaining deterministic lexical retrieval plus Agent-owned
intent and hint generation. It does not justify embeddings, reranking, a built-in
model, MCP, or another routing Skill.

## Evidence boundary

The run froze the clean source commit/tree, CLI, Bootstrap, target package,
manifest, driver, Codex executable/version, model, reasoning effort, timeout,
sandbox, arm schedule, and pair invariants. All source and executable identities
remained stable. Runtime SQLite and Find audit files lived in a unique
sandbox-writable temporary boundary per trial; only the redacted audit was
copied into external raw evidence, and the temporary state was removed.

The target fixture treats a complete verified activation result as equivalent
to a complete visible-Skill load; the gate still rejects any redundant target
read. Earlier diagnostic attempts are not promoted: one exposed an unwritable
harness audit path, another exposed a contradictory legacy fixture contract,
and a later rerun exposed that the Core description no longer required direct
loading. The final fixture states the two activation branches explicitly.

The [redacted artifact](artifacts/codex-skill-protocol-isolation-v7.json) omits
prompts, transcripts, absolute paths, full Skill content, task output, and
authentication. Raw evidence remains outside the repository; isolated auth
copies remaining after the run: `0`.
