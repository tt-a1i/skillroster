# SkillRoster v1.8.44 release receipt

## Outcome

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

## Source and review chain

Release preparation started from exact `origin/main` revision
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

[#366](https://github.com/tt-a1i/skillroster/pull/366) prepared version 1.8.44
at exact head `113cc1c2d201a0b4ddadc350042e8c71647f6241` and merged as exact
release source revision `c79b08346552814b6b4438a4f87964f9ab9a06a0`.
The PR
[CI run 33329426160](https://github.com/tt-a1i/skillroster/actions/runs/33329426160)
and exact-main
[CI run 33455660320](https://github.com/tt-a1i/skillroster/actions/runs/33455660320)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at their exact revisions. Two independent exact-head
reviews passed after one release-note scope finding was corrected. The complete
local candidate gate passed 331 Rust unit tests, 8 acceptance tests, 122 CLI
tests, and 152 Node harness tests, plus strict Clippy, formatting,
installation-surface validation, archive README validation, the change-scope
self-test, and `git diff --check`.

## Published release

Annotated tag `v1.8.44` has tag object
`9f712cee1d06a9016eaae87ca2dd242853431031` and resolves to exact source
revision `c79b08346552814b6b4438a4f87964f9ab9a06a0`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33455956695)
passed the strict repository gate, all four supported-platform jobs, each
governance smoke, and WSL2 at that exact revision.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.44)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `1546b4711c5a2a64c1771f236a3f74786aff7caa8df79c45cb499becfe63203f` |
| `x86_64-apple-darwin` | `ece76757edaec6aea3c2b7aa078b558299320b70cd6c9c144c0644896dba23b7` |
| `x86_64-pc-windows-msvc` | `b2b8cb46f8c386ffab4160d0fc798bdd14707b1cab8f30dc336fb437406ca412` |
| `x86_64-unknown-linux-gnu` | `608d5cb4884d2f02d52a42635c099c99c1dd4fa539a47e9498e5651970f4a407` |

All adjacent checksums passed. Every archive kept its binary, README, and
LICENSE under one versioned top-level directory or path prefix; the tar archives
also contained an explicit directory entry, while the Windows ZIP contained
only the three prefixed payload entries. Each README and LICENSE matched the
checked-in Git blob byte-for-byte. The public asset inventory contained exactly
those eight files. An anonymous macOS arm64 download matched its adjacent
checksum and the tag-workflow artifact byte-for-byte, reported
`skillroster 1.8.44`, and passed the release governance smoke in isolated
temporary directories.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #18](https://github.com/tt-a1i/homebrew-skillroster/pull/18)
at exact head `9c5c80e44815605b137b0f5c479e46e090101420`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33456916677)
built, installed, and tested macOS arm64 and Linux x86_64 bottles at that PR
head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33457382749)
was dispatched from tap `main` at
`f141e46f3f492fae25fdc1d69f2b5d6527448df5` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`503ba7012333e7975a4b0bb860a2367b151ce9fe`, published both bottles, and
advanced tap `main` through bottle commit
`7b7d445a7f43f30ab16602c997c3f1cc1f9ae864`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `9c6611bf0ed693efc7da1fbc14b1857aa1eafbbad09ab4d210b86fcd9918ec2c` |
| `x86_64_linux` | `344fdbdc317926759cfd29863a3659a586ce0de4a387ec34c2a38d482dc1f6d4` |

The public Formula source checksum matches an independent v1.8.44 tag archive
download. An anonymous arm64 bottle download matched its Formula checksum,
reported version 1.8.44, and completed an isolated `scan --summary --json`
smoke with `ok=true` and `files_changed=false`. Verification extracted the
bottle in a temporary directory and did not mutate the user's installed
Homebrew package.

## Boundaries

- Package postconditions cover new mutating Bootstrap Setup Plans, not every
  historical or non-Bootstrap Plan.
- Identity-relevant retained files inside the bounded package set participate
  in drift checks; documented package exclusions remain outside that boundary.
- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
