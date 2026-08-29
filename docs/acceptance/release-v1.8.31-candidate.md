# SkillRoster v1.8.31 release receipt

## Release notes

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

## Release evidence

The public baseline was v1.8.30. Candidate preparation started from exact
`origin/main` revision `bf82dfbaccaceedcd048284a4261bc692c0a34f6`, after
[#304](https://github.com/tt-a1i/skillroster/pull/304) merged the Evidence-scope
fix and [#305](https://github.com/tt-a1i/skillroster/pull/305) merged the
privacy-safe packaged first-use acceptance record. Both pull requests passed
their Linux, Windows, macOS arm64, macOS x86_64, change-scope, and aggregate CI
gates. Candidate
[run 33260982973](https://github.com/tt-a1i/skillroster/actions/runs/33260982973)
then passed the exact-SHA repository gate, all four build/governance jobs, and
the WSL2 Linux archive smoke.

The annotated `v1.8.31` tag object
`e214a30028f13614319a1d100dd58e9cd1bae3ca` resolves exactly to
`aa688e265a0541cbe40548d847ac9349676fc02f`. The official tag workflow
[run 33261632527](https://github.com/tt-a1i/skillroster/actions/runs/33261632527)
passed the same strict repository gate, Linux, Windows, macOS arm64 and x86_64
build/governance jobs, and the WSL2 Linux governance smoke.

The [public GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.31)
contains four platform archives and four adjacent checksum files:

- macOS arm64:
  `6cf0dbdff7f1f6e515cbd88434d6f3c219d7da405dcf3cfd00160807d5ff5e11`;
- macOS x86_64:
  `abac4e5b994bf4b6adfba9fb82d94ac471a2712881097708071c39b5747c1b23`;
- Windows x86_64:
  `e36db7f2d0b2fe768382c18574893e022e82d4baba9652eb2371c8eb7cd89c9c`;
- Linux x86_64:
  `292d4bd88c83860ea43516a0cafedd7aae3a8e74bed1579c055d434680ea5e08`.

All eight tag-workflow artifacts passed their adjacent checksums before
publication. The public macOS arm64 archive was then downloaded anonymously,
matched its published checksum, reported `skillroster 1.8.31`, and passed the
complete synthetic release governance smoke.

The [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster) publishes
v1.8.31 macOS arm64 and x86_64 Linux bottles. Both Homebrew test-bot jobs passed
in [run 33262319710](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33262319710),
and exact-head `brew pr-pull` published the bottles in
[run 33262638714](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33262638714).
Tap `main` reached `1841601539063f4e5a58a63cc778ca7cf5bea8b1`.
The public macOS arm64 bottle was downloaded anonymously, matched Formula
checksum
`ef74fd693ac351db3cf138420345a8bea17bbbb28229b7e4cdc25182acfa275f`,
reported `skillroster 1.8.31`, and passed the same governance smoke.

## Accepted gates

The exact tagged revision passed:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and both Node routing harnesses;
- `git diff --check`, installation-surface validation, and the CI change-scope
  self-test;
- four platform build and governance jobs plus the WSL2 governance smoke.

The release WSL boundary remains WSL2. WSL1 mutation fails closed when its
kernel cannot provide the atomic no-replace rename required by SkillRoster's
handle-bound recovery model.
