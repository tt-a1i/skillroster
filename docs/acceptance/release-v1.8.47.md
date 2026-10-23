# SkillRoster v1.8.47 release receipt

Date: 2026-10-23. CLI **1.8.47** bundles Bootstrap **1.8.51**.

## User-visible changes

- Preserve observed Codex/Hermes local disable and platform rules during governance, including shared read-only paths and dependent consumers.
- Discover Codex shared Skills and Hermes nested/category paths without equating local visibility with a complete live session catalog.
- Bind search evidence and verified loading to content observations; conflicting copies with the same declared source/version require explicit placement selection.
- Preserve read-only review routing and reject malformed session records as usage evidence. Unknown coverage cannot justify archival or Roster reduction.
- Reuse advisory package reads within Find while independently validating actual loads.
- Explain these changes and the upgrade/rescan boundary in both READMEs.

## Source and validation

[PR #386](https://github.com/tt-a1i/skillroster/pull/386) merged as
`ab2f673420551cf8b2d002c383d2677016d5869c`. Its tree exactly matches accepted
candidate `v1.8.47-rc.4` / `0b669b1a9ef04a0af270d0a2378116e739742ef1`.

- [PR CI](https://github.com/tt-a1i/skillroster/actions/runs/39886996534): passed.
- [Candidate acceptance](https://github.com/tt-a1i/skillroster/actions/runs/39886994515): passed.
- [Merged-main CI](https://github.com/tt-a1i/skillroster/actions/runs/39887704242): passed.
- [Formal tag build](https://github.com/tt-a1i/skillroster/actions/runs/39888181441): passed.

The release workflow validates the exact SHA with Rust 1.85, then runs native
tests, builds, Plan/Apply/Undo smoke, and extracted-archive first-use checks on
Linux x86_64, Windows x86_64, macOS arm64 and macOS x86_64. The Linux archive also
passes checksum-pinned Ubuntu WSL2 acceptance.

Release preparation caught and fixed five Rust 1.85-incompatible let chains and
two Windows test-fixture assumptions: accidental insertion of a Posix identity
and comparison of noncanonical path strings. Failed/superseded candidates were
not published as Releases. The accepted candidate and formal tag passed all gates.
The full local gate passed 617 Rust tests and 152 Node tests, with one existing
ignored Rust test; formatting, strict Clippy and documentation guards passed.

## Published artifacts

Annotated tag `v1.8.47` has tag object `67d5ed511bd5af58f5c7e1dad91cc939a141ed00` and resolves to the merged SHA above.
The [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.47)
was published at **2026-10-23 02:21:47 UTC** and verified as Latest. It contains
four platform archives and four adjacent checksum files.

| Archive | SHA-256 |
| --- | --- |
| `skillroster-1.8.47-aarch64-apple-darwin.tar.gz` | `a15911c262a6598cd4d82475ec7b5437ae5aacde97df82bad8f08adbfffddd1b` |
| `skillroster-1.8.47-x86_64-apple-darwin.tar.gz` | `7fa5bffef1341096fe90ba65b4faa7fbd2addf2d147fce0b6122bd75efb57096` |
| `skillroster-1.8.47-x86_64-pc-windows-msvc.zip` | `6fb06046126e45154e34038b4dd7efadfeb4f845514ddc432f062ae90ea429e9` |
| `skillroster-1.8.47-x86_64-unknown-linux-gnu.tar.gz` | `20665bd8b26ffd9d85c95499f46237644ef01b968594fbac466d322f36610c3e` |

All eight public asset sizes and SHA-256 digests match the formal workflow
artifacts. Anonymous downloads of all eight files match those artifacts byte for
byte. Each archive contains only its executable, README and LICENSE under the
versioned prefix; README and LICENSE match the release source exactly.

The anonymously downloaded macOS ARM binary passed governance smoke and all four
first-use scenarios, including exact Bootstrap installation and Undo. A separate
`cargo install --locked --git ... --tag v1.8.47` into a temporary prefix also
passed the four scenarios and reports CLI 1.8.47 / Bootstrap 1.8.51.

## Homebrew

Homebrew delivery is tracked separately through [Tap PR #22](https://github.com/tt-a1i/homebrew-skillroster/pull/22).
The source tag archive SHA-256 is
`75e048cb7d68759aa8b5b3e3a829a24a7364a025fc92f58c78013457db09fa9d`.
The tap builds and publishes bottles through its own test-bot and SHA-guarded
pr-pull workflows; GitHub archive verification above is not bottle verification.

## Boundaries

Upgrade the CLI, rescan, and inspect optional Bootstrap updates before applying
any Setup Plan. Open a new Agent session after updating Bootstrap. Existing
Snapshots or Plans may require refreshed evidence; viewing history does not
establish that it remains applicable.

These controlled checks do not establish independent-user acceptance, complete
live session catalogs, model quality or token savings. WSL1 mutation, native
Linux arm64 and native Windows arm64 remain outside the release matrix. No user
CLI installation, real Agent configuration or Skill package was upgraded by this
release verification. Public website delivery is verified separately.
