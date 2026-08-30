# SkillRoster v1.8.36 release receipt

## Outcome

SkillRoster 1.8.36 closes two Agent continuation and local-read authority gaps.

- Commands that require a first Snapshot fail with the stable
  `snapshot_required` error plus one context-bound, read-only
  `scan --summary --json` continuation. The Agent does not have to reconstruct
  Home, state, explicit roots, source roots, or the running executable.
- Revoking a durable source-root permission immediately invalidates a current
  Snapshot that used it. Persistent root identity drift has the same effect.
- `find --load` rechecks durable read authority before returning content, so an
  old Snapshot cannot continue reading an external `SKILL.md` after revoke or
  replacement.
- Home, `status`, and typed failures expose bounded invalidating permission IDs,
  exact totals, and a Scan continuation. The new invalidation fields and typed
  failures do not repeat external paths or return Skill content.
- New Snapshots record only durable permissions that actually authorized a
  Skill read. Existing v1.8.35 payloads remain compatible through conservative
  inference from retained durable-read Placements; unused permissions and
  temporary one-Scan `--source-root` overrides do not cause false invalidation.

This release changes Agent continuation metadata, Snapshot read-authority
facts, and readiness validation. SQLite remains at schema 12, the JSON envelope
remains at schema 1, and bundled Bootstrap content remains at version 1.8.29.
No Agent or Skill files are changed by these checks.

## Source and review chain

[#325](https://github.com/tt-a1i/skillroster/pull/325) fixed the fresh-state
continuation at exact head `aae16d40188b49413226d29c231a37c186635b87`
and merged as `6420b38dd7df1c59ee09f39af69c8e25e4d00809`.

[#327](https://github.com/tt-a1i/skillroster/pull/327) fixed source-root
Snapshot authority at exact head
`631ca74b0cbbe526659e17ef034d49ebb82eeadc` and merged as
`082fbcead88eb8166c50312c6635f18075af5df4`. The original dogfood fixture
reproduced a successful old-Snapshot `find --load` after revoke; the corrected
binary returned `source_root_snapshot_rescan_required`, reported
`rescan_required`, and did not return the external Skill content. Independent
Spec/Safety and Standards/Compatibility reviews were Clean after their
findings were fixed.

[#328](https://github.com/tt-a1i/skillroster/pull/328) prepared version 1.8.36
at exact head `e64f39d245b7295fd068428b58feffee0d1ba87c` and merged as exact
release source revision `9a5fce3e57ef59fad7b2ccd4f5ff1242e0105846`.
The PR [CI run 33283348164](https://github.com/tt-a1i/skillroster/actions/runs/33283348164)
and exact-main [CI run 33283579580](https://github.com/tt-a1i/skillroster/actions/runs/33283579580)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The final local release-preparation gate passed 321 Rust unit tests, 8
acceptance tests, 101 CLI tests, and 152 Node harness tests, plus strict
Clippy, formatting, installation-surface validation, archive README validation,
the change-scope self-test, and `git diff --check`. Two sequential independent
release reviews were Clean after one overbroad documentation claim was
corrected.

## Candidate acceptance

The exact-main candidate
[run 33283749052](https://github.com/tt-a1i/skillroster/actions/runs/33283749052)
passed the strict repository gate, four platform build and governance jobs,
and the WSL2 governance smoke at exact revision
`9a5fce3e57ef59fad7b2ccd4f5ff1242e0105846`.

All four downloaded candidate checksums passed. Each archive README matched
the checked-in Git blob byte-for-byte. The macOS arm64 candidate reported
`skillroster 1.8.36` and passed the release-governance smoke.

## Published release

Annotated tag `v1.8.36` has tag object
`14774c6d4984711bc26355b9de01924464659006` and resolves to exact source
revision `9a5fce3e57ef59fad7b2ccd4f5ff1242e0105846`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33284198267)
repeated the strict gate, all four supported-platform jobs, and WSL2
successfully.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.36)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `dc6ef92ba78ef5e96a72d759b4e16ffc553e4a2310a55e229462743a2d5dee63` |
| `x86_64-apple-darwin` | `8a7e2a4d2fce6cce3387cef4db355a394e36541d6cbc7851857a109000201796` |
| `x86_64-pc-windows-msvc` | `847e66873c9dd4881a6a059e0cedcf598ba5e5b1b52fff7eb4036f2494f3f360` |
| `x86_64-unknown-linux-gnu` | `7b32a243365fee5d805f94bce07be801d7d2f7f82cb8968af4cc4e0067bc4c03` |

The public release asset inventory was read back and contained exactly those
eight uploaded files with matching service-side digests. An anonymous macOS
arm64 download passed its adjacent checksum, reported `skillroster 1.8.36`,
passed the release-governance smoke, and contained the exact checked-in archive
README bytes.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #10](https://github.com/tt-a1i/homebrew-skillroster/pull/10)
at exact head `ee9044a33f0ad9ad23c57626c954b5ba4c47352a`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33284775441)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33285019217)
was dispatched from tap `main` at
`cb5babe56475a818b3d2ecb52528b5b1caa62457` after test-bot passed. It
closed the PR without a regular merge commit, created equivalent Formula
commit `dbac10bcfe122f7c0aa7cb3f4a315c7fcf3c6e23`, published both bottles, and
advanced tap `main` to `2e56d1d1122ddf2e5eb26a8222e4513b0656912e`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `f17ae58187e2e52ccc6da89b7acb8f469fd56ef7dd57b739de8d2545d62fb0fc` |
| `x86_64_linux` | `72bc9854246f06b1e688a8670aa287bd3411dc5048842cdb2a433f924c4296e5` |

The public arm64 bottle was downloaded without repository credentials, matched
the Formula checksum, reported version 1.8.36, and passed the
release-governance smoke. Verification extracted the bottle in an isolated
temporary directory and did not mutate the user's installed Homebrew package.

## Boundaries

- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent files, or change
  the Bootstrap content version.
