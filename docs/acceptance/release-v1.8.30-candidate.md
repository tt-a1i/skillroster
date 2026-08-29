# SkillRoster v1.8.30 release receipt

## Release notes

SkillRoster 1.8.30 keeps large Apply and Undo responses bounded for Agent
callers without weakening recovery.

- Ordinary mutation JSON now returns at most ten deterministically ordered
  changed-path previews.
- `changed_path_count` still reports the exact total, while the additive
  `changed_paths_truncated` field states whether the preview is incomplete.
- The persisted Receipt journal still contains the complete changed-path set,
  and Undo continues to use that complete recovery record.

This patch release keeps SQLite schema 12 and JSON envelope schema 1. Strict
JSON consumers must tolerate the new additive field. There is no migration and
no change to the local-only, explicit-confirmation, fail-closed mutation model.
The bundled Bootstrap instructions are unchanged at content version 1.8.29, so
upgrading the CLI does not cause an unnecessary Bootstrap replacement Plan.

## Release evidence

The public baseline was v1.8.29. Candidate preparation started from exact
`origin/main` revision `3b471620445285c9f97eacbdeca211e1bee07c7c`, after
[#300](https://github.com/tt-a1i/skillroster/pull/300) merged with its Linux,
Windows, macOS arm64, macOS x86_64, change-scope, and aggregate CI gates green.
That change closed [#299](https://github.com/tt-a1i/skillroster/issues/299).

The annotated `v1.8.30` tag resolves exactly to
`2891c383e3beca999788b0c54b044eb4f91f52a5`. The official tag workflow
[run 33256760259](https://github.com/tt-a1i/skillroster/actions/runs/33256760259)
passed the strict repository gate, Linux, Windows, macOS arm64 and x86_64
build/governance jobs, and the WSL2 Linux governance smoke.

The [public GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.30)
contains four platform archives and four adjacent checksum files. All eight
workflow artifacts passed their adjacent checksums before publication. The
public macOS arm64 archive was then downloaded through its anonymous URL,
matched its published checksum, reported `skillroster 1.8.30`, and passed the
complete synthetic release governance smoke.

The [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster) publishes
v1.8.30 macOS arm64 and x86_64 Linux bottles. Both Homebrew test-bot jobs passed
in [run 33257348384](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33257348384),
and exact-head `brew pr-pull` published the bottles in
[run 33257632463](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33257632463).
The public macOS arm64 bottle was downloaded anonymously, matched the Formula
checksum, reported `skillroster 1.8.30`, and passed the same governance smoke.

## Accepted gates

The exact tagged revision passed:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and both Node routing harnesses;
- `git diff --check`, installation-surface validation, and the CI change-scope
  self-test;
- four platform build and governance jobs plus the WSL2 governance smoke.

The release WSL boundary remains WSL2. WSL1 mutation fails closed when its
kernel cannot provide the atomic no-replace rename required by SkillRoster's
handle-bound recovery model.
