# SkillRoster v1.8.41 candidate

## Release notes draft

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

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`d8a871a2c9bf2cec46cd07867df27f9a0f767cdf`.

[#351](https://github.com/tt-a1i/skillroster/pull/351) added the typed load
abstention at exact head `8748fc891ab9734286f09a98c9fbe82f56e6abfe`
and merged as the candidate base. The linked
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

The test-before-fix regression failed because the weak unhinted `--load`
command succeeded and returned unrelated complete instructions. At the final
fix head, the full local gate passed 327 Rust unit tests, 8 acceptance tests,
116 CLI tests, and 152 Node harness tests, plus strict Clippy, formatting,
installation-surface validation, archive README validation, the change-scope
self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.41. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.40 until the v1.8.41 tag,
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
