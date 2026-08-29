# SkillRoster v1.8.35 release receipt

## Outcome

SkillRoster 1.8.35 makes Bootstrap Setup Plans report their real Agent impact.

- `setup --json` and `plan --show` include every logical Agent whose Bootstrap
  target will actually be installed or replaced.
- Agents that share one physical root remain distinct in the bounded impact
  summary, so deduplication cannot hide affected callers.
- Already-current, unsupported, and retained-local targets are not counted as
  affected.
- Complete Plans created by older versions with an incorrect zero-Agent
  summary remain immutable, but Setup does not reuse them.

This patch changes Setup Plan reporting and reuse identity only. SQLite remains
at schema 12, the JSON envelope remains at schema 1, and bundled Bootstrap
content remains at version 1.8.29. Setup is still preview-only until explicit
Apply confirmation, and Apply/Undo remain Receipt-backed.

## Source and review chain

[#321](https://github.com/tt-a1i/skillroster/pull/321) fixed
[#320](https://github.com/tt-a1i/skillroster/issues/320) at exact head
`3e259fdbec93c34160898a725f6ef8efae3d239b`. The public v1.8.34 binary first
reproduced the defect with six detected and missing Agents, 25 planned
filesystem operations, four physical targets, and an incorrect zero-Agent
summary. The corrected source created a new Plan, reported all six logical
Agents, and did not modify the Home fixture. Independent Spec and Standards
reviews were Clean.

[#322](https://github.com/tt-a1i/skillroster/pull/322) prepared version 1.8.35
and merged as exact source revision
`8764f2d71a25697c77af5652616083b0f77ec6fc`. Its exact-main candidate
[run 33278766650](https://github.com/tt-a1i/skillroster/actions/runs/33278766650)
passed the strict repository gate, Linux, Windows, both macOS architectures,
and WSL2. All four downloaded candidate checksums passed, every archive README
matched the checked-in Git blob, and the macOS arm64 binary reported 1.8.35 and
passed the release-governance smoke.

The candidate gate included:

- `cargo fmt --all --check`;
- strict Clippy across all targets and features;
- 321 Rust unit tests, 8 acceptance tests, 98 CLI tests, and 152 Node harness
  tests;
- installation-surface, archive-README, change-scope, and `git diff --check`
  validation;
- four platform build/governance jobs and the WSL2 governance smoke.

## Published release

Annotated tag `v1.8.35` has tag object
`834b936466cd027c89575310bdb5677147b08668` and resolves to exact source
revision `8764f2d71a25697c77af5652616083b0f77ec6fc`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33279179458)
repeated the strict gate and all supported-platform jobs successfully.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.35)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `ea08604862b35ee4dc323cff95c4e478bc8c007c79d3b93f5b47ffc2bc949e2c` |
| `x86_64-apple-darwin` | `12bd2f32049794196e406ff61e8c3cf641c66a12d9554b5175d32f75dd87cf63` |
| `x86_64-pc-windows-msvc` | `905efd7cc8131e828ce605202c716ff3fb4008b3de9203da1c27ee8d8f66a527` |
| `x86_64-unknown-linux-gnu` | `3dbfa430c0ed475398d19b4ebdd9ee1cf349775aa91cec785fd4936441883a1c` |

The public release asset inventory was read back and contained exactly those
eight files. An anonymous macOS arm64 download passed its adjacent checksum,
reported `skillroster 1.8.35`, passed the release-governance smoke, and
contained the exact checked-in archive README bytes.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #9](https://github.com/tt-a1i/homebrew-skillroster/pull/9)
at exact head `055807ec3c7311dadf7e61331862eedb0e85ff06`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33279788451)
built and tested both macOS arm64 and Linux x86_64 bottles. The exact-head
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33280033423)
published the bottles and advanced tap `main` to
`cb5babe56475a818b3d2ecb52528b5b1caa62457`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `58924df4b2135196bdab0a1abb86ba73d927ad7ced87a11014918bb50d81dbfe` |
| `x86_64_linux` | `039bfffeb6694068e6d27d4464202e642217004569a26f767cf87c349e5b909f` |

The public arm64 bottle was downloaded without repository credentials, matched
the Formula checksum, reported version 1.8.35, and passed the release-governance
smoke. The local Homebrew installation was upgraded from 1.8.34 to 1.8.35 and
passed the same checks using `/opt/homebrew/opt/skillroster/bin/skillroster`.

The user's existing `~/.local/bin/skillroster` remains version 1.8.28 and
precedes Homebrew on `PATH`. It was deliberately preserved: invoking
`skillroster` by name in that shell still resolves to the user-owned older
binary, while the verified Homebrew binary is available by its absolute path.

## Boundaries

- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent files, or change
  the Bootstrap content version.
