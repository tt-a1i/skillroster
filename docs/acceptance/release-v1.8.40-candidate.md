# SkillRoster v1.8.40 release receipt

## Outcome

SkillRoster 1.8.40 recognizes explicit capability exclusions when ordinary
English or Chinese coordinators introduce the negative clause.

- Tasks such as “review this code, but do not simplify or refactor it” and
  “审查代码，但是不要简化或重构代码” now keep the requested review capability
  while excluding modification-oriented Skills.
- The deterministic parser accepts only six bounded prefixes immediately before
  an existing exclusion marker: `but`, `and`, `yet`, `但是`, `不过`, and `但`.
- `task_exclusions` preserves the person's original coordinated clause for
  auditability. The coordinator and marker are removed only when comparing the
  prohibited capability tokens.
- A coordinator without an immediate exclusion marker remains positive task
  text, and negative-state phrases such as “tests do not pass” remain unchanged.

The real-inventory dogfood used the anonymous public macOS arm64 v1.8.39 binary,
one fresh read-only Snapshot of 263 Skills and 1,037 placements, and isolated
temporary state. Published v1.8.39 returned no exclusion for coordinated
English or Chinese clauses and left a simplify Skill in the candidates. The
fixed source reports the original coordinated clause, removes the prohibited
simplify candidates, and keeps `files_changed=false` in both languages.

This patch changes bounded lexical Find routing only. SQLite remains at schema
12, the JSON envelope remains at schema 1, and bundled Bootstrap content remains
at version 1.8.29. It does not mutate Agent or Skill files.

## Source and review chain

[#345](https://github.com/tt-a1i/skillroster/pull/345) fixed coordinated task
exclusions at exact head `b8287753df7fccbbafd5f408b7decba5cdecef8b`
and merged as `3a03538920aa642f450fe8adc74a758f4a52ad3c`. The linked
[issue #344](https://github.com/tt-a1i/skillroster/issues/344) records the real
bilingual reproducer, one-variable differential, and bounded acceptance
criteria. The exact-head PR
[CI run 33303452975](https://github.com/tt-a1i/skillroster/actions/runs/33303452975)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The exact-main
[CI run 33303660610](https://github.com/tt-a1i/skillroster/actions/runs/33303660610)
also passed the same four-platform matrix and aggregate gate at candidate base
revision `3a03538920aa642f450fe8adc74a758f4a52ad3c`.

[#346](https://github.com/tt-a1i/skillroster/pull/346) prepared version 1.8.40
at exact head `402aeba629f7a3d60d21b1baac4563b2815c119c` and merged as exact
release source revision `7df4c68e5e37444a7a701c735e19f2bb3e4b39ec`.
The PR [CI run 33303888383](https://github.com/tt-a1i/skillroster/actions/runs/33303888383)
and exact-main [CI run 33304146689](https://github.com/tt-a1i/skillroster/actions/runs/33304146689)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at their exact revisions.

The test-before-fix public CLI regression failed with an empty English
`task_exclusions` array. At the final fix head, the full local gate passed 326
Rust unit tests, 8 acceptance tests, 114 CLI tests, and 152 Node harness tests,
plus strict Clippy, formatting, installation-surface validation, archive README
validation, the change-scope self-test, and `git diff --check`.

## Published release

Annotated tag `v1.8.40` has tag object
`e9a619a08b0807931d45ea42460c3202056dd09f` and resolves to exact source
revision `7df4c68e5e37444a7a701c735e19f2bb3e4b39ec`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33304469207)
passed the strict repository gate, all four supported-platform jobs, the
governance smoke, and WSL2 at that exact revision.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.40)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `bfbf0c259d41f7d8817f6f6171c3643b745d76980cdbb5a980928632c5089bfb` |
| `x86_64-apple-darwin` | `bed295976203cc2ab01dd527300940dbccb44e1dc6d7015df7d1e9f8b011d457` |
| `x86_64-pc-windows-msvc` | `4abf4968910170068107ff1e3d93ec34de245e6e839d21fc0002f0dee1bef92f` |
| `x86_64-unknown-linux-gnu` | `39726cdfdc40d497811a8798f7b12f8f3cec6a1fee51d1bd5c58c644878530a3` |

All four adjacent checksums passed, and every archive README and LICENSE
matched the checked-in Git blobs byte-for-byte. The public asset inventory
contained exactly those eight files with matching service-side archive
digests. An anonymous macOS arm64 download passed its adjacent checksum,
matched the tag-workflow artifact byte-for-byte, reported `skillroster
1.8.40`, and passed the full release governance smoke in isolated temporary
directories.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #14](https://github.com/tt-a1i/homebrew-skillroster/pull/14)
at exact head `f0130f1a6e31bc5567ec9417747ee5abd69c48ae`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33305221735)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33305513508)
was dispatched from tap `main` at
`4df9a5e26aeced5a598364463dd4a2a5bb1a5843` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`232cee41d4f2a17121e398260ba4e59b9e7f76a1`, published both bottles, and
advanced tap `main` through bottle commit
`4e4be54f5b105ac2e1f6b1f5e3b1371ec84661b9`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `d7134699850bbf129c7e98c54e2960ed1a660b3bc15a6fa3075745bc0e4a63dd` |
| `x86_64_linux` | `8e291267270b39d4c21464f5723acd0eed5a9016cc44352f408f141be261ede1` |

The public arm64 bottle was downloaded without repository credentials,
matched the Formula checksum, reported version 1.8.40, and passed the release
governance smoke. Verification extracted the bottle in an isolated temporary
directory and did not mutate the user's installed Homebrew package.

## Boundaries

- Coordinated task exclusion remains a bounded lexical rule, not a general
  semantic-negation or pronoun-resolution engine.
- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
