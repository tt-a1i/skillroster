# SkillRoster v1.8.35 candidate

## Release notes draft

SkillRoster 1.8.35 makes Bootstrap Setup Plans report their real Agent impact.

- `setup --json` and `plan --show` now include every logical Agent whose
  Bootstrap target will actually be installed or replaced.
- Agents that share one physical root remain distinct in the bounded impact
  summary, so deduplication cannot hide affected callers.
- Already-current, unsupported, and retained-local targets are not counted as
  affected.
- Complete Plans created by older versions with an incorrect zero-Agent
  summary remain immutable, but Setup will not reuse them.

This patch changes Setup Plan reporting and reuse identity only. SQLite remains
at schema 12, the JSON envelope remains at schema 1, and bundled Bootstrap
content remains at version 1.8.29. Setup is still preview-only until explicit
Apply confirmation, and Apply/Undo remain Receipt-backed.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`3f2aadfc539c86d898bb8f6e7be21f4487e09648`, where
[#321](https://github.com/tt-a1i/skillroster/pull/321) closed
[#320](https://github.com/tt-a1i/skillroster/issues/320).

The defect was reproduced first with the public v1.8.34 binary: six detected
and missing Agents, 25 planned filesystem operations, and four physical targets
were incorrectly summarized as zero affected Agents. The corrected source
created a new Plan instead of reusing the old summary and reported all six
logical Agents without modifying the Home fixture.

PR #321 passed change-scope, Linux x86_64, Windows x86_64, macOS arm64, macOS
x86_64, and aggregate CI gates at exact head
`3e259fdbec93c34160898a725f6ef8efae3d239b`. Its local full gate passed 321
Rust unit tests, 8 acceptance tests, 98 CLI tests, and 152 Node harness tests.
Independent Spec and Standards reviews were Clean.

The source candidate and Cargo package are versioned as 1.8.35. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.34 until the v1.8.35 tag,
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
