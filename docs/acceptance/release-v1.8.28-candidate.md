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

Candidate source revision `ff357bb96c5487197471eae67b9d3908ba5cb624`
passed the complete PR CI matrix in run
[`32809541414`](https://github.com/tt-a1i/skillroster/actions/runs/32809541414).
Linux x64, macOS arm64, macOS x64, and Windows x64 passed their required tests;
the CI gate passed.

Release-candidate run
[`32809543938`](https://github.com/tt-a1i/skillroster/actions/runs/32809543938)
passed for macOS arm64, macOS x64, Linux x64, Windows x64, and checksum-pinned
Ubuntu WSL. All four downloaded archives passed their adjacent SHA-256 checks.
All tar listings and the Windows zip integrity check passed. The packaged
macOS arm64 binary reported SkillRoster 1.8.28 and passed its help smoke test.

The same downloaded macOS arm64 candidate then performed a fresh read-only
real-home cold start against a temporary state directory:

- Scan completed in 6.47 seconds; Report in 0.59 seconds; Find in 0.13 seconds;
  Status in 0.01 seconds on this machine. These are observations, not portable
  performance guarantees.
- Report found 251 independent Skills, 887 placements, and 521 default
  exposures, matching the v1.8.27 baseline. It observed use for three Agents.
- Coverage remained conservative: zero complete Agents, five sampled/limited
  Agents, and three missing session roots. No unused claim is supported.
- The review task ranked `code-review`, `review`, and `github-code-review` as
  its Top 3 matches.
- Status reported zero pending Plans, no Receipt, clear recovery, and no
  journal issues.
- Scan, Report, Find, and Status each reported `files_changed: false`. Setup,
  Apply, and Undo were not invoked against the real home.

Candidate artifacts do not publish a tag or GitHub Release. Final publication
remains a separate user-authorized gate.
