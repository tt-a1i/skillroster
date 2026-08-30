# SkillRoster v1.8.37 release receipt

## Outcome

SkillRoster 1.8.37 makes cross-language Skill routing follow a precise Agent
hint without discarding the user's original task.

- A complete match for a one-token Skill name such as `diagnose` or `pdf`
  counts as direct hint evidence. A weaker native-language candidate can no
  longer take Top-1 only because it matched the original task.
- A partial hit on a multi-token Skill name such as `review` in
  `github-code-review` does not receive that protection.
- Chinese negative routing clauses beginning with `不应触发` are treated as
  exclusions, including clauses separated by `。！？；`, rather than being
  scored as positive capability text.
- The original task and the Agent-authored English hint remain separate,
  visible evidence channels. Lower-ranked native-language candidates remain
  available in the bounded result when they still carry relevant evidence.

The real-inventory replay covered Chinese tasks for PR review, diagnosis,
codebase simplification, PDF layout inspection, and data-quality analysis. All
five returned the intended Top-1 Skill. Exact `find --load` checks for
`diagnose` and `pdf` returned verified complete content without changing
files.

This patch changes deterministic Find ranking and description polarity only.
SQLite remains at schema 12, the JSON envelope remains at schema 1, and bundled
Bootstrap content remains at version 1.8.29. It does not mutate Agent or Skill
files.

## Source and review chain

[#331](https://github.com/tt-a1i/skillroster/pull/331) fixed precise
cross-language routing at exact head
`3dfdb74f13e15e2bba945a5684017e4deebd857f` and merged as
`c6c4f5235d428fc664e5c47245b08ba5b8540002`. The linked
[issue #330](https://github.com/tt-a1i/skillroster/issues/330) records the
reproducible failure, ranked hypotheses, root cause, and bounded acceptance
criteria. Sequential Spec and Standards reviews passed after the implementation
was corrected to keep partial multi-token name hits from being treated as
direct name evidence.

[#332](https://github.com/tt-a1i/skillroster/pull/332) prepared version 1.8.37
at exact head `d1acecf6f550183c701df1161add6b3e410056c7` and merged as exact
release source revision `cbfebe6e3dbaa29aecca109bc8858779812f02a6`.
The PR [CI run 33289094357](https://github.com/tt-a1i/skillroster/actions/runs/33289094357)
and exact-main [CI run 33289252565](https://github.com/tt-a1i/skillroster/actions/runs/33289252565)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The final local release-preparation gate passed 321 Rust unit tests, 8
acceptance tests, 105 CLI tests, and 152 Node harness tests, plus strict
Clippy, formatting, installation-surface validation, archive README validation,
the change-scope self-test, and `git diff --check`. Sequential Release
Spec/Evidence and Standards/Compatibility reviews were Clean.

## Candidate acceptance

The exact-main candidate
[run 33289450524](https://github.com/tt-a1i/skillroster/actions/runs/33289450524)
passed the strict repository gate, four platform build and governance jobs,
and the WSL2 governance smoke at exact revision
`cbfebe6e3dbaa29aecca109bc8858779812f02a6`.

All four downloaded candidate checksums passed. Each archive README matched
the checked-in Git blob byte-for-byte. The macOS arm64 candidate reported
`skillroster 1.8.37` and passed the release-governance smoke.

## Published release

Annotated tag `v1.8.37` has tag object
`aea2732e30378599b2f441cf4e560b5714f0be46` and resolves to exact source
revision `cbfebe6e3dbaa29aecca109bc8858779812f02a6`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33290122154)
repeated the strict gate, all four supported-platform jobs, and WSL2
successfully.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.37)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `36b49fe4e5572c91603a3969377ff431f3f9466a03fd4938156ada4632f6abea` |
| `x86_64-apple-darwin` | `7498ebd8a7db734e5c5af1a7caa767158e81ca12575e0ace41ef5d9a4e33df28` |
| `x86_64-pc-windows-msvc` | `06d9dc8a286bf13811aa0c52232b3a2950184f418eeb6c59b38cee029dda8c7f` |
| `x86_64-unknown-linux-gnu` | `ff794f9b9fb1858c37302514185f70322b835afe4f73ff62792e5cea63da76a9` |

The public release asset inventory was read back and contained exactly those
eight uploaded files with matching service-side archive digests. An anonymous
macOS arm64 download passed its adjacent checksum, matched the tag-workflow
artifact byte-for-byte, reported `skillroster 1.8.37`, and passed the
release-governance smoke.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #11](https://github.com/tt-a1i/homebrew-skillroster/pull/11)
at exact head `701c14da9b9b292c87efb4b59738c032035f1632`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33291109014)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33291374890)
was dispatched from tap `main` at
`2e56d1d1122ddf2e5eb26a8222e4513b0656912e` after test-bot passed. It
closed the PR without a regular merge commit, created equivalent Formula
commit `42d9779d93e4c75825a3b88c80def2850aad4d74`, published both bottles,
and advanced tap `main` through bottle commit
`b07a7382107bbffa0f949cf6f7ce321d5277e0fc`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `db495039f5140b67221ec2ee8d6b96f156812820b6563ce5e6b64b6036277ed7` |
| `x86_64_linux` | `b568b793c1a73856e52b804de2f675e2cc72498a348390b018b0d123787a9a32` |

The public arm64 bottle was downloaded without repository credentials, matched
the Formula checksum, reported version 1.8.37, and passed the
release-governance smoke. Verification extracted the bottle in an isolated
temporary directory and did not mutate the user's installed Homebrew package.

## Boundaries

- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent files, or change
  the Bootstrap content version.
