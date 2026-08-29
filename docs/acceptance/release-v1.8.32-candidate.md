# SkillRoster v1.8.32 candidate

## Release notes draft

SkillRoster 1.8.32 keeps every Agent continuation bound to the executable that
created the current local state.

- Home, Scan, Report, Plan, Apply, Undo, recovery, and nested suggested actions
  now start with the exact absolute path of the running SkillRoster binary.
- A release archive or explicitly selected binary therefore cannot silently
  hand the next step to an older `skillroster` found on `PATH`.
- Source-confirmation overflow details preserve the same executable and
  discovery context in schema 4 while remaining compatible with schema 1-3.
- A non-Unicode current executable path fails closed instead of falling back to
  an ambiguous PATH lookup.

This patch release keeps SQLite schema 12 and JSON envelope schema 1. There is
no database migration and no change to the local-only, explicit-confirmation,
Receipt-backed mutation model. The bundled Bootstrap instructions remain at
content version 1.8.29, so upgrading the CLI does not cause an unnecessary
Bootstrap replacement Plan.

## Preparation evidence

The public baseline is v1.8.31. Candidate preparation starts from exact
`origin/main` revision `52aa0cdd089127a6f1a99c9cb84e26a8b538055d`, after
[#309](https://github.com/tt-a1i/skillroster/pull/309) merged the executable
binding fix and closed
[#308](https://github.com/tt-a1i/skillroster/issues/308). The pull request passed
Linux, Windows, macOS arm64, macOS x86_64, change-scope, and aggregate CI gates.
Its Linux gate also ran the real non-Unicode executable process test.

The original failure was reproduced with the official v1.8.31 macOS arm64
archive: Home returned a bare `skillroster` continuation, PATH selected local
v1.8.28, and the next command failed because schema 12 was newer than schema
10. After the fix, a fresh real-home Home → Scan → Report journey completed
while PATH still selected v1.8.28; every emitted argv stayed bound to the current
release binary and no Agent files changed.

The source candidate and Cargo package are versioned as 1.8.32. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.31 until the v1.8.32 tag,
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
