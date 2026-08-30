# SkillRoster v1.8.38 release receipt

## Outcome

SkillRoster 1.8.38 makes Agent routing obey explicit task exclusions and makes
verified loading prefer the Skill placement that is actually exposed to an
Agent.

- `find` recognizes bounded, independent `不要` / `也不要` / `do not`
  clauses as negative capability constraints instead of positive task
  evidence. A strong retrieval hint cannot override an explicit exclusion.
- JSON keeps the exclusion decision auditable: `task_exclusions` preserves the
  recognized clauses, while `task_exclusion_effects` provides a bounded
  10-item preview with complete count and truncation facts. Neither returns
  Skill content.
- After Skill identity ranking, `find --load` prefers an eligible
  `default_exposed` placement over hidden exact-copy inventory such as
  `.bak-*` directories.
- Hidden copies remain inventory and duplicate evidence. When no exposed copy
  exists, source-only and On-demand Skills retain a deterministic eligible
  fallback.
- Existing root, trust, digest, fingerprint, UTF-8, size, and drift checks are
  unchanged and still fail closed.

Real-inventory replay used a fresh Snapshot of 263 Skills and 1,037
placements. Explicit modification exclusions no longer routed to modification
Skills, while the positive task kept its prior score. `code-review` loaded
from an Agent-owned exposed placement instead of a `.bak-*` copy. Both replays
reported `files_changed=false`.

This patch changes Find routing and verified placement selection only. SQLite
remains at schema 12, the JSON envelope remains at schema 1, and bundled
Bootstrap content remains at version 1.8.29. It does not mutate Agent or Skill
files.

## Source and review chain

[#335](https://github.com/tt-a1i/skillroster/pull/335) fixed explicit task
exclusions and merged as `30ce00dde95567d28b4e567078120fb446770fbf`.
The linked [issue #334](https://github.com/tt-a1i/skillroster/issues/334)
records its reproducer and acceptance boundary.

[#337](https://github.com/tt-a1i/skillroster/pull/337) fixed exposed-placement
loading at exact head `e1dd36c95e4eb4de341df74368d9abdfcc23b1a4` and merged as
`03dc398cd7ca8050eac64ad28987b5bf25e8e9fa`. The linked
[issue #336](https://github.com/tt-a1i/skillroster/issues/336) records the real
`.bak-*` load failure, root cause, and bounded acceptance criteria.

[#338](https://github.com/tt-a1i/skillroster/pull/338) prepared version 1.8.38
at exact head `0d32e67fd456f7d2d4c061f8fd1b06d7adf2db3f` and merged as exact
release source revision `74243e5f1a9e283ef08be9a69c2dce9de0274833`.
The PR [CI run 33296194445](https://github.com/tt-a1i/skillroster/actions/runs/33296194445)
and exact-main [CI run 33296351682](https://github.com/tt-a1i/skillroster/actions/runs/33296351682)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The final local gate passed 325 Rust unit tests, 8 acceptance tests, 112 CLI
tests, and 152 Node harness tests, plus strict Clippy, formatting,
installation-surface validation, archive README validation, the change-scope
self-test, and `git diff --check`. Sequential Spec/Evidence and
Standards/Compatibility reviews were Clean.

## Published release

Annotated tag `v1.8.38` has tag object
`a4058a50a8492004b66e278c35d879d90a0cfa82` and resolves to exact source
revision `74243e5f1a9e283ef08be9a69c2dce9de0274833`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33296693186)
passed the strict repository gate, all four supported-platform jobs, the
governance smoke, and WSL2 at that exact revision. The first strict-gate
attempt encountered a transient Linux `ETXTBSY` while replacing a running test
executable; the failed job was rerun at the same SHA and passed without a
source or tag change.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.38)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `daea023385e9699db530d86ea7bf68a796a214bfec2a81c8fb9851c31b875536` |
| `x86_64-apple-darwin` | `e9b0c327121f9f75a8d06f856e0bc163208716c551a0b2991aea3e3f6e50fa17` |
| `x86_64-pc-windows-msvc` | `7aff6ef15085b1022db50430ab0a7d472ff633dc9aedefb2165ce2c7cc9ca0f6` |
| `x86_64-unknown-linux-gnu` | `3b66e4f24ee0899388316f72d9d7ccd4a3d96b349e771033ee59e9b3bb291689` |

All four adjacent checksums passed, and each archive README matched the
checked-in Git blob byte-for-byte. The public asset inventory contained
exactly those eight files with matching service-side archive digests. An
anonymous macOS arm64 download passed its adjacent checksum, matched the
tag-workflow artifact byte-for-byte, reported `skillroster 1.8.38`, and passed
the full Scan, Setup, Apply, Undo, and Status governance smoke in isolated
temporary directories.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #12](https://github.com/tt-a1i/homebrew-skillroster/pull/12)
at exact head `28118b14738ad2a93cfa9820fe69276cb5df6d6e`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33297952661)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33298180364)
was dispatched from tap `main` at
`b07a7382107bbffa0f949cf6f7ce321d5277e0fc` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`539b2bc8cb89afbf7ce742e4c8bf5b4eac5be6f3`, published both bottles, and
advanced tap `main` through bottle
commit `5b8a44d0e474938993ffc6cca9082c223ebdc504`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `e43437b327bca03c8de84b29919606abada84ba572b5982b02d33ac40ef0037b` |
| `x86_64_linux` | `64c22cdb7a4f96675b65347895a6030eb15ad5eccb69e38c6bc096a8d9c3f8d0` |

The public arm64 bottle was downloaded without repository credentials, matched
the Formula checksum, reported version 1.8.38, and passed the governance
smoke. Verification extracted the bottle in an isolated temporary directory
and did not mutate the user's installed Homebrew package.

## Boundaries

- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
