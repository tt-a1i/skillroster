# SkillRoster v1.8.32 release receipt

## Release notes

SkillRoster 1.8.32 keeps every Agent continuation bound to the executable that
emitted it.

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

## Release evidence

The public baseline was v1.8.31. Candidate preparation started from exact
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

[#310](https://github.com/tt-a1i/skillroster/pull/310) prepared the release from
that exact baseline and passed Linux, Windows, macOS arm64, macOS x86_64,
change-scope, and aggregate CI gates. Candidate
[run 33265583930](https://github.com/tt-a1i/skillroster/actions/runs/33265583930)
then passed the exact-SHA repository gate, all four build/governance jobs, and
the WSL2 Linux archive smoke.

The annotated `v1.8.32` tag object
`9fade7229270ec27871445c3d7a595932f0df450` resolves exactly to
`4329cf2a2b476aafa6d77fc6edcfb6b31cd604ed`. The official tag workflow
[run 33265997530](https://github.com/tt-a1i/skillroster/actions/runs/33265997530)
passed the same strict repository gate, Linux, Windows, macOS arm64 and x86_64
build/governance jobs, and the WSL2 Linux governance smoke.

The [public GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.32)
contains four platform archives and four adjacent checksum files:

- macOS arm64:
  `15eaac54a3348e6d04e95ae20680283e4900ba1e9a03cc3b8806a40dfe998b8d`;
- macOS x86_64:
  `336f9e4a0c4ad7a21f71fc569aaa313e5bb3f60394f9058f1be98139bb2136e8`;
- Windows x86_64:
  `e471f4bd3e7a6b1537937546afddce780a9296e8410073490d6870aa70ecf673`;
- Linux x86_64:
  `860a1545e2c0f10937fbffd7ad620a9579a48984bf88862d317f05996ad41ff6`.

All eight tag-workflow artifacts passed their adjacent checksums before
publication. The public macOS arm64 archive was then downloaded anonymously,
matched its published checksum, reported `skillroster 1.8.32`, and passed the
complete synthetic release governance smoke.

The [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster) publishes
v1.8.32 macOS arm64 and x86_64 Linux bottles. Both Homebrew test-bot jobs passed
in [run 33266757082](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33266757082),
and exact-head `brew pr-pull` published the bottles in
[run 33267024232](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33267024232).
Tap `main` reached `e2e9c4026d383aa7123bc4d4d921d0269058a125`.
The public macOS arm64 bottle was downloaded anonymously, matched Formula
checksum
`71f7a2b694161a6cd7f0f2f9d555cf5b91476ad507c506161a731c6b752d1308`,
reported `skillroster 1.8.32`, and passed the same governance smoke.

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
