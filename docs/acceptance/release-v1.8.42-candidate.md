# SkillRoster v1.8.42 candidate

## Release notes draft

SkillRoster 1.8.42 prevents broad native-CJK lexical overlap from authorizing
the complete instructions of an unrelated Skill.

- Ordinary `find` remains read-only and keeps the same bounded candidates,
  ranking scores, match reasons, and cross-language hint warning.
- Unhinted CJK `find --load` now requires direct selection evidence: an exact
  declared name or trigger, complete Skill-name token coverage, or an exact
  description phrase with at least two matching tokens.
- Broad description-token or CJK bigram overlap can still preserve a useful
  lexical candidate, but cannot return complete instructions.
- Weak loads fail closed with the typed
  `cjk_hint_required_for_weak_match` blocker, no partial instructions, and
  `files_changed=false`; a faithful Agent-authored English hint remains
  loadable.

The real-inventory dogfood used the anonymous public macOS arm64 v1.8.41
binary, a fresh read-only Snapshot of 263 Skills and 1,037 placements, and
isolated temporary state. For a Chinese product-dogfood task combining problem
discovery, root-cause analysis, repair, and retrospective, published v1.8.41
ranked an unrelated publishing Skill first and returned its complete 56 KiB
instructions even while warning that an English hint was needed. Three of
three runs in a fresh two-Skill fixture reproduced the same policy failure
without usage history or duplicate placements.

The fixed source preserves the ordinary Find projection byte-for-byte in both
the real inventory and minimal fixture. The same unhinted loads now return the
typed blocker with no result, while a faithful English diagnostic hint loads
`diagnose` completely.

This patch changes bounded lexical Find loading only. SQLite remains at schema
12, the JSON envelope remains at schema 1, and bundled Bootstrap content remains
at version 1.8.29. It does not mutate Agent or Skill files.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`dbf673b636a557de4b08e70d1d6e28d03ab3d841`.

[#355](https://github.com/tt-a1i/skillroster/pull/355) separated rank-preservation
evidence from complete-load authorization at exact head
`65f3c1fbaabc7e62636ad71a78c234bcf3b8572c` and merged as the candidate base.
The linked [issue #354](https://github.com/tt-a1i/skillroster/issues/354)
records the public reproducer, ranked hypotheses, deterministic fixture, and
bounded acceptance criteria. The exact-head PR
[CI run 33313718785](https://github.com/tt-a1i/skillroster/actions/runs/33313718785)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The exact-main
[CI run 33314035356](https://github.com/tt-a1i/skillroster/actions/runs/33314035356)
also passed the same four-platform matrix and aggregate gate at candidate base
revision `dbf673b636a557de4b08e70d1d6e28d03ab3d841`.

The test-before-fix regression failed because the broad unhinted CJK
`find --load` command succeeded and returned unrelated complete instructions.
At the final fix head, the full local gate passed 327 Rust unit tests, 8
acceptance tests, 117 CLI tests, and 152 Node harness tests, plus strict Clippy,
formatting, installation-surface validation, archive README validation, the
change-scope self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.42. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.41 until the v1.8.42 tag,
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
