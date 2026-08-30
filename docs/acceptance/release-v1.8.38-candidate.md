# SkillRoster v1.8.38 candidate

## Release notes draft

SkillRoster 1.8.38 makes Agent routing obey explicit task exclusions and makes
verified loading prefer the Skill placement that is actually exposed to an
Agent.

- `find` recognizes bounded, independent `不要` / `也不要` / `do not`
  clauses as negative capability constraints instead of positive task
  evidence. A strong retrieval hint cannot override an explicit exclusion.
- JSON keeps the exclusion decision auditable: `task_exclusions` preserves the
  recognized clauses, while `task_exclusion_effects` provides a bounded
  10-item preview with complete count and truncation facts. Neither returns
  Skill content.
- After Skill identity ranking, `find --load` prefers an eligible
  `default_exposed` placement over hidden exact-copy inventory such as
  `.bak-*` directories.
- Hidden copies remain inventory and duplicate evidence. When no exposed copy
  exists, source-only and On-demand Skills retain a deterministic eligible
  fallback.
- Existing root, trust, digest, fingerprint, UTF-8, size, and drift checks are
  unchanged and still fail closed.

Real-inventory replay used a fresh Snapshot of 263 Skills and 1,037
placements. Explicit modification exclusions no longer routed to modification
Skills, while the positive task kept its prior score. `code-review` loaded
from an Agent-owned exposed placement instead of a `.bak-*` copy. Both replays
reported `files_changed=false`.

This patch changes Find routing and verified placement selection only. SQLite
remains at schema 12, the JSON envelope remains at schema 1, and bundled
Bootstrap content remains at version 1.8.29. It does not mutate Agent or Skill
files.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`03dc398cd7ca8050eac64ad28987b5bf25e8e9fa`.

[#335](https://github.com/tt-a1i/skillroster/pull/335) fixed explicit task
exclusions and merged as `30ce00dde95567d28b4e567078120fb446770fbf`.
The linked [issue #334](https://github.com/tt-a1i/skillroster/issues/334)
records its reproducer and acceptance boundary.

[#337](https://github.com/tt-a1i/skillroster/pull/337) fixed exposed-placement
loading and merged as the candidate base. The linked
[issue #336](https://github.com/tt-a1i/skillroster/issues/336) records the
real `.bak-*` load failure, root cause, and bounded acceptance criteria.
Sequential Spec and Standards reviews were Clean for both changes. The final
Windows-specific test representation correction was also reviewed in that
order before merge.

The exact-main
[CI run 33295591534](https://github.com/tt-a1i/skillroster/actions/runs/33295591534)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate at the candidate base. One first-attempt Linux test
hit an unrelated transient `ETXTBSY`; the failed job was rerun at the same SHA
and passed without a source change. The final local gate for #337 passed 325
Rust unit tests, 8 acceptance tests, 112 CLI tests, and 152 Node harness tests,
plus strict Clippy, formatting, installation-surface validation, archive
README validation, the change-scope self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.38. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.37 until the v1.8.38 tag,
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
