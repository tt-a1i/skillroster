# SkillRoster v1.8.31 candidate

## Release notes draft

SkillRoster 1.8.31 closes an Evidence-scope hole in raw Roster planning.

- A raw `roster_changes` request now fails closed unless its cited Evidence
  covers every Skill or Placement being changed.
- Rejection uses the stable, bounded `plan_evidence_scope_mismatch` error and
  reports `files_changed=false`; no applicable Plan is created.
- Relevant Skill Evidence and Placement Evidence remain accepted, and
  Finding-derived Roster planning keeps its existing behavior.

This patch release keeps SQLite schema 10 and JSON envelope schema 1. There is
no migration and no change to the local-only, explicit-confirmation,
Receipt-backed mutation model. The bundled Bootstrap instructions remain at
content version 1.8.29, so upgrading the CLI does not cause an unnecessary
Bootstrap replacement Plan.

## Preparation evidence

The public baseline is v1.8.30. Candidate preparation starts from exact
`origin/main` revision `bf82dfbaccaceedcd048284a4261bc692c0a34f6`, after
[#304](https://github.com/tt-a1i/skillroster/pull/304) merged the Evidence-scope
fix and [#305](https://github.com/tt-a1i/skillroster/pull/305) merged the
privacy-safe packaged first-use acceptance record. Both pull requests passed
their Linux, Windows, macOS arm64, macOS x86_64, change-scope, and aggregate CI
gates.

The source candidate and Cargo package are versioned as 1.8.31. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.30 until the v1.8.31 tag,
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
