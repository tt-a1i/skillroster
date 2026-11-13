# v1.9.0 source candidate acceptance

Local validation on 2026-11-14 used macOS ARM64, Rust 1.89.0 and Node 22.23.3.
The source candidate is CLI **1.9.0**, with Bootstrap **1.8.53**. The published
installation surface remains pinned to the existing v1.8.48 Formula. No tag,
GitHub Release, npm publication, website deployment or promotion was performed.

## Local gates

`bash scripts/ci-full-check.sh` passed formatting, strict Clippy, all enabled
Rust tests, the Node harness, npm generation/packaging tests and all three
surface verifiers:

| Check | Result |
| --- | --- |
| Rust tests | 681 passed, 0 failed; 1 existing opt-in scale measurement ignored |
| Node harness | 152 passed |
| npm distribution tests | 3 passed |
| Release first-use | Empty, small, duplicate and large scenarios passed |
| Release governance smoke | Setup, Apply, verified Undo and retained-ID checks passed |
| Native npm/npx | Real ARM64 binary packed, installed offline, Check and MCP passed |

The npm generator also rejects a version outside the source candidate or its
prereleases. Main and all four platform templates use exact 1.9.0 versions.
The real native npx probe accepted null tool arguments, a null list cursor and
blank stdio lines, then returned complete verified content for a Chinese task.
Foreign-target archives in local npm tests are synthetic packaging fixtures;
they are not evidence of execution on those operating systems.

## Real-home isolation and observations

The real-home run supplied an independent temporary `--state-dir` to every
CLI and MCP process. It ran Check, Inspect, multilingual Find/load, an
inventory-only load and a read-only Plan attempt. It never invoked Init,
Apply, Undo or source-permission confirmation against real directories.

Known Agent Skill roots, the shared local Skill store and default SkillRoster
state were recorded before and after. Regular files were hashed; symlinks were
recorded without following external targets. These records were unchanged.
The default `~/.skillroster` directory was absent before and after the test.
Session files were read through the bounded scanner, not copied as raw text.
Temporary inspection state was removed after collecting local diagnostics.

| Observed fact | Value |
| --- | --- |
| Independent Skills / Placements | 258 / 504 |
| Default-exposed Placements | 233 |
| Known exposed frontmatter / complete entrypoint bytes | 82,678 / 1,900,850 |
| Unknown exposed entrypoint measurements | 4 |
| Shared roots / shared-read-only Placements | 3 / 274 |
| Provider-read-only Placements | 86 |
| Unconfirmed external links | 5 |
| Session coverage | 8 sampled Agents; 4 complete, 4 limited |
| Observed-use Agents | 2 |
| Human Check | 25 lines at the tested width |

Sampled file/byte limits and line-alignment limitations remained visible.
Counts and byte measurements describe this local observation, not runtime
tokens or a claim that unobserved Skills are unused.

On this optimized local binary, the first Check took 2.377 seconds and a cached
Check took 0.0355 seconds. An independent Find-only store completed a first
Scan plus verified load in 1.8175 seconds; its cached load took 0.0398 seconds.
Those two Find calls left **1 Scan, 0 Reports, 0 Plans and 0 Receipts**. These
are individual observations on this machine, not a general speed guarantee.

Chinese rewriting loaded `humanizer-zh` with the shared-read-only boundary
intact. An exact inventory-only `template-creator` lookup loaded complete
verified instructions while retaining provider-read-only authority and no
default exposure. Retrieval did not execute either Skill's task.

## Friction found and changes

- MCP Find previously ran Check/Report before every query. It now prepares only
  the needed Snapshot. Explicit Snapshot requirements retain normal CLI
  optimistic-lock refusals without an automatic Scan.
- SDK nulls and blank lines are accepted without extra parse-error frames.
- PTY regressions verify split Escape, arrows and bracketed-paste cancellation,
  terminal-mode restoration and an empty input queue before shell control resumes.
- The real large-Roster Plan remained blocked because Codex had shared roots
  but no eligible exclusive mutation destination. Its old message incorrectly
  suggested no roots were included. The revised message identifies that exact
  distinction, offers keeping the Roster or configuring an exclusive root and
  rescanning, and preserves the read-only boundary. No Plan or Receipt was saved.
- Check retains blocked/Plan-available labels and gives exact external-path
  review guidance. The architecture query's faithful hint refinement reached
  the existing same-name ambiguity gate; no variant was silently selected.

Detailed local logs, protocol frames and the native candidate binary are in
ignored `dist/v1.9.0-candidate/`. Those real-home diagnostics are local review
artifacts and are not release archive inputs.

## Release handoff

Local candidate gates passed. Before public delivery, the existing release
workflow must still build and test this source SHA on Linux, Windows, both
macOS targets and WSL2. Review the four real archives/checksums, stage npm
packages with `scripts/release/build-npm.py`, and publish platform packages
before the main package. Formula hashes and public release labels change only
after the corresponding artifacts are published and verified.
