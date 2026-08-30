# SkillRoster v1.8.39 release receipt

## Outcome

SkillRoster 1.8.39 keeps explicit task exclusions from accidentally removing
the capability the user actually requested.

- A task such as “review this code, but do not simplify or refactor it” keeps
  `code-review` at Top-1 while excluding modification-oriented Skills such as
  `simplify-codebase`.
- English `code` and Chinese `代码` are treated as shared task-object context
  only when the same negative clause contains another searchable constraint
  and they are not its leading token. Sole or leading capability exclusions
  remain effective.
- Exclusions are derived independently per clause, so `do not code; do not
  publish` still excludes both capabilities rather than weakening either one.
- JSON exclusion evidence remains bounded and auditable, and Agent hints still
  cannot override an actual prohibited capability.

The real-inventory A/B replay reused one read-only Snapshot of 263 Skills and
1,037 placements. Before the fix, adding “do not simplify or refactor the code”
removed `code-review` and promoted `impact-gated-pr`. After the fix,
`code-review` remained Top-1, `simplify-codebase` remained excluded, and both
the English and Chinese replays reported `files_changed=false`.

This patch changes lexical Find exclusion handling only. SQLite remains at
schema 12, the JSON envelope remains at schema 1, and bundled Bootstrap content
remains at version 1.8.29. It does not mutate Agent or Skill files.

## Source and review chain

[#341](https://github.com/tt-a1i/skillroster/pull/341) fixed shared task-object
exclusions at exact head `817fc48eabac53ba58d8716985baf61d39b29985`
and merged as `0cb27d9982a526fef8f83cc61fd15d4e6670f674`. The linked
[issue #340](https://github.com/tt-a1i/skillroster/issues/340) records the
real bilingual reproducer, root cause, and bounded acceptance criteria.
Sequential Spec/Evidence and Standards/Compatibility reviews were Clean.

[#342](https://github.com/tt-a1i/skillroster/pull/342) prepared version 1.8.39
at exact head `35547897fad2b9a64d35ed80fbe74894602302c7` and merged as exact
release source revision `efa98c05ec042d17dd9a7f1310daee6fc43da8e8`.
Spec/Evidence review found one documentation sentence that was narrower than
the implemented clause rule; the wording was corrected, all gates were rerun,
and both Spec/Evidence and Standards/Compatibility re-reviews passed at the
final head.

The PR [CI run 33300490903](https://github.com/tt-a1i/skillroster/actions/runs/33300490903)
and exact-main [CI run 33300681762](https://github.com/tt-a1i/skillroster/actions/runs/33300681762)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate. The final local gate passed 325 Rust unit tests, 8
acceptance tests, 114 CLI tests, and 152 Node harness tests, plus strict
Clippy, formatting, installation-surface validation, archive README
validation, the change-scope self-test, and `git diff --check`.

## Published release

Annotated tag `v1.8.39` has tag object
`8604dc31f4bdc03295af69614d504963dc339ba0` and resolves to exact source
revision `efa98c05ec042d17dd9a7f1310daee6fc43da8e8`. The tag
[release workflow](https://github.com/tt-a1i/skillroster/actions/runs/33300975709)
passed the strict repository gate, all four supported-platform jobs, the
governance smoke, and WSL2 at that exact revision.

The public [GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.39)
contains four archives and four adjacent checksum files:

| Target | SHA-256 |
| --- | --- |
| `aarch64-apple-darwin` | `d68c707fc07725a2008b3e50b3ed09b68bbab7c8b35f135ef868ba7a52343ff2` |
| `x86_64-apple-darwin` | `3a66ec33aed7cada5f514f630674afc3d8b9821e2426e7ff59c3a73fb535ba56` |
| `x86_64-pc-windows-msvc` | `f7259519a653922c3fd9f3dd8061c99570a2310d80623c54a0a494ffeeab56f5` |
| `x86_64-unknown-linux-gnu` | `0728c17e9122b7797474a176205ec42ccf0db30c876ee6f1279020519cbe7126` |

All four adjacent checksums passed, and each archive README and LICENSE
matched the checked-in Git blobs byte-for-byte. The public asset inventory
contained exactly those eight files with matching service-side archive
digests. An anonymous macOS arm64 download passed its adjacent checksum,
matched the tag-workflow artifact byte-for-byte, reported `skillroster
1.8.39`, and passed the full Scan, Setup, Apply, Undo, and Status governance
smoke in isolated temporary directories.

## Homebrew

The public [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster)
updated through [PR #13](https://github.com/tt-a1i/homebrew-skillroster/pull/13)
at exact head `71068e596f506246567a01c81841aca28e85fb4d`. The
[brew test-bot run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33301932343)
built and tested macOS arm64 and Linux x86_64 bottles at that PR head. The
[brew pr-pull run](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33302210671)
was dispatched from tap `main` at
`5b8a44d0e474938993ffc6cca9082c223ebdc504` after test-bot passed. It closed
the PR without a regular merge commit, created equivalent Formula commit
`4f95853502739ff5b5efd8892febf6dd05b7a7c1`, published both bottles, and
advanced tap `main` through bottle commit
`4df9a5e26aeced5a598364463dd4a2a5bb1a5843`.

| Bottle | SHA-256 |
| --- | --- |
| `arm64_tahoe` | `1512732b61d49050fe1c8b29b95b9cfae36c4c8a2a04ec709bfd20f8e9ec167a` |
| `x86_64_linux` | `3f3e2bd606a7464c5f7cc271df58e96c331077c267fdfec840ff62ddc88b0a2d` |

The public arm64 bottle was downloaded without repository credentials,
matched the Formula checksum, reported version 1.8.39, and passed the
governance smoke. Verification extracted the bottle in an isolated temporary
directory and did not mutate the user's installed Homebrew package.

## Boundaries

- The lexical `code` / `代码` rule is deliberately bounded; it is not a
  general semantic-negation engine.
- WSL2 is verified with the Linux archive. WSL1 remains fail-closed for Apply
  and Undo because it lacks the required atomic no-replace rename primitive.
- The release does not claim native Linux arm64 or Windows arm64 artifacts.
- Publishing this release did not migrate state, modify Agent or Skill files,
  or change the Bootstrap content version.
