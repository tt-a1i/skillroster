# SkillRoster v1.8.42 release receipt

## Outcome

SkillRoster 1.8.42 prevents broad native-CJK lexical overlap from authorizing
the complete instructions of an unrelated Skill.

- Ordinary `find` remains read-only and keeps the same bounded candidates,
  ranking scores, match reasons, and cross-language hint warning.
- Unhinted CJK `find --load` now requires direct selection evidence: an exact
  declared name or trigger, complete Skill-name token coverage, or an exact
  description phrase with at least two matching tokens.
- Broad description-token or CJK bigram overlap can still preserve a useful
  lexical candidate, but cannot return complete instructions.
- Weak loads fail closed with the typed
  `cjk_hint_required_for_weak_match` blocker, no partial instructions, and
  `files_changed=false`; a faithful Agent-authored English hint remains
  loadable.

The real-inventory dogfood used the anonymous public macOS arm64 v1.8.41
binary, a fresh read-only Snapshot of 263 Skills and 1,037 placements, and
isolated temporary state. For a Chinese product-dogfood task combining problem
discovery, root-cause analysis, repair, and retrospective, published v1.8.41
ranked an unrelated publishing Skill first and returned its complete 56 KiB
instructions even while warning that an English hint was needed. Three of
three runs in a fresh two-Skill fixture reproduced the same policy failure
without usage history or duplicate placements.

The fixed source preserves the ordinary Find projection byte-for-byte in both
the real inventory and minimal fixture. The same unhinted loads now return the
typed blocker with no result, while a faithful English diagnostic hint loads
`diagnose` completely.

This patch changes bounded lexical Find loading only. SQLite remains at schema
12, the JSON envelope remains at schema 1, and bundled Bootstrap content remains
at version 1.8.29. It does not mutate Agent or Skill files.

## Source and review chain

Release preparation started from exact `origin/main` revision
`dbf673b636a557de4b08e70d1d6e28d03ab3d841`.

[#355](https://github.com/tt-a1i/skillroster/pull/355) separated rank-preservation
evidence from complete-load authorization at exact head
`65f3c1fbaabc7e62636ad71a78c234bcf3b8572c` and merged as the candidate base.
The linked [issue #354](https://github.com/tt-a1i/skillroster/issues/354)
records the public reproducer, ranked hypotheses, deterministic fixture, and
bounded acceptance criteria. The exact-head PR
[CI run 33313718785](https://github.com/tt-a1i/skillroster/actions/runs/33313718785)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The exact-main
[CI run 33314035356](https://github.com/tt-a1i/skillroster/actions/runs/33314035356)
also passed the same four-platform matrix and aggregate gate at candidate base
revision `dbf673b636a557de4b08e70d1d6e28d03ab3d841`.

The test-before-fix regression failed because the broad unhinted CJK
`find --load` command succeeded and returned unrelated complete instructions.
At the final fix head, the full local gate passed 327 Rust unit tests, 8
acceptance tests, 117 CLI tests, and 152 Node harness tests, plus strict Clippy,
formatting, installation-surface validation, archive README validation, the
change-scope self-test, and `git diff --check`.

[#356](https://github.com/tt-a1i/skillroster/pull/356) prepared version 1.8.42
at exact head `0fa88bcac559d100bdcfadb167439c5233649fff` and merged as exact
release source revision `b365824bb49ec55cb77501368d913e77f958ee8c`.
The PR [CI run 33314498797](https://github.com/tt-a1i/skillroster/actions/runs/33314498797)
and exact-main [CI run 33314804782](https://github.com/tt-a1i/skillroster/actions/runs/33314804782)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at their exact revisions.

## Published release

Annotated tag `v1.8.42` has tag object
`5d422a8aebd0d34ef2f7ab08c89db66628b59696` and resolves to exact source
revision `b365824bb49ec55cb77501368d913e77f958ee8c`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33315010094)
passed the strict repository gate, all four supported-platform jobs, each
governance smoke, and WSL2 at that exact revision.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.42)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `d1f40d6add081edbb7403dbe04ede75abff7d7fca8fcb84235cde6b9662926c8` |
| `x86_64-apple-darwin` | `37cf9efafa19268832687c3b122576a280831a1451f464d4ede481ab1bd29b16` |
| `x86_64-pc-windows-msvc` | `c767a52d413c0975dae444f353064863490d58b6be42be0526e7fc9eecf7f117` |
| `x86_64-unknown-linux-gnu` | `ae308cbc0747aa38e7cc6e2b8ac3e34039b3bfb8634cce4aafdbff1c6dcd264b` |

All adjacent checksums passed, and every archive README and LICENSE matched
the checked-in Git blobs byte-for-byte. The public asset inventory contained
exactly those eight files with matching service-side archive digests. An
anonymous macOS arm64 download passed its adjacent checksum, matched the tag
workflow artifact byte-for-byte, reported `skillroster 1.8.42`, and passed the
full release governance smoke in isolated temporary directories. The released
binary also replayed the two-Skill regression: the broad unhinted CJK load
returned the typed blocker with no result, while a faithful diagnostic hint
loaded `diagnose` completely.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #16](https://github.com/tt-a1i/homebrew-skillroster/pull/16)
at exact head `4e3c68ab0f99aa53a0d54e6647706290e60490e0`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33316203246)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33316532993)
was dispatched from tap `main` at
`b19d9841db8a08dc420516079ffc3e7258e66476` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`d6c2fbfb6010ffe2d0c3ee8562c8ee24ce6b8868`, published both bottles, and
advanced tap `main` through bottle commit
`6019feffa842324cdf0687b95b7ed9b16c5977ed`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `e7400b165a7d63ee2f8c63ed71709acf32bf85696f316a9a3f85b9b19cc8044e` |
| `x86_64_linux` | `78c1e00bf22453dca157a3eb2b0f9a3718d378e8cd8f2132ad0643e060fee3b8` |

Both public bottles were downloaded without repository credentials and
matched their Formula checksums. The arm64 bottle reported version 1.8.42 and
passed the release governance smoke. Verification extracted the bottles in
isolated temporary directories and did not mutate the user's installed
Homebrew package.

## Boundaries

- The stricter load boundary applies to unhinted CJK lexical routing; it is not
  a general semantic translation or intent-resolution engine.
- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
