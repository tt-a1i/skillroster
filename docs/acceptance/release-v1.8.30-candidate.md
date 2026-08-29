# SkillRoster v1.8.30 candidate

## Release notes draft

SkillRoster 1.8.30 keeps large Apply and Undo responses bounded for Agent
callers without weakening recovery.

- Ordinary mutation JSON now returns at most ten deterministically ordered
  changed-path previews.
- `changed_path_count` still reports the exact total, while the additive
  `changed_paths_truncated` field states whether the preview is incomplete.
- The persisted Receipt journal still contains the complete changed-path set,
  and Undo continues to use that complete recovery record.

This patch release keeps SQLite schema 10 and JSON envelope schema 1. Strict
JSON consumers must tolerate the new additive field. There is no migration and
no change to the local-only, explicit-confirmation, fail-closed mutation model.
The bundled Bootstrap instructions are unchanged at content version 1.8.29, so
upgrading the CLI does not cause an unnecessary Bootstrap replacement Plan.

## Preparation evidence

The public baseline is v1.8.29. Candidate preparation starts from exact
`origin/main` revision `3b471620445285c9f97eacbdeca211e1bee07c7c`, after
[#300](https://github.com/tt-a1i/skillroster/pull/300) merged with its Linux,
Windows, macOS arm64, macOS x86_64, change-scope, and aggregate CI gates green.
That change closed [#299](https://github.com/tt-a1i/skillroster/issues/299).

The source candidate and Cargo package are versioned as 1.8.30. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.29 until the v1.8.30 tag,
artifacts, checksums, and Homebrew package actually exist.

## Candidate gates

The candidate is not accepted until one exact final source revision passes:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and both Node routing harnesses;
- `git diff --check`, installation-surface validation, and the CI change-scope
  self-test;
- four platform build and governance jobs plus the WSL2 governance smoke.

Candidate preparation does not create or push a tag, publish a GitHub Release,
update Homebrew, or mutate any public release asset. Those steps are recorded
only after their external evidence exists.
