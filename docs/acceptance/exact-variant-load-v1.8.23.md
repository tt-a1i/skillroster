# Exact variant load acceptance v1.8.23

This is a redacted receipt for Issue #151. It records a real-home, isolated,
read-only run without retaining Skill bodies, session content, or raw local
paths. The structured evidence is in
[artifacts/exact-variant-load-v1.8.23.json](artifacts/exact-variant-load-v1.8.23.json).

## Scope

- Baseline: `main@6096480`
- Candidate: `71dea6b`
- Snapshot: 260 independent Skills and 887 placements
- State: isolated under a temporary dogfood directory
- Mutation boundary: every Find and exact load returned `files_changed: false`

This receipt is intentionally bound to `71dea6b`, the executable used for the
dogfood run. Later review commits only scoped ranking evidence, suppressed
exact actions for a drifted Top-1 group, and added this receipt; those deltas
are covered by focused regression tests and are not retroactively claimed as
part of the dogfood execution.

## Observations

`goal-crafter` was a genuine same-name, divergent-entrypoint Top-1. Ordinary
Find returned two exact non-mutating load actions. Both requested identities
loaded successfully; their complete entrypoints differed in byte length and
SHA-256 digest, and each loaded identity matched the requested ID.

`humanizer-zh` and `agent-session-miner` each returned two exact actions and
all four loads passed the same checks. Within each family, the two entrypoint
digests were identical. Read-only directory comparison localized the package
difference to `.gitignore` (and an ignored `.git` directory). These are
fingerprint-noise evidence, not evidence of semantic entrypoint divergence.

## Decision

The exact selector closes the governed read path for true entrypoint variants
and also lets the Agent prove entrypoint equivalence. It does not resolve a
remaining package fingerprint difference or authorize canonicalization. The
source-control metadata finding is deferred to a separate issue so this change
does not weaken the existing safety fingerprint.
