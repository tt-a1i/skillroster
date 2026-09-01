# SkillRoster v1.8.44 candidate

## Release notes draft

SkillRoster 1.8.44 makes Bootstrap Setup previews truthful and binds Apply to
the exact package identity that the preview promised.

- Setup now reports the exact post-Apply Bootstrap Skill identities instead of
  showing zero Skills when it has planned a Bootstrap installation or upgrade.
- The impact summary counts logical Agent placements independently from
  deduplicated physical package targets, so shared roots no longer hide who is
  affected.
- Each new mutating Bootstrap Setup Plan records package-level before and after
  fingerprints. Apply validates them through retained root handles and
  compensates managed writes if the final package projection does not match.
- Identity-relevant preserved files within the bounded package set remain part
  of content identity. A change to one of those files between Setup and Apply
  fails closed with typed `plan_drifted` evidence and no managed files left
  changed. The documented package exclusions remain outside this boundary.
- Scan, Setup projection, and handle-bound Apply now share one bounded package
  hashing implementation: depth 8, 16 MiB total, and the same exclusions for
  `.git`, `target`, `node_modules`, and `.DS_Store`.
- Projected relative paths are normalized by components before hashing, so the
  promised and materialized package identities also match on Windows.
- Legacy Plans without complete package projections keep their historical
  digest compatibility but are not reused as complete Setup previews.

The real-inventory dogfood used a fresh read-only Snapshot of 263 Skills and
1,037 placements with isolated temporary state. Before the fix, Setup planned
25 operations for six detected Agents but reported zero Skills and zero
placements. After the fix, the same preview reports one exact projected Skill,
six logical placements, four deduplicated physical target packages, and the
same 25 operations, with confirmation still required and no files changed.

This patch changes Bootstrap impact projection and package postcondition
validation. SQLite remains at schema 12, the JSON envelope remains at schema 1,
and bundled Bootstrap content remains at version 1.8.29. It does not mutate
Agent or Skill files during Scan or Setup.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`233fc7c51f75e21a8738ac1f7b429407ebb52c90`.

[#365](https://github.com/tt-a1i/skillroster/pull/365) implements the fix at
exact head `30ca44b850de9a363fa2fc2c32f2c4a0ac18d464` and merged as revision
`233fc7c51f75e21a8738ac1f7b429407ebb52c90`. The linked
[issue #364](https://github.com/tt-a1i/skillroster/issues/364) records the
public reproducer, bounded package boundary, and acceptance criteria.

Two independent exact-head reviews passed with no blocking findings. The PR
[CI run 33328908779](https://github.com/tt-a1i/skillroster/actions/runs/33328908779)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate using Rust 1.85. The first CI attempt exposed an MSRV
let-chain incompatibility; the second exposed Windows path-key divergence.
Both were fixed and the complete matrix was rerun rather than waived.

At the final fix head, the full local gate passed 331 Rust unit tests, 8
acceptance tests, 122 CLI tests, and 152 Node harness tests, plus strict Clippy,
formatting, installation-surface validation, archive README validation, the
change-scope self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.44. Public install
examples, website current-release labels, README release evidence, and the
Homebrew Formula deliberately remain at v1.8.43 until the v1.8.44 tag,
artifacts, checksums, and Homebrew package actually exist.

## Candidate gates

The candidate is not accepted until one exact final source revision passes:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and Node harnesses;
- `git diff --check`, installation-surface validation, archive README
  validation, and the CI change-scope self-test;
- four platform build and governance jobs plus the WSL2 governance smoke;
- downloaded checksum verification and an external four-archive comparison
  against the checked-in README and LICENSE Git blobs.

Candidate preparation does not create or push a tag, publish a GitHub Release,
update Homebrew, or mutate an existing public release asset. Those operations
remain separately evidenced after candidate acceptance.
