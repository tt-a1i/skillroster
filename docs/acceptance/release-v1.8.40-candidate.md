# SkillRoster v1.8.40 candidate

## Release notes draft

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

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`3a03538920aa642f450fe8adc74a758f4a52ad3c`.

[#345](https://github.com/tt-a1i/skillroster/pull/345) fixed coordinated task
exclusions at exact head `b8287753df7fccbbafd5f408b7decba5cdecef8b`
and merged as the candidate base. The linked
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

The test-before-fix public CLI regression failed with an empty English
`task_exclusions` array. At the final fix head, the full local gate passed 326
Rust unit tests, 8 acceptance tests, 114 CLI tests, and 152 Node harness tests,
plus strict Clippy, formatting, installation-surface validation, archive README
validation, the change-scope self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.40. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.39 until the v1.8.40 tag,
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
