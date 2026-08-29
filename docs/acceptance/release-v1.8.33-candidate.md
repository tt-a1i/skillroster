# SkillRoster v1.8.33 release receipt

## Release notes

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

The four platform archives contain the Rust CLI binary, `README.md`, and the
Apache-2.0 `LICENSE`; the installed `skillroster` command gains no pilot
subcommand. Rust CLI behavior is unchanged apart from its version string. The
protocol, harness, fixtures, and tests are available from a source checkout.
Because those immutable archives were built from the release-candidate tag,
their packaged README still names v1.8.32 as the then-current public release
and links its evidence. The v1.8.33 binary, tag, checksums, and public release
metadata are unaffected. A later release must use version-neutral packaged
copy or generate archive-specific release text before tagging; this follow-up
is tracked by [#314](https://github.com/tt-a1i/skillroster/issues/314).

This patch release keeps SQLite schema 12 and JSON envelope schema 1. There is
no database migration and no change to the local-only, explicit-confirmation,
Receipt-backed mutation model. The bundled Bootstrap instructions remain at
content version 1.8.29. The protocol authorizes no participant recruitment,
messaging, real-environment read, or Apply; those remain separate #261
boundaries.

## Release evidence

The public baseline was v1.8.32. Candidate preparation started from exact
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

[#313](https://github.com/tt-a1i/skillroster/pull/313) prepared the release from
that exact baseline and passed Linux, Windows, macOS arm64, macOS x86_64,
change-scope, and aggregate CI gates. Candidate
[run 33269706239](https://github.com/tt-a1i/skillroster/actions/runs/33269706239)
then passed the exact-SHA repository gate, all four build/governance jobs, and
the WSL2 Linux archive smoke.

The annotated `v1.8.33` tag object
`0cb502d7c14562387ab4e06ef46ed3767ec3e7dd` resolves exactly to
`a9b649efbc0ca0b5c7a853e795ee6257c21629c8`. The official tag workflow
[run 33270251405](https://github.com/tt-a1i/skillroster/actions/runs/33270251405)
passed the same strict repository gate, Linux, Windows, macOS arm64 and x86_64
build/governance jobs, and the WSL2 Linux governance smoke.

The [public GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.33)
contains four platform archives and four adjacent checksum files:

- macOS arm64:
  `20f3ab866390ad9eef631fef20ddf86618ba200ecbd9a523f65bcf50483e0211`;
- macOS x86_64:
  `5570010a87940251b1fdf49ec405ef49971de2acd199ab3c1635a40d324eb554`;
- Windows x86_64:
  `f10973adfb9fdd37e5da1b66c0cb59a28025e520d1bac4d870decf5aee57b8d1`;
- Linux x86_64:
  `f9f6aa4cfd07f620696976f8034fcf4fe97efe718dc6cfce9b6ac67040cb4907`.

All eight tag-workflow artifacts passed their adjacent checksums before
publication. The public macOS arm64 archive was then downloaded anonymously,
matched its published checksum, reported `skillroster 1.8.33`, and passed the
complete synthetic release governance smoke.

The [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster) publishes
v1.8.33 macOS arm64 and x86_64 Linux bottles. Both Homebrew test-bot jobs passed
in [run 33270842211](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33270842211),
and exact-head `brew pr-pull` published the bottles in
[run 33271338074](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33271338074).
Tap `main` reached `e9cf1d23cc78d53b6c7722d1e930f01e6bb18876`.
The public macOS arm64 bottle was downloaded anonymously, matched Formula
checksum
`6f8e5789f5ec72cb0966c61e62927c7c9f4cf4c364e67cb7f4ff4c3e53371353`,
reported `skillroster 1.8.33`, and passed the same governance smoke. The Linux
x86_64 bottle checksum is
`b1272a53f33cfa3fc4b901dd3359c80d1812d957d7f667b3b630a187625a6a20`.

## Accepted gates

The exact tagged revision passed:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and Node harnesses;
- `git diff --check`, installation-surface validation, and the CI change-scope
  self-test;
- four platform build and governance jobs plus the WSL2 governance smoke.

The release WSL boundary remains WSL2. WSL1 mutation fails closed when its
kernel cannot provide the atomic no-replace rename required by SkillRoster's
handle-bound recovery model.
