# Read-only task routing regression

Date: 2026-10-22. Base: `21d14c7` (published CLI version 1.8.46).

## Observed defect

An isolated temporary home contained two synthetic Skills:

- `code-review`: `Review code for correctness and security. Inspect files for defects.`
- `simplify-codebase`: `Simplify and refactor code for maintainability.`

After Scan, the installed 1.8.46 binary loaded `code-review` for
`Review this code for correctness and security`. Adding
`, do not modify files` excluded that same Skill solely because its positive
description contained `files`. With the faithful `code review` hint, one-call
Find/load then failed with `verified_skill_load_blocked` and
`hint_direct_selection_evidence_required` on the remaining candidate.

The original task remained authoritative, so refining its hint could not repair
this false exclusion. Separately, replacing `do not` with `don't` in
`but don't simplify or refactor it` produced no recognized exclusion at all.

## Change and replay

The existing per-clause task-object rule now covers `file` / `files` and `文件`,
alongside `code` and `代码`. An object following constraint material does not
by itself prohibit a Skill that inspects that object. Declared `modify`
capabilities still conflict; a standalone `do not file` remains an exclusion.
English `don't` and `don’t` use the same case-insensitive, complete-marker
boundary as `do not`. No ranking weights, hint authority, package validation,
or mutation behavior changed.

Both binaries were replayed against the same synthetic home with separate
temporary state directories, using literal argv:

```text
skillroster --home HOME --state-dir STATE --json scan
skillroster --home HOME --state-dir STATE --json find --hint "code review" --load --limit 1 -- TASK
```

| Task | Installed 1.8.46 | Working-tree binary |
| --- | --- | --- |
| Baseline review | Complete `code-review` load | Complete `code-review` load |
| Review, do not modify files | Typed load blocker | Complete `code-review` load; original exclusion retained |
| Review, but don't simplify or refactor it | Complete load, exclusion absent | Complete load, original exclusion recognized |

The public regression additionally tests sentence-final singular/plural file
objects, Chinese `文件`, both apostrophe forms, preserved original TASK and
baseline score, exclusion of a file-modification Skill, and standalone object
exclusions. Unit controls retain positive text for `don'tpanic`, `don’tpanic`,
and embedded negative states such as `diagnose why tests do not pass`.

## Validation and limits

- `cargo build --locked --offline` and `cargo run --locked --offline -- --help`: passed.
- `cargo test --test cli public_find_`: 20 passed.
- `cargo test task_routing_sections_`: 3 passed.
- `bash scripts/ci-full-check.sh`: passed; formatting, strict Clippy, 493 Rust
  tests passed (one existing ignored test), 152 Node tests passed, README,
  installation surface, and release archive checks passed.
- Independent focused static review: no actionable findings.
- `git diff --check`: passed.

These are deterministic CLI and parser checks, not a new live-model trial or
proof of general negation understanding. The bounded lexical grammar remains
explicit; semantic interpretation and the original user's constraints remain
the caller's responsibility. No real user Skills or Agent configuration changed.
This record does not establish merge, publication, or installed-binary upgrade.
