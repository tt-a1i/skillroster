# SkillRoster v1.8.45 release receipt

The public CLI release is v1.8.45. This record separates source review, the
candidate build, the formal tag build, public downloads and Homebrew acceptance.

## User-visible changes

- Semantic-overlap analysis retains at most 25 candidates while preserving
  the previous ranked output. The recorded dense 5,000-Skill synthetic
  workload changed from 7,752 ms to 1,558 ms, with process peak RSS changing
  from 339,918,848 to 38,338,560 bytes. These are single-machine measurements,
  not a universal latency promise; pair enumeration remains quadratic.
- Recovery acceptance now injects storage-full/I/O errors and terminates
  Apply subprocesses at journal and target-publication checkpoints. A fresh
  process must preserve recovery evidence and refuse further mutation.
  These tests do not simulate physical power loss or hardware cache behavior.
- Copy and Replace preserve supported observable metadata or fail closed.
  Recursive copies finalize directory modes after copying children; Windows
  copy handles retain directory rename protection. Replace/Undo retain their
  mode/readonly guarantees and refuse observable metadata drift.
- Upgrade guidance now verifies which executable the Agent actually resolves
  after a package upgrade, and preserves recoverable backups when an older
  standalone binary shadows the intended installation.

The exact platform metadata matrix, unsupported layouts, experiment setup,
and regression limitations are in [hardening evidence](../hardening-evidence.md)
and [acceptance evidence](../acceptance.md#synthetic-overlap-scale-baseline-release-hardening-round).

## Compatibility

CLI source version is 1.8.45; bundled Bootstrap content remains 1.8.29.
SQLite and JSON envelope schema versions are unchanged. Unsupported ACL,
xattr, stream, or ownership layouts may now be refused instead of silently
losing metadata. Windows legacy replacement Receipts lacking original
security evidence need explicit manual recovery; other Undo operations and
Unix Receipts are not invalidated by that policy. Preservation of timestamps,
hard-link topology, privileged/invisible metadata, and Windows SACL settings
is not claimed.

## Source and verified fix

Preparation starts at `main@eb38952c49d8c4992129ea126ab44a2f9e480887`, the
merge of [PR #373](https://github.com/tt-a1i/skillroster/pull/373). It closes
issues #370, #371, and #372. The fix was independently reviewed on Standards
and Spec axes at exact head `8c9b0c0228f6c5acb9fddd56b32c180332cd74e3`.
Both axes had zero remaining findings.

That head passed the complete local gate: 341 unit tests, 8 acceptance tests,
122 CLI tests, 152 Node tests, strict Clippy, formatting, and installation /
archive documentation-surface checks. Its
[PR CI](https://github.com/tt-a1i/skillroster/actions/runs/33754750269)
passed Linux x86_64, Windows x86_64, both macOS architectures, and CI gate
using Rust 1.85. This evidence belongs to the fix head; candidate and final-tag
acceptance are separately recorded below.

## Candidate and exact release source

[PR #374](https://github.com/tt-a1i/skillroster/pull/374) prepared version
1.8.45 at exact head `c762843258a4da52f19d539930e53da0a6de6dd2`, with
zero remaining findings in independent Standards and Spec reviews. Its
[PR CI](https://github.com/tt-a1i/skillroster/actions/runs/33756007597)
passed; it merged as `25b39ebc279b5af5ae44c9c3cf9266da532eb693`.
The [exact-main CI](https://github.com/tt-a1i/skillroster/actions/runs/33756481903)
and [candidate workflow](https://github.com/tt-a1i/skillroster/actions/runs/33756507766)
passed at that source revision. Candidate acceptance included four archives,
adjacent checksums, source-identical README/LICENSE, WSL2 and isolated macOS
arm64 governance smoke. Candidate hashes are not reused as final tag hashes.

Upgrade-path documentation is independently reviewed in
[PR #369](https://github.com/tt-a1i/skillroster/pull/369).

## Published release

Annotated tag `v1.8.45` has tag object
`c1197c16fb6b6ef98acece486348089e27c9af69` and resolves to source
`25b39ebc279b5af5ae44c9c3cf9266da532eb693`.
The [tag workflow](https://github.com/tt-a1i/skillroster/actions/runs/33759099791)
passed all six jobs: strict exact-SHA validation with Rust 1.85, four platform
builds/portability tests/governance smokes, and WSL2.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.45)
was published on 2026-09-03 and contains exactly four archives and four adjacent
checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `cc6f0b89a49a3dcd2e6df56af6384640a6210e5e474d299be299f6e2b421c2c0` |
| `x86_64-apple-darwin` | `b719159e7333bbace83e287c5667c7d6505514bbf9765237f48f68d37c1be59e` |
| `x86_64-unknown-linux-gnu` | `6b2d5252666531c613b8e77aa075b03952bfa90aab9697400d2eaaf61ff1dfb1` |
| `x86_64-pc-windows-msvc` | `dce739c302438c386348628324a8b20639b42aae1eb66b363dcadcabdc986dee` |

Every archive has only its expected binary, README and LICENSE under the
versioned directory/path prefix. Tar archives include an explicit directory
entry; the Windows ZIP has three prefixed payload entries. README and LICENSE
bytes match the exact source. All eight files were independently downloaded
without authentication from the public Release URLs and matched the validated
tag artifacts byte-for-byte, including GitHub's reported asset digests.

Installing from the immutable Git tag with `cargo install --locked` into an
isolated temporary prefix reported 1.8.45 and passed the release governance
smoke. Both the macOS arm64 tag archive binary and the anonymously downloaded
public archive binary passed that smoke independently.
These checks exercise Setup/Apply/Undo, clear recovery state, a synthetic
retained-ID collision and stable repeat Scan; they use no real Skill library.

## Homebrew

The official [tap PR #19](https://github.com/tt-a1i/homebrew-skillroster/pull/19)
updated the Formula at exact head
`ac4d03a91d167f80b08f3e9d24382cd13d2f36f2`, with zero findings from
independent Standards and Spec reviews. Its
[test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33759373293)
built, installed and tested macOS arm64 and Linux x86_64 bottles. The
[SHA-guarded pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33761488909)
was dispatched from tap `main@7b7d445a7f43f30ab16602c997c3f1cc1f9ae864`
with that exact PR head. It published the bottles, closed the PR without a
regular merge commit, and advanced tap main through Formula commit
`db3af585e92d2894ecdc237cfe679f158b37f181` and bottle commit
`7107873827cd3ea8e1ec2a2dd93172b478d60371`.

The Formula uses the actual v1.8.45 source archive SHA-256
`bd1c065cfcccc1bce5e8749864de63120e96f2757355487e857f6acf1ef4ea77`.
The public [bottle release](https://github.com/tt-a1i/homebrew-skillroster/releases/tag/skillroster-1.8.45)
contains:

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `d9a1a5091c6c81d208743a50d99b4daf4481849967254476703ca11c664155fe` |
| `x86_64_linux` | `f85bf30512892d5362f0f73e5cdc355d85dba91abcc67887846531d43df9e2c2` |

Both bottles were anonymously downloaded and matched the public Formula and
GitHub asset digests. The extracted arm64 bottle reported 1.8.45 and passed the
full isolated release governance smoke. This did not install or upgrade the
user's Homebrew package. Linux bottle installation and runtime test evidence
comes from test-bot; it was not executed on the local macOS host.

## Boundaries

- Historical [asset policy #55](https://github.com/tt-a1i/skillroster/issues/55)
  is tracked separately from this release. The 21-release / 168-attachment
  inventory and backup are complete. No historical attachment was deleted or
  replaced; old-asset disposition is not claimed as completed by this release.
- Independent user-pilot work is outside this round.
- WSL2 uses the Linux archive. WSL1 mutation remains fail-closed; native Linux
  arm64 and Windows arm64 artifacts are not included.
- Publication does not change the user's installed binary, state, Agent files
  or Skill library. Installation smoke uses temporary prefixes and state.
