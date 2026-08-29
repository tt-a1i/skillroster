# SkillRoster v1.8.34 release receipt

## Outcome

SkillRoster 1.8.34 makes the documentation inside every immutable platform
archive trustworthy for the lifetime of that archive.

- Linux, Windows, macOS arm64, and macOS x86_64 archives package one
  version-neutral guide instead of copying the repository README.
- The guide tells users to verify the adjacent checksum, identify the exact
  binary with `skillroster --version`, and use the public Releases page for
  current version evidence.
- Packaging rejects a linked source path on Unix and a reparse-point source or
  ancestor on Windows before reading or copying the guide.
- CI rejects hard-coded release versions and requires LF checkout bytes on
  every platform. Each packager verifies the extracted README against the
  checked-in source.

This patch changes release packaging and validation only. SQLite remains at
schema 12, the JSON envelope remains at schema 1, and bundled Bootstrap content
remains at version 1.8.29. There is no database migration or change to the
local-only, explicit-confirmation, Receipt-backed mutation model.

## Source and review chain

[#317](https://github.com/tt-a1i/skillroster/pull/317) implemented the archive
contract and closed [#314](https://github.com/tt-a1i/skillroster/issues/314).
Independent Spec and Standards reviews were Clean after Unix symlink and
Windows ancestor-junction findings were fixed. External archive readback also
caught a Windows LF-to-CRLF conversion that the first green packaging run had
missed; `.gitattributes` now pins the guide to LF, and packaging verifies the
effective Git attribute.

[#318](https://github.com/tt-a1i/skillroster/pull/318) prepared version 1.8.34
and merged as exact source revision
`188335ffe3eef570d403c344c954b366faaad0b5`. Its exact-main candidate
[run 33275243875](https://github.com/tt-a1i/skillroster/actions/runs/33275243875)
passed the strict repository gate, Linux, Windows, both macOS architectures,
and WSL2. Four downloaded candidate checksums passed, and all four archive
READMEs matched the checked-in Git blob byte for byte.

The candidate gate included:

- `cargo fmt --all --check`;
- strict Clippy across all targets and features;
- 321 Rust unit tests, 8 acceptance tests, 97 CLI tests, and 152 Node harness
  tests;
- installation-surface, archive-README, change-scope, and `git diff --check`
  validation;
- four platform build/governance jobs and the WSL2 governance smoke.

## Published release

Annotated tag `v1.8.34` has tag object
`3496c4f7ef1af1eed530cb77729671e9ecc65c72` and resolves to exact source
revision `188335ffe3eef570d403c344c954b366faaad0b5`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33275766709)
repeated the strict gate and all supported-platform jobs successfully.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.34)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `ced3b792cfa57cd12bfe47a17404ea09bf0bef01d757fd49c512da6f1b3eae0f` |
| `x86_64-apple-darwin` | `106b43962963a140e1b030e2c1441381466d775a8878ef8187de60374dc6ad1a` |
| `x86_64-pc-windows-msvc` | `05d3f3ccc67bb58a547689e98529d0e78cf5f442ab73b52336768b0717454430` |
| `x86_64-unknown-linux-gnu` | `33bc9062ca525abc1a71bd59755d202e0cf3091e8e45388b823c027b61b0f0fe` |

The public release asset inventory was read back and contained exactly those
eight files. An anonymous macOS arm64 download passed its adjacent checksum,
reported `skillroster 1.8.34`, passed the release-governance smoke, and
contained the exact checked-in archive README bytes.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #8](https://github.com/tt-a1i/homebrew-skillroster/pull/8)
at exact head `db693ef878fff14733f1de51049bb3449d08e3ad`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33276415624)
built and tested both macOS arm64 and Linux x86_64 bottles. The exact-head
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33276709861)
published the bottles and advanced tap `main` to
`0c15049251853e5c1a274bbbbcfefbb4eebb4450`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `55602a66c8ae05f9f3b36163831bb047821e62a140054a7857ce7925a0181ad8` |
| `x86_64_linux` | `d47197b9dee091757821c01053490ed5a224985ba035e5a053425926a6a495ed` |

The public arm64 bottle was downloaded without repository credentials, matched
the Formula checksum, reported version 1.8.34, and passed the release-governance
smoke. The local Homebrew installation was then upgraded from 1.8.28 to 1.8.34
and passed the same checks using `/opt/homebrew/opt/skillroster/bin/skillroster`.

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
