# v1.9.1 local usability acceptance

Validated on macOS with Rust 1.85 on 2026-11-14. This is an **unpublished local
candidate**, not a release or independent-user evaluation. Public installation
remains v1.9.0; bundled Bootstrap remains 1.8.54. No bundled Skill files changed.

Implementation commits: `7f56e75` and final resource-bound fix
`1bd040a7cb8c5fa07e8a5a7ebcb18e9e9b4cab59` on
`codex/v1.9.1-usability`, based on main `910adaf`.
Optimized candidate executable SHA-256:
`e5e39800d65ac2e860572503dddad4b4e3bef5e9283fcba4170d15751fd75638`.

## Changes and boundaries

- CLI/MCP loads share one evidence gate for every language. Small lexical
  aliases and corroborated capability/domain features support short hints;
  domain-generic terms are downweighted and cannot authorize loading alone.
  Strong original-task matches remain protected from unrelated hints.
- Blocked loads keep up to ten candidate summaries and no instructions in
  `error.details.retrieval`, including Snapshot and truncation facts. Same-name,
  declared-content, archive, trust, path and fingerprint refusals remain intact.
- Session sampling fairly distributes independent read and parse budgets across
  recent files and beginning/middle/end regions. Parsing recognizes current
  local Codex function/custom-tool calls, Pi toolCall and Claude tool_use shapes.
  Literal read requests are separated from prompt examples and fixture writers;
  dynamic shell expressions remain unobserved.
- Check and MCP implicit reads compare a bounded metadata signature before reuse.
  Pinned reads refuse drift without rescanning. Metadata detection supplements
  the existing exact load/Plan hashes; it does not replace them.
- Supported Finding summaries expose schema-versioned Plan templates. Human
  `plan --finding ID [--core-limit N]` and `plan --adopt-shared [--core-limit N]`
  use the same immutable preview validators. Help and unknown-field errors list
  the Finding and shared-adoption request fields.
- Undo review aggregates recorded changes and scope instead of printing backup
  paths. Known pre-confirmation refusals report no file changes; uncertain
  post-mutation outcomes remain unknown. Project-local Skill directories produce
  explicit-selection hints, not automatic reads or mutation authority.

No new dependencies, database tables, locks or MCP mutation tools were added.
`src/app.rs` decreased from 15,218 to 14,179 lines; Find, human review, session
sampling and filesystem freshness use focused modules.

## Fixed session input comparison

The live machine had 269 Skills, 518 Placements and 236 default-exposed
Placements at the final real-home check. To avoid measuring an actively growing
conversation twice, 367 Codex/Pi/Claude JSONL files were frozen with APFS
copy-on-write clones: **2,498,312,416 bytes**. The same frozen files and real
Skill roots were used for both CLI versions, with separate local state.
That isolated inventory contained 268 Skills and 517 Placements.

The reference reader streamed every frozen JSONL record through the final
structured-event parser and matched exact, uniquely identified entrypoint paths.
This reference covers recognized literal read/invocation requests; it does not
prove successful tool execution, task completion, or recognition of every format.
The previously reported 43 used Skills was not assumed to be a verified denominator.

| Agent | v1.9.0 sampled Skills | v1.9.1 sampled Skills | Full-stream reference |
| --- | ---: | ---: | ---: |
| Codex | 0 | 11 | 19 |
| Pi | 0 | 6 | 6 |
| Claude Code | 0 | 0 | 0 |
| Distinct across these Agents | 0 | 16 | 23 |

Replaying the old tail selection with the final parser also found zero, isolating
sampling coverage from parser changes. All 16 newly sampled identities occur in
the full-stream reference. Observed read events had median file offset 40%.
The bounded sample reaches 16/23 of this reference; the remaining seven are
unknown to the sample, not unused Skills. Zero Claude observations establishes
no positive evidence in this corpus, not that Claude Skills were never used.

Limits remain explicit: 250 recent files per root, 512 MiB read per Agent,
64 MiB per file, 16 MiB fair parsing/extraction budget per Agent and 500,000
inspected lines. Unread regions, prefilter skips and parse quotas retain typed
partial-coverage facts. JSONL record copies are capped before allocation across
all windows at the remaining line budget plus one boundary record; a million
empty-line regression verifies this bound. No usage percentage claims a complete
denominator.

## Cross-language retrieval

Both CLIs used the same refreshed real-home Snapshot for **21 Chinese tasks
with English hints and seven generic/nonexistent-capability negatives**.
No case was scored while its Snapshot was stale. A live plugin-cache root
disappeared during an earlier trial; Find correctly refused that trial, and a
read-only Scan refreshed the reference before scoring.

| Outcome | v1.9.0 | v1.9.1 |
| --- | ---: | ---: |
| Positive direct verified loads | 12/21 | 19/21 |
| Negative automatic loads refused | 1/7 | 7/7 |
| Remaining positive same-name ambiguities | 2 | 2 |

The two v1.9.1 ambiguities retained both variants. Selecting one returned
identity with `--variant-skill-id` and `--require-snapshot` completed both
verified reads. Thus all 21 positive cases reached the expected capability,
while two deliberately required explicit selection. This is a bounded test set,
not a general retrieval-accuracy claim.

Examples include `frontend design`, `technical article writing`,
`GitHub pull request workflow`, `Chinese humanizer`, `domain model glossary ADR`
and `install Codex curated skill`. Negatives include `use xxx nonexistent skill`,
`agent ai tool skill workflow`, `skill` and `writer`.

## Performance, refresh and planning

Optimized builds were compared; timings include process startup and local state.
“New state” means no existing Snapshot, not a flushed operating-system disk cache.

| Workflow | Observed time |
| --- | --- |
| v1.9.0 new-state check on frozen input | 2.397 s |
| v1.9.1 new-state checks on frozen input | 3.595 / 2.773 / 2.779 s |
| Final allocation-bounded build, new-state frozen check | 3.953 s |
| v1.9.1 reused checks on frozen input | 95.81 / 90.93 / 94.95 ms |
| v1.9.1 final live-home check / reuse | 4.114 s / 169 ms |

At the same real inventory/log scale, an additional temporary Skill root
verified external addition, same-length edit and deletion. Check and MCP
refreshed automatically; the added Skill was retrieved and fully verified, and
the deleted Skill disappeared. A pinned read rejected the changed filesystem
while retaining the original Snapshot ID and creating no Scan. Subsequent
unpinned Find refreshed normally. MCP accepted blank lines and null parameters,
exited cleanly at EOF and emitted no stderr. Receipt count remained zero.

Both real-home human planning entrances returned Ready previews without Apply:

- Hermes Finding preview: default exposure **236 → 209**, 27 fewer placements.
- Shared adoption preview: default exposure **236 → 173**, 48 hosted packages.
  Its Codex Core budget of 20 contained **11 forced + 8 target-Agent usage +
  1 cross-Agent usage**, with **zero alphabetical fallback selections**.

These are proposed counts, not changes made to the machine. Forced and
cross-Agent provenance, partial usage coverage and fallback review language
remain visible. No real Agent or existing user-state mutations were executed.

## Validation and retention

`bash scripts/ci-full-check.sh` passed with Rust 1.85: formatting, strict Clippy,
**746 Rust tests passed**, one opt-in scale test ignored, **152 Node harness
tests** and **three npm packaging tests**, plus all three surface checks.
This records local macOS validation; it does not claim new remote cross-platform
CI or a v1.9.1 registry publication.

Additional temporary integration tests cover precise/generic aliases, task/hint
protection, bounded blocked candidates, ambiguity, native session formats,
sampling fairness and giant records, added/removed/edited roots and packages,
plugins and source links, pinned freshness, project hints, human Plan entrances,
non-TTY refusal and bounded Undo followed by verified restoration.

Private aggregate evidence, runner scripts and the optimized candidate are kept
under ignored `dist/v1.9.1-dogfood/`. Raw frozen session clones and Cargo build
artifacts are removed after validation. None of these local data artifacts are
part of a commit, upload or release.
