# SkillRoster v1.8.43 candidate

## Release notes draft

SkillRoster 1.8.43 makes Agent-authored retrieval hints both more reliable and
safer at the complete-instruction boundary.

- A hint that completely names a multi-token Skill can now outrank broad native
  task overlap, matching the existing direct-evidence rule for single-token
  names. Partial multi-token name matches remain weak.
- Ordinary `find` remains read-only and keeps weak lexical candidates visible
  for Agent judgment, with explicit direct-selection match evidence.
- `find --load` now requires direct selection evidence even when the Agent
  supplies a hint.
- Direct evidence is an exact declared name or trigger, complete Skill-name
  token coverage, or complete normalized coverage of one task or hint phrase
  by the positive Skill description, with at least two tokens.
- Partial Skill-name matches and unrelated description-token overlap cannot
  authorize a complete load.
- A weak hinted load fails closed with the typed
  `hint_direct_selection_evidence_required` reason, no partial instructions,
  and `files_changed=false`; the Agent can retry with a more specific hint.

The real-inventory dogfood used fresh read-only Snapshots of 263 Skills and
1,037 placements with isolated temporary state. First, a faithful hint naming
`simplify-codebase` lost Top-1 to broad native overlap in three of three runs;
the ranking fix moved `simplify-codebase` to Top-1 in three of three runs while
preserving the same-name variant boundary and exact verified variant load.

Second, for the task “把这组产品决定整理成可执行规格” and the hint `convert
product decisions into an executable specification`, the previous behavior
loaded `product-business-analysis` instead of the intended `to-spec`. The
wrong Top-1 had only partial name and description-token overlap.

The #361 fix preserves the ordinary Find ranking and order while adding a
direct-selection match reason. The same weak load now returns the typed blocker
with no result. A faithful direct capability hint ranks `to-spec` first; the
real-inventory `find --load` replay then correctly stops at its existing
same-name variant ambiguity, while exact verified variant loads remain
available.

This patch changes hinted lexical ranking, direct-selection evidence, and
verified-load authorization. SQLite remains at schema 12, the JSON envelope
remains at schema 1, and bundled Bootstrap content remains at version 1.8.29.
It does not mutate Agent or Skill files.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`5996148473a09d39b9956e7832f091894fdff55d`.

[#359](https://github.com/tt-a1i/skillroster/pull/359) aligned complete
multi-token Skill-name hints with the existing direct-evidence rule at exact
head `d11c05df35128aad06c28f1479b548a2bf2dc433` and merged as revision
`9964b1f90a7d76de5f88d5817b297255de3b2a3a`. The linked
[issue #358](https://github.com/tt-a1i/skillroster/issues/358) records the
real-inventory ranking failure and bounded acceptance criteria. Both
independent exact-head reviews passed. The PR
[CI run 33318742373](https://github.com/tt-a1i/skillroster/actions/runs/33318742373)
and exact-main
[CI run 33319047172](https://github.com/tt-a1i/skillroster/actions/runs/33319047172)
passed the four-platform matrix and aggregate gate at their exact revisions.

[#361](https://github.com/tt-a1i/skillroster/pull/361) added the hinted-load
authorization boundary at exact head
`e320f802260dc19eb1b239f61b115600399b7b03`. The linked
[issue #360](https://github.com/tt-a1i/skillroster/issues/360) records the
public reproducer, root cause, deterministic fixture, and bounded acceptance
criteria. Two independent exact-head reviews passed with no blocking findings.
The PR
[CI run 33320656155](https://github.com/tt-a1i/skillroster/actions/runs/33320656155)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate.

The exact-main
[CI run 33320872443](https://github.com/tt-a1i/skillroster/actions/runs/33320872443)
passed the same four-platform matrix and aggregate gate at candidate base
revision `5996148473a09d39b9956e7832f091894fdff55d`.

The test-before-fix regression failed because the weak hinted `find --load`
command succeeded and returned unrelated complete instructions. At the final
fix head, the full local gate passed 327 Rust unit tests, 8 acceptance tests,
119 CLI tests, and 152 Node harness tests, plus strict Clippy, formatting,
installation-surface validation, archive README validation, the change-scope
self-test, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.43. Public install
examples, the website current-release labels, README release evidence, and the
Homebrew Formula deliberately remain at v1.8.42 until the v1.8.43 tag,
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
