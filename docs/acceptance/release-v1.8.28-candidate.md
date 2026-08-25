# SkillRoster v1.8.28 candidate

## Release notes draft

SkillRoster 1.8.28 makes local Skill governance facts more accurate and keeps
ordinary Agent continuations bounded.

- Coverage evidence uses canonical public Agent IDs and typed, actionable
  limitations. Legacy coverage stays readable but cannot support a reliable
  usage denominator until a new Scan records current facts.
- Escaping-link continuation separates durable read permission from a temporary
  one-Scan source override.
- Stable placement identities now refresh every current-projection field on
  rescan, while immutable Snapshot payloads retain historical facts.
- Historical Findings expose bounded current continuity through stable
  placement IDs, without loading a complete current Report.
- Bootstrap Setup returns a decision-complete bounded Plan summary without
  embedding operations, file bodies, or fingerprints.
- Analysis-only requests stop at the bounded Report. Expected impact comes from
  a validated Plan, and actual impact comes from its Receipt.

The release keeps schema v10, JSON envelope schema 1, local-only operation,
explicit confirmation, and reversible Apply/Undo. Strict external JSON
consumers must tolerate additive fields. Existing legacy coverage remains
`legacy_unknown` until rescanned.

## Preparation evidence

The source baseline is public v1.8.27; the preparation branch starts at
`c479fc8`. Cargo, installation examples, presentation fixtures, and the bundled
Bootstrap content are versioned as 1.8.28.

The v1.8.27 Bootstrap package contained four managed files at content version
1.8.23. Because its governance reference changed after that release, v1.8.28
records the exact four-file v1.8.23 manifest instead of treating the official
copy as a local modification.

An isolated macOS arm64 upgrade used the exact files from tag v1.8.27:

- Setup reported one `official_outdated` target, zero modified targets, and two
  `replace_file` operations.
- Preview changed no files; Apply verification passed and changed two paths.
- A second Setup reported `up_to_date` at Bootstrap content version 1.8.28.
- Undo verification passed and restored all four v1.8.27 files byte-identically.

## Candidate gates

The release-candidate workflow must still prove the packaged binary on macOS
arm64, macOS x64, Linux x64, Windows x64, and checksum-pinned Ubuntu WSL. The
four archives and adjacent SHA-256 files must be downloaded and independently
verified. A packaged v1.8.28 binary must then repeat the read-only real-home
cold start without Apply or user-file changes.

Candidate artifacts do not publish a tag or GitHub Release. Final publication
remains a separate user-authorized gate.
