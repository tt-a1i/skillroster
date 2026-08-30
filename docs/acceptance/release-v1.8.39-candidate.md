# SkillRoster v1.8.39 candidate

## Release notes draft

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

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`0cb27d9982a526fef8f83cc61fd15d4e6670f674`.

[#341](https://github.com/tt-a1i/skillroster/pull/341) fixed shared task-object
exclusions at exact head `817fc48eabac53ba58d8716985baf61d39b29985`
and merged as the candidate base. The linked
[issue #340](https://github.com/tt-a1i/skillroster/issues/340) records the
real bilingual reproducer, root cause, and bounded acceptance criteria.
Sequential Spec/Evidence and Standards/Compatibility reviews were Clean.

The exact-main
[CI run 33300043677](https://github.com/tt-a1i/skillroster/actions/runs/33300043677)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at the candidate base. The final local gate for #341
passed 325 Rust unit tests, 8 acceptance tests, 114 CLI tests, and 152 Node
harness tests, plus strict Clippy, formatting, installation-surface validation,
archive README validation, the change-scope self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.39. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.38 until the v1.8.39 tag,
artifacts, checksums, and Homebrew package actually exist.

## Candidate gates

The candidate is not accepted until one exact final source revision passes:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and Node harnesses;
- `git diff --check`, installation-surface validation, archive README
  validation, and the CI change-scope self-test;
- four platform build and governance jobs plus the WSL2 governance smoke;
- downloaded checksum verification and an external four-archive comparison
  against the checked-in README Git blob.

Candidate preparation does not create or push a tag, publish a GitHub Release,
update Homebrew, or mutate an existing public release asset. Those operations
remain separately evidenced after candidate acceptance.
