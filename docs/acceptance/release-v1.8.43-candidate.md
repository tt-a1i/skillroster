# SkillRoster v1.8.43 release receipt

## Outcome

SkillRoster 1.8.43 makes Agent-authored retrieval hints both more reliable and
safer at the complete-instruction boundary.

- A hint that completely names a multi-token Skill can now outrank broad native
  task overlap, matching the existing direct-evidence rule for single-token
  names. Partial multi-token name matches remain weak.
- Ordinary `find` remains read-only and keeps weak lexical candidates visible
  for Agent judgment, with explicit direct-selection match evidence.
- `find --load` now requires direct selection evidence even when the Agent
  supplies a hint.
- Direct evidence is an exact declared name or trigger, complete Skill-name
  token coverage, or complete normalized coverage of one task or hint phrase
  by the positive Skill description, with at least two tokens.
- Partial Skill-name matches and unrelated description-token overlap cannot
  authorize a complete load.
- A weak hinted load fails closed with the typed
  `hint_direct_selection_evidence_required` reason, no partial instructions,
  and `files_changed=false`; the Agent can retry with a more specific hint.

The real-inventory dogfood used fresh read-only Snapshots of 263 Skills and
1,037 placements with isolated temporary state. First, a faithful hint naming
`simplify-codebase` lost Top-1 to broad native overlap in three of three runs;
the ranking fix moved `simplify-codebase` to Top-1 in three of three runs while
preserving the same-name variant boundary and exact verified variant load.

Second, for the task “把这组产品决定整理成可执行规格” and the hint `convert
product decisions into an executable specification`, the previous behavior
loaded `product-business-analysis` instead of the intended `to-spec`. The
wrong Top-1 had only partial name and description-token overlap.

The #361 fix preserves the ordinary Find ranking and order while adding a
direct-selection match reason. The same weak load now returns the typed blocker
with no result. A faithful direct capability hint ranks `to-spec` first; the
real-inventory `find --load` replay then correctly stops at its existing
same-name variant ambiguity, while exact verified variant loads remain
available.

This patch changes hinted lexical ranking, direct-selection evidence, and
verified-load authorization. SQLite remains at schema 12, the JSON envelope
remains at schema 1, and bundled Bootstrap content remains at version 1.8.29.
It does not mutate Agent or Skill files.

## Source and review chain

Release preparation started from exact `origin/main` revision
`5996148473a09d39b9956e7832f091894fdff55d`.

[#359](https://github.com/tt-a1i/skillroster/pull/359) aligned complete
multi-token Skill-name hints with the existing direct-evidence rule at exact
head `d11c05df35128aad06c28f1479b548a2bf2dc433` and merged as revision
`9964b1f90a7d76de5f88d5817b297255de3b2a3a`. The linked
[issue #358](https://github.com/tt-a1i/skillroster/issues/358) records the
real-inventory ranking failure and bounded acceptance criteria. Both
independent exact-head reviews passed. The PR
[CI run 33318742373](https://github.com/tt-a1i/skillroster/actions/runs/33318742373)
and exact-main
[CI run 33319047172](https://github.com/tt-a1i/skillroster/actions/runs/33319047172)
passed the four-platform matrix and aggregate gate at their exact revisions.

[#361](https://github.com/tt-a1i/skillroster/pull/361) added the hinted-load
authorization boundary at exact head
`e320f802260dc19eb1b239f61b115600399b7b03`. The linked
[issue #360](https://github.com/tt-a1i/skillroster/issues/360) records the
public reproducer, root cause, deterministic fixture, and bounded acceptance
criteria. Two independent exact-head reviews passed with no blocking findings.
The PR
[CI run 33320656155](https://github.com/tt-a1i/skillroster/actions/runs/33320656155)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The exact-main
[CI run 33320872443](https://github.com/tt-a1i/skillroster/actions/runs/33320872443)
passed the same four-platform matrix and aggregate gate at candidate base
revision `5996148473a09d39b9956e7832f091894fdff55d`.

The test-before-fix regression failed because the weak hinted `find --load`
command succeeded and returned unrelated complete instructions. At the final
fix head, the full local gate passed 327 Rust unit tests, 8 acceptance tests,
119 CLI tests, and 152 Node harness tests, plus strict Clippy, formatting,
installation-surface validation, archive README validation, the change-scope
self-test, and `git diff --check`.

[#362](https://github.com/tt-a1i/skillroster/pull/362) prepared version 1.8.43
at exact head `59be020d07dfc0ec8c730864783154887746ddcc` and merged as exact
release source revision `76a6885669f96ba37c919641f4b207e99b2b27fe`.
The PR
[CI run 33321599897](https://github.com/tt-a1i/skillroster/actions/runs/33321599897)
and exact-main
[CI run 33321885194](https://github.com/tt-a1i/skillroster/actions/runs/33321885194)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at their exact revisions. Two independent exact-head
reviews passed with no blocking findings. The complete local gate passed twice
on the candidate bytes: 327 Rust unit tests, 8 acceptance tests, 119 CLI tests,
and 152 Node harness tests, plus strict Clippy, formatting, installation-surface
validation, archive README validation, the change-scope self-test, and
`git diff --check`.

## Published release

Annotated tag `v1.8.43` has tag object
`7eb064beaba68061ad8cf3258ecbc5f737df54fe` and resolves to exact source
revision `76a6885669f96ba37c919641f4b207e99b2b27fe`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33322098095)
passed the strict repository gate, all four supported-platform jobs, each
governance smoke, and WSL2 at that exact revision.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.43)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `0def10401e283cce13332eec4af16d135b5963c8352da54d07f44768358edbca` |
| `x86_64-apple-darwin` | `ffc54fecb49a9e540879f7a2fb9ca278d7d5bb387c60b6f6bfe741873664b5df` |
| `x86_64-pc-windows-msvc` | `941f0b9f697fa4bbc135b8b536528844fd566c74ccbfa66078a976ebff96bb04` |
| `x86_64-unknown-linux-gnu` | `a661bc5952305f884f1f22a0cd78daa9c20b2b9e53bcd36252e0b2f8e9171cae` |

All adjacent checksums passed. Every archive contained only its versioned
directory, binary, README, and LICENSE; each README and LICENSE matched the
checked-in Git blob byte-for-byte. The public asset inventory contained exactly
those eight files. An anonymous macOS arm64 download matched its adjacent
checksum and the tag-workflow artifact byte-for-byte, reported
`skillroster 1.8.43`, and passed the release governance smoke in isolated
temporary directories.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #17](https://github.com/tt-a1i/homebrew-skillroster/pull/17)
at exact head `335cd7956b7575bbf377f7f0c3d6ddeda4a74fbc`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33323499540)
built, installed, and tested macOS arm64 and Linux x86_64 bottles at that PR
head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33323773777)
was dispatched from tap `main` at
`6019feffa842324cdf0687b95b7ed9b16c5977ed` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`c4f679d1a9bd950d12b341c02d64950a60527605`, published both bottles, and
advanced tap `main` through bottle commit
`f141e46f3f492fae25fdc1d69f2b5d6527448df5`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `40527cc32a3604f4795ff3a9241cf0ff0e1d44187b4ea5ce879127e8f5686dad` |
| `x86_64_linux` | `bb4576bf7ead8842750994ed9d8d8107ea3ddebb8da63604eb2db70f294f2eca` |

The public Formula checksums match the release-asset digests. An anonymous
arm64 bottle download matched its Formula checksum, reported version 1.8.43,
and completed an isolated `scan --summary --json` smoke with `ok=true` and
`files_changed=false`. Verification extracted the bottle in a temporary
directory and did not mutate the user's installed Homebrew package.

## Boundaries

- Direct-selection evidence is a lexical authorization boundary, not a general
  semantic translation or intent-resolution engine.
- Same-name Skill variants remain ambiguous until the Agent supplies an exact
  verified variant; the ranking fix does not silently choose between them.
- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
