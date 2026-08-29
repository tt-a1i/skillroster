# SkillRoster v1.8.33 candidate

## Release notes draft

SkillRoster 1.8.33 publishes, in the public source tree, a
[privacy-safe protocol](../research/roster-recommendation-pilot-v1.md) and
offline Node research harness for evaluating whether independent users accept
or reject proposed Core and On-demand Roster decisions.

- A strict offline ledger separates setup, Agent invocation, diagnosis, Plan,
  deterministic retrieval, recommendation decision, and final-task outcomes.
- Scheduled participants who leave before observation remain reported without
  invented inventory, Agent, or recommendation facts.
- The ledger excludes raw conversations, Skill contents, secrets, participant
  identities, and identifying absolute paths; its summary emits only bounded
  aggregate counts.
- Input is read through one retained file descriptor, rejects symlinks and
  non-regular files, and is limited to 1 MiB before JSON parsing.
- A checked-in three-participant synthetic dry run exercises acceptance,
  rejection, a typed blocker, invocation failure, and retrieval failure. It
  authorizes no ranking, embedding, model, or policy change. Its evidence is
  recorded in the [synthetic dry-run receipt](roster-recommendation-pilot-v1-dry-run.md).

The four platform archives still contain only the Rust CLI binary and LICENSE;
the installed `skillroster` command gains no pilot subcommand. Rust CLI behavior
is unchanged apart from its version string. The protocol, harness, fixtures,
and tests are available from a source checkout.

This patch release keeps SQLite schema 12 and JSON envelope schema 1. There is
no database migration and no change to the local-only, explicit-confirmation,
Receipt-backed mutation model. The bundled Bootstrap instructions remain at
content version 1.8.29. The protocol authorizes no participant recruitment,
messaging, real-environment read, or Apply; those remain separate #261
boundaries.

## Preparation evidence

The public baseline is v1.8.32. Candidate preparation starts from exact
`origin/main` revision `b16af99ef9171f6a7d5540c9e4c32f2a4fb3c665`, after
[#312](https://github.com/tt-a1i/skillroster/pull/312) merged the frozen pilot
protocol and closed
[#260](https://github.com/tt-a1i/skillroster/issues/260). That pull request
passed change-scope, Linux, Windows, macOS arm64, macOS x86_64, and aggregate CI
gates. Independent Spec and Standards reviews were Clean after the initial
Standards findings were fixed.

The local full gate passed 321 Rust unit tests, 8 acceptance tests, 97 CLI
tests, and 152 Node harness tests. The frozen synthetic summary is reproduced
exactly from its public fixture, and no real participant or environment data is
present.

The source candidate and Cargo package are versioned as 1.8.33. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.32 until the v1.8.33 tag,
artifacts, checksums, and Homebrew package actually exist.

## Candidate gates

The candidate is not accepted until one exact final source revision passes:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and Node harnesses;
- `git diff --check`, installation-surface validation, and the CI change-scope
  self-test;
- four platform build and governance jobs plus the WSL2 governance smoke.

Candidate preparation does not create or push a tag, publish a GitHub Release,
update Homebrew, or mutate any public release asset. Those steps are recorded
only after their external evidence exists.
