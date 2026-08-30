# SkillRoster v1.8.37 candidate

## Release notes draft

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
five returned the intended Top-1 Skill, and exact `find --load` checks for
`diagnose` and `pdf` returned verified complete content without changing files.

This patch changes deterministic Find ranking and description polarity only.
SQLite remains at schema 12, the JSON envelope remains at schema 1, and bundled
Bootstrap content remains at version 1.8.29. It does not mutate Agent or Skill
files.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`c6c4f5235d428fc664e5c47245b08ba5b8540002`.

[#331](https://github.com/tt-a1i/skillroster/pull/331) fixed precise
cross-language routing at exact head
`3dfdb74f13e15e2bba945a5684017e4deebd857f` and merged as the candidate base.
The linked [issue #330](https://github.com/tt-a1i/skillroster/issues/330)
records the reproducible failure, ranked hypotheses, root cause, and bounded
acceptance criteria. Sequential Spec and Standards reviews passed after the
implementation was corrected to keep partial multi-token name hits from being
treated as direct name evidence.

The exact-main
[CI run 33287816660](https://github.com/tt-a1i/skillroster/actions/runs/33287816660)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at the candidate base. The final local gate for #331
passed 321 Rust unit tests, 8 acceptance tests, 105 CLI tests, and 152 Node
harness tests, plus strict Clippy, formatting, installation-surface validation,
archive README validation, the change-scope self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.37. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.36 until the v1.8.37 tag,
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
