# Routing content identity acceptance v1.8.24

This redacted receipt records a real-home, isolated-state, read-only run for
Issue #153. Structured evidence is in
[artifacts/content-identity-v1.8.24.json](artifacts/content-identity-v1.8.24.json).

## Scope

- Candidate: local `v1.8.24` debug build
- Agents checked: 8
- Snapshot: 251 independent Skills and 887 placements
- Prior comparable Snapshot: 260 independent Skills and 887 placements
- Mutation boundary: Scan, Report, Find, and exact loads returned no Agent file changes

## Observations

`humanizer-zh` and `agent-session-miner` each collapsed from two package
identities to one routing identity. Every previously observed placement remains
attached to the resulting identity; no copy was deleted or rewritten.

`goal-crafter` remains a genuine two-variant Top-1 result. Both exact selectors
loaded complete, different entrypoints (7,980 and 8,484 bytes), and both passed
Snapshot identity and complete package-fingerprint verification.

## Decision

The new identity removes source-control-only false variants without hiding
different Agent Skills payloads. The complete package fingerprint remains
independent and continues to protect exact loading and mutation workflows.
This run authorizes no canonical selection or filesystem change.
