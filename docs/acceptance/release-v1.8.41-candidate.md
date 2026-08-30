# SkillRoster v1.8.41 release receipt

## Outcome

SkillRoster 1.8.41 prevents an unhinted CJK task from loading complete Skill
instructions from a weak incidental English Top-1 match.

- Ordinary `find` remains read-only and continues to return bounded lexical
  candidates with the existing cross-language hint warning.
- `find --load` now returns the typed
  `cjk_hint_required_for_weak_match` blocker when a CJK task has no
  Agent-authored English hint and Top-1 lacks strong direct metadata or
  correlated CJK evidence.
- A faithful English `--hint`, native CJK routing metadata, and an explicit
  Skill name such as `archify` remain loadable.
- The blocker returns no partial instructions and identifies the Agent-owned
  retry action without implying task success.

The real-inventory dogfood used the anonymous public macOS arm64 v1.8.40
binary, one fresh read-only Snapshot of 263 Skills and 1,037 placements, and
isolated temporary state. Published v1.8.40 reproducibly loaded
`request-refactor-plan` for `审查这个 Pull Request` in three of three runs while
also warning that an English hint might be required. The same result held in a
minimal two-Skill fixture, ruling out usage history and duplicate placements.
The fixed source keeps ordinary Find unchanged, blocks the weak unhinted load,
loads `github-code-review` with a faithful English hint, and preserves explicit
mixed-language selection.

The release also includes the bilingual README correction from
[#349](https://github.com/tt-a1i/skillroster/pull/349): the Chinese quick-start
example now routes to the documented `github-code-review` result instead of
showing a weaker public v1.8.40 candidate as if it were successful.

This patch changes bounded lexical Find loading only. SQLite remains at schema
12, the JSON envelope remains at schema 1, and bundled Bootstrap content remains
at version 1.8.29. It does not mutate Agent or Skill files.

## Source and review chain

[#351](https://github.com/tt-a1i/skillroster/pull/351) added the typed load
abstention at exact head `8748fc891ab9734286f09a98c9fbe82f56e6abfe`
and merged as `d8a871a2c9bf2cec46cd07867df27f9a0f767cdf`. The linked
[issue #350](https://github.com/tt-a1i/skillroster/issues/350) records the
public reproducer, ranked hypotheses, minimal fixture, and bounded acceptance
criteria. The exact-head PR
[CI run 33307618391](https://github.com/tt-a1i/skillroster/actions/runs/33307618391)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The exact-main
[CI run 33307849310](https://github.com/tt-a1i/skillroster/actions/runs/33307849310)
also passed the same four-platform matrix and aggregate gate at candidate base
revision `d8a871a2c9bf2cec46cd07867df27f9a0f767cdf`.

[#352](https://github.com/tt-a1i/skillroster/pull/352) prepared version 1.8.41
at exact head `7eedb6b84b922bbcb2dfab38171e0bcc32469bfd` and merged as exact
release source revision `97784d4c1faf4253e802e273b8404603aa108263`.
The PR [CI run 33308246502](https://github.com/tt-a1i/skillroster/actions/runs/33308246502)
and exact-main [CI run 33308436653](https://github.com/tt-a1i/skillroster/actions/runs/33308436653)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at their exact revisions.

The test-before-fix regression failed because the weak unhinted `--load`
command succeeded and returned unrelated complete instructions. At the final
fix head, the full local gate passed 327 Rust unit tests, 8 acceptance tests,
116 CLI tests, and 152 Node harness tests, plus strict Clippy, formatting,
installation-surface validation, archive README validation, the change-scope
self-test, and `git diff --check`.

## Published release

Annotated tag `v1.8.41` has tag object
`67d8c096de06b2fc0a35ac478b8452a5f6ce163e` and resolves to exact source
revision `97784d4c1faf4253e802e273b8404603aa108263`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33308649493)
passed the strict repository gate, all four supported-platform jobs, the
governance smoke, and WSL2 at that exact revision.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.41)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `9c2ee528b46a475fb14e8061f361d51e3b02858f1b26e6b734271faabdee78d0` |
| `x86_64-apple-darwin` | `7869cdf60d0b6734aff7fcd914d551e315d259145be6673c82bf3a6fca1a97af` |
| `x86_64-pc-windows-msvc` | `dad19fbb2524b5a62ceb22f47aace784a1c45e36dbe14ad1d0c538cfd21ac78f` |
| `x86_64-unknown-linux-gnu` | `26fdacfdbaad9a963dbb82eda94e7dadc454eb549a87b2ea3217ca9faf3595d6` |

All four adjacent checksums passed, and every archive README and LICENSE
matched the checked-in Git blobs byte-for-byte. The public asset inventory
contained exactly those eight files with matching service-side archive
digests. An anonymous macOS arm64 download passed its adjacent checksum,
matched the tag-workflow artifact byte-for-byte, reported `skillroster
1.8.41`, and passed the full release governance smoke in isolated temporary
directories.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #15](https://github.com/tt-a1i/homebrew-skillroster/pull/15)
at exact head `fd87af1ec67da0979ae75f8b517cb4eb19d5038e`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33309738470)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33310021051)
was dispatched from tap `main` at
`4e4be54f5b105ac2e1f6b1f5e3b1371ec84661b9` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`b774ae3812cc0f387c2caa4c5c9286532455fdef`, published both bottles, and
advanced tap `main` through bottle commit
`b19d9841db8a08dc420516079ffc3e7258e66476`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `cf0235c43f5a8c7701bbcc2f1cfe75da8b74702767445ef5a79eefb9b67a206c` |
| `x86_64_linux` | `493d57bb958928c6af4ff89dac40fd951fbde9c807c65d675b9fe28eedd2db73` |

Both public bottles were downloaded without repository credentials and
matched their Formula checksums. The arm64 bottle reported version 1.8.41 and
passed the release governance smoke. Verification extracted the bottles in
isolated temporary directories and did not mutate the user's installed
Homebrew package.

## Boundaries

- Weak unhinted cross-language loading remains a bounded lexical policy, not
  a general semantic translation or intent-resolution engine.
- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
