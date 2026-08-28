# SkillRoster 1.0 Acceptance

This document separates deterministic, synthetic acceptance from release checks that require a person or a real operating system. Fixtures contain no user Skills, sessions, prompts, credentials, or governance state.

## Automated acceptance

Run the core suite with:

```bash
cargo test --test acceptance
```

The suite proves:

- all eight direct adapters discover their independent filesystem fixture;
- each adapter conservatively normalizes Exposed, Matched, Loaded, Applied, and Outcome evidence, while prose-only mentions create no observed stage;
- the maintained 59-task routing set, including Agent-hinted Chinese tasks, reaches 100% Top-3 recall;
- a public `plan`/`apply` moves non-Core Skills to On-demand, `find` still returns readable paths, and Receipt-bounded `undo` restores the original Agent tree;
- small (5 Skills), large (120 Skills), and cross-Agent (12 Skills) duplicate scenarios preserve counts, traceable Finding evidence, and the four report metrics;
- plain 60-, 80-, and 120-column reports retain their core fields, no-change statement, and no ANSI bytes.

Top-3 routing and task success are separate checks:

- **routing hit:** the expected Skill appears in the first three public `find` results;
- **task success:** after routing, the evaluator opens a returned `SKILL.md` path and verifies its deterministic `CAPABILITY:` contract.

The governed run uses the public `scan`, `report`, `plan`, `apply`, `find`, and `undo` commands. It marks seven of ten Skills On-demand, then repeats both checks against paths returned after the real Apply. This validates deterministic fixture capability, not whether an external model completed a natural-language task.

Current local result (2026-08-22): **59/59 Top-3 hits before governance and 59/59 after governance**. Re-run the test above for release evidence; this recorded result is not a substitute for CI.

## Executed three-arm value comparison

The acceptance test creates three isolated temporary homes from the same deterministic 120-Skill inventory. Results are calculated from actual Scans and Reports; no result fixture is loaded.

- **Unmanaged:** leaves 120 Codex placements plus 80 Claude Code copies untouched.
- **Careful manual:** moves canonical packages into `.agents_skills`, uses a declared scope budget to resolve 70 of 80 cross-Agent copies, creates 54 hard-linked Core placements, and records the 10 unresolved copies in `manual-roster.json`.
- **SkillRoster:** runs the public CLI, submits 200 Roster changes, applies the validated Plan, measures the resulting filesystem, then undoes the Receipt and compares the restored Agent tree byte-for-byte.

Current local result (2026-08-22):

| Approach | Default exposure | Duplicate placements left | Receipt-backed |
|---|---:|---:|---|
| Unmanaged | 200 | 80 | No |
| Careful manual | 64 | 10 | No |
| SkillRoster Apply | 36 | 0 | Yes; Undo restored 200/80 |

“Duplicate placements” counts additional same-content non-symlink placements across Agent roots; shared-library canonicals are not double-counted. The manual arm's 10 is verified by scanning the filesystem after its bounded procedure, not inserted as a result fixture. The run proves the 50% exposure-reduction gate, duplicate reduction, readable On-demand routing, and restoration on controlled data. It does **not** claim measured labor savings, token savings, production performance, or superiority over every external manager.

The [relevant-manager execution run](acceptance/manager-comparison-run.md)
separately installs the same synthetic Skills with Vercel `skills` 1.5.23,
then proves SkillRoster can scan, govern, route, and exactly restore that
manager-owned symlink layout.

## Executed local environment checks

On the reference macOS machine, a release build scanned 180 independent Skills
and 676 placements across all eight adapters in 3.37 seconds. It changed no
Agent files, left schema v8 recovery clear, and remained below the five-minute
gate. A real PTY run also verified:

- Apply and Undo show the Plan or Receipt impact before reading input;
- cancellation exits successfully, creates no Receipt, and changes no paths;
- progress starts only after confirmation;
- SIGINT exits 130 with cursor restoration when styled and a truthful static
  summary in `TERM=dumb`;
- confirmed Apply and Undo both verify and leave recovery clear.

The 1.6 read-only dogfood scan added only Skill roots from Codex plugins marked
`enabled = true` in local configuration. It found 193 independent Skills and
689 placements while leaving default Agent-directory exposure at 548. Browser,
presentation, and spreadsheet tasks ranked their current plugin Skills first;
the terminal and JSON results identified each plugin provider and its read-only
governance boundary. Disabled plugins stayed absent, ambiguous cache versions
failed closed, and exact-duplicate planning refused provider-managed placements.

## Agent-led retrieval precision evidence

The 1.7 pre-release dogfood started from the public 1.6 behavior reported in
[Issue #31](https://github.com/tt-a1i/skillroster/issues/31), then rescanned the
same reference macOS home into fresh isolated state. It again found 193 Skills,
689 placements, and eight Agents with `files_changed=false`. The source build:

- ranked the dedicated browser, presentation, standalone-spreadsheet,
  live-Excel, session-mining, and cross-Agent Skill-management capabilities
  first for seven English tasks and five Chinese tasks with Agent-authored
  English hints;
- returned no more than three results for each of those 12 decisions instead
  of filling a ten-result response with incidental body-token matches;
- read folded YAML descriptions such as the real `agent-skills-manager`
  metadata, which moved that capability from rank four to rank one;
- reduced an unhinted CJK miss to one actionable warning while preserving the
  original task and requiring no cloud model or embedded translation table.

The maintained routing fixture now contains 59 English and cross-language
cases. All 59 route within Top-3, all 12 surface-disambiguation cases require
the dedicated capability at rank one, and the Apply/Undo governance loop must
preserve those results.

## Agent-led first-report performance evidence

The 1.7.1 dogfood run used a fresh isolated state directory against the same
reference home: eight Agents, 193 independent Skills, and 689 placements. An
unoptimized 1.7.0 build took 34.26 seconds to produce the first 3,237-byte
`report --summary --json`; the cached response took 0.04 seconds. The official
optimized 1.7.0 binary produced the complete 626,759-byte report in 4.85
seconds. Timing the unoptimized build showed semantic-overlap analysis consumed
34.59 seconds while all other report analysis completed in under 8
milliseconds.

The analyzer now tokenizes each complete Skill search document once and reuses
those vocabularies for pair comparison. On the unchanged real Snapshot the
unoptimized first report fell to 2.22 seconds and the optimized 1.7.1 build
produced the complete report in 0.84 seconds. Normalized complete reports from
the official 1.7.0 binary and the 1.7.1 build were byte-identical after removing
random Run, Report, and Finding IDs. A 193-Skill core regression that previously
took 75.32 seconds now completes in under one second and enforces a five-second
upper bound in the unoptimized test profile. These measurements describe the
recorded reference run, not a universal machine-performance guarantee.

## Agent session-evidence dogfood

The 1.8 dogfood run reproduced a usage-evidence failure on the same reference
home. Five of eight supported Agent session roots existed, but v1.7.1 reported
zero complete denominators and observed use for only one Agent. Discovery found
as many as 1,856 files in one root, while the scanner sorted only a traversal
prefix and skipped a newest file when it exceeded the four-megabyte Agent
budget. Claude Code consequently had a present session root but zero observed
bytes.

The scanner now discovers before selecting the newest bounded set, samples
complete-line tails from large JSONL files and complete nested objects from
large monolithic JSON tails, and spreads the byte budget across recent sessions.
Representative nested Claude Code, Pi, Cursor, and Hermes tool-call fixtures
bind Skill names or `SKILL.md` paths while tool declarations and Skill catalog
text remain non-events. Complete JSON session dumps are parsed as one structured
record when they fit the per-file budget.

The repaired real Scan found the same 193 Skills and 689 placements with
`files_changed=false`. It reported five roots present, five sampled, five
limited, three missing, zero inaccessible, and zero complete denominators.
Observed-use coverage increased from one Agent to two, and recent structured
`Loaded` evidence increased from 8 to 61 observed events across 56 Evidence
records, including 53 Cursor records. The result still emits no unsupported
usage percentage: bounded samples support positive event counts, while absence
remains unknown. Scan output reports discovered, observed, partially observed,
skipped, and discovery-truncation facts per Agent; no raw conversation text is
persisted.

## Release-candidate platform evidence

The 2026-08-22 release gates use actual hosted operating systems and the packaged
release binaries, always against synthetic temporary homes:

- [four-platform CI](https://github.com/tt-a1i/skillroster/actions/runs/32548259913)
  passed Rust 1.85 formatting, Clippy, and 79 core/high-risk tests on Linux x86_64,
  Windows x86_64, macOS arm64, and macOS x86_64;
- [release candidate 8](https://github.com/tt-a1i/skillroster/actions/runs/32548259787)
  built all four archives and ran `--version`, `--help`, Scan, Setup preview,
  Apply verification, Receipt-bounded Undo, and recovery-clear Status on every
  corresponding operating system;
- all four candidate checksums were downloaded and independently verified; each
  archive contained only the binary and README, and the macOS arm64 archive
  passed the same governance smoke after extraction;
- `cargo install --locked --path .` into an isolated prefix passed version and
  governance smoke checks.

WSL is tested as Linux in the release workflow by importing a checksum-pinned
Canonical Ubuntu 24.04 image into WSL2 on the Windows runner, then executing the
packaged Linux binary through the same governance loop. The final successful
candidate and official tag run are the release record; a failed or skipped WSL
job does not count as support.

## Human and Agent presentation review

A human review of plain first viewports found the intended hierarchy intact:

- the eight-Agent fixture at 60 and 120 columns showed the four key metrics,
  top Findings, category totals, read-only boundary, and next action without
  wrapping or ANSI noise;
- the real 180-Skill/676-placement environment at 80 columns surfaced 548
  default exposures and the escaping-link and exact-duplicate Findings in the
  first viewport, with no Agent-file changes;
- automated 60/80/120-column, `NO_COLOR`, `TERM=dumb`, non-TTY, CJK-width, and
  styled/plain semantic-equivalence checks cover the remaining repeatable cases;
- the PTY acceptance above covers visible impact, confirmation, cancellation,
  SIGINT, progress timing, verification, Receipt, Undo, and recovery states.

In this Agent-led release run, the Agent consumed only JSON, inspected the Setup
Plan preview and mutation boundary, used the user's explicit execution
authorization for the synthetic scope, then verified Apply, Receipt, Undo, and
recovery-clear Status. The bootstrap Skill separately requires an explicit user
confirmation before any real Apply or Undo and never asks per-file questions.

## Agent-led Finding drilldown evidence

Before the 1.3 release, the Agent reused one immutable read-only Snapshot from
the reference macOS home: 180 independent Skills, 676 placements, 548 default
exposures, and 189 Findings. It changed no Agent files. On that same Snapshot:

- the unsafe-link Finding response fell from 24,817 to 10,410 bytes (58.1%) in
  default compact mode while retaining all 16 paged Evidence items;
- an exact-duplicate Finding fell from 11,814 to 6,807 bytes (42.4%) while
  retaining seven traceable Evidence items and its planning action;
- `--full` preserved the complete IDs, placements, and Evidence records on
  explicit request;
- unsafe links no longer suggest a generic Plan. They return a typed trust
  decision, observed targets, and a repeatable `--source-root` rescan template;
- plain 60-, 80-, and 120-column Finding views preserve the issue, counts,
  Evidence paths, and next decision without exceeding the requested width.

## Agent-led Finding enumeration evidence

The 1.4 discovery run started with the public 1.3.0 macOS arm64 binary and a
fresh isolated state directory. Its read-only Scan found the same 180 Skills,
676 placements, 548 default exposures, and 189 Findings in 9.41 seconds, with
no Agent-file changes. The Agent then measured a 3,179-byte three-Finding
summary against a 645,511-byte exhaustive report and confirmed there was no
bounded enumeration path.

On the same immutable Snapshot, the new paged interface returned 20 complete
Finding summaries in 15,590 bytes. A `usage` category filter selected exactly
two Findings; combined `overlap` and `medium` filters selected 125. The summary
now suggests `list_findings`, a partial page preserves its filters in
`list_more_findings`, and the exhaustive diagnostic export suggests no generic
Plan. Selected IDs still lead to compact Evidence drilldown. Automated tests
cover pagination, filtering, action argv, invalid flag combinations, and plain
60-, 80-, and 120-column rendering.

## Agent-led confirmed-source normalization dogfood

The 1.8.5 pre-release dogfood used a fresh isolated state directory against the
real eight-Agent home. Before source confirmation, Scan reported 244
independent Skills, 740 placements, 548 default exposures, and 16 escaping-link
placements without reading those targets. The trust-resolution response listed
four observed targets. Two were a symlink alias and physical path for the same
source; following the response literally caused the pre-fix inventory to add
four source placements instead of three.

Confirmed source roots are now resolved only after the caller explicitly
supplies `--source-root`. The scanner preserves each confirmed alias for trust
containment but inventories its canonical physical directory once. The same
real rescan completed in 10.51 seconds with 231 independent Skills, 743
placements, unchanged default exposure at 548, no unsafe-link warning, and no
escaping-link Finding. The lower Skill count reflects formerly unread aliases
resolving to shared content rather than removal or archive activity.

On that Snapshot, `setup` prepared but did not apply a six-operation Plan for
five logical bootstrap targets across three physical roots. The Chinese
cross-Agent Skill-management task returned `agent-skills-manager` at rank one.
The exact-duplicate Finding retained six logical placements, three physical
canonical candidates, and a bounded five-operation managed-Library Plan. No
real Agent file was changed; Apply/Undo remained confined to automated
temporary-home acceptance tests.

## Agent-led governance-routing dogfood

The 1.8.6 pre-release run added the bundled Bootstrap as an explicitly trusted,
non-exposed source beside the real 231-Skill inventory. This represented the
searchable state after Setup without applying the pending real-home Setup Plan.
In v1.8.5, a request to inventory installed Skills and analyze duplicates or
uncertain usage ranked the shared-symlink `agent-skills-manager` first and
`skillroster` third. The former cannot produce governance Evidence, Plans, or
Receipts.

The failure was not an `ego-browser` result previously seen in one query; that
candidate did not reproduce across the routing matrix. The stable defect was
lexical: common words such as `and` and `for` contributed positive score and
could combine with `skill` to trigger a false two-token exclusion penalty for
`Not for ...` clauses. A second independent replay showed that concatenating a
CJK task and English hint also discarded exact phrase and declared-trigger
evidence. Search scoring now removes a bounded English stopword set before
field overlap and exclusion evaluation, keeps task and hint phrase boundaries,
and accepts standard-compatible string-valued
`metadata.skillroster-routing-triggers`. The Bootstrap
pointer and routing metadata state the governance boundary against installation
and shared-directory management.

Fresh real-home rescans then ranked `skillroster` first for inventory/usage
evidence, Core/On-demand Roster governance, and approved Apply/Undo requests;
shared installation and symlink synchronization still ranked
`agent-skills-manager` first. The exact original `duplicate unused` query and
each inventory, duplicate, usage-evidence, Roster, Plan, Apply, Receipt, and
Undo branch passed independently. The maintained 86-task English/CJK routing set
passed before governance, after Apply, and after Undo. Inspection and routing
changed no real Agent files.

The v1.8.11 CJK routing dogfood reused a fresh isolated Snapshot of the real
home with the bundled Bootstrap as a trusted non-exposed source: 232 independent
Skills and 744 placements. Before the change, a near-verbatim Chinese
description routed to `humanizer-zh`, while the natural paraphrases
`把中文改自然一点` and `编辑中文让它更像人写的` returned no matches. CJK-aware
local scoring then ranked `humanizer-zh` first for both, exposed the contributing
description/body bigrams, and kept the previously sampled real task routes in
Top-3. A bounded Chinese stop-unit list removed generic `帮我` / `看看` noise;
the maintained 86-task set passed before governance, after Apply, and after
Undo. All real-home Find operations were read-only.

The same run followed the `humanizer-zh` variant warning into its Layout
Finding. Before the fix, full detail contained two Skill IDs and two digests but
zero placements or readable paths, while still suggesting a generic Plan. The
new immutable Report identified five real placements: one shared-content
variant across Codex, Claude Code, Pi, and the shared source, plus one divergent
Hermes variant. Compact and full detail kept each digest, Agent set, root, and
path together, returned `choose_same_name_variant`, and offered no Plan before
the canonical-content decision. Human output preserved the same facts at 60,
80, and 120 columns without displaying null paths or changing Agent files.

The v1.8.12 Agent-hint dogfood used another fresh isolated state store against
the same 232-Skill, 744-placement real inventory. Twenty previously unseen
Chinese tasks tested ordinary Find, followed by twelve Agent-authored English
capability hints where the target metadata was English or the native result was
uncertain. The pre-fix combined-token ranking moved `humanizer-zh` from rank two
for the original task to rank six after the appropriate hint `humanize Chinese
writing remove AI tone`, outside the requested Top-5. Task/hint reciprocal-rank
fusion restored it to rank two while independently recovering
`computer-history`, `Spreadsheets`, `control-chrome`, `code-review`,
`hi-im-reader`, `documents`, and `imagegen` in Top-2. The run also retained
honest misses: a generic `executable specification` hint did not contain the
`spec` vocabulary needed to discover `to-spec`, while a refined `write
technical spec` hint ranked it second. The maintained 86-task routing set and
the task-preservation regression passed without changing Agent files.

The same dogfood reproduced an Agent-pipeline failure by closing the consumer
before a full Finding JSON response was written. v1.8.11 exited 101 and printed
a raw Rust Broken-pipe panic. The fallible stdout boundary now treats only
`BrokenPipe` as quiet consumer completion; other output errors remain failures.
A process-level regression closes the stdout reader before write, and the real
large-response replay exits zero with empty stderr. Connected JSON responses
remain one complete versioned document.

The v1.8.13 usage-evidence dogfood started from a freshly rebuilt release
binary after rejecting a stale v1.8.6 build artifact as invalid evidence. The
current read-only real-home Scan again found 232 independent Skills, 744
placements, 548 default exposure, and 169 Findings with `files_changed=false`.
The `Five-stage usage evidence` Finding reported observed Loaded events, but
both compact and full drill-down exposed only opaque Skill IDs and serialized
Claude Code as the internal `claude_code` enum spelling. Agent callers could
not directly answer which Skills were observed without an out-of-band lookup
and identifier rewrite. Usage Evidence now carries `skill_name` and the
canonical public Agent ID while preserving stage, quality, count, timestamps,
and the source-path digest. An eight-Agent fixture verifies the same facts in
compact and full JSON; no real Agent file was changed.

The v1.8.14 plain-usage dogfood used a fresh isolated state store and release
build against the current real home. The read-only Scan found 244 independent
Skills, 740 placements, 548 default exposure, and 174 Findings. The usage
Finding reported 53 observed Loaded events across bounded local evidence. Its
60-, 80-, and 120-column views now separate all five stages, aggregate session
coverage, and five recent named Agent/Skill signals; Exposed is explicitly
counted in placements rather than mislabeled as events. Real output contained
no session paths or ANSI bytes and stayed within every target width. Exposed
inventory omits session timestamps; event stages retain their observed times.
Dogfood also exposed duplicate Agent/stable-Skill-ID/stage preview rows from
separate evidence sources; the final overview groups those rows, sums their
event counts, retains the strongest evidence quality and latest timestamp, and
produced five unique preview keys. Compact and full JSON retained the same
bounded overview with `files_changed=false`; no Apply was run and no Agent file
changed.

The v1.8.15 Agent-hint dogfood reused the official v1.8.14 real-home Snapshot:
244 independent Skills and 740 placements. For the Chinese task `把产品想法压力
测试成明确规格`, the positive English capability hint found `grilling` at
augmented rank two, but the previous large reciprocal-rank offset still placed
the unrelated `x-post-writer` first from weak task-rank-six and
augmented-rank-fourteen overlap. The small-pool fusion now preserves rank
discrimination: `plan`, `grilling`, and `product-business-analysis` form the
Top-3 and the unrelated writer is removed from the bounded result. Three other
real read-only probes ranked `code-review`, `computer-history`, and
`analyze-data-quality` first; the maintained routing baseline and the strong
native-task preservation regression remained green. Every Find response kept
both channel ranks and `files_changed=false`.

The v1.8.16 source-root dogfood started from the official v1.8.15 binary and
three explicitly reviewed local source directories. Passing the final canonical
directory for one shared Skill source left five high-severity escaping-link
placements, while passing its symlink alias cleared them; both Snapshots stored
the same canonical root facts. The scanner now makes trust decisions from the
resolved destination. Fresh read-only Scans using the canonical path and the
alias produced identical results: 231 independent Skills, 743 placements, 548
default exposure, 169 Findings, no escaping-link Finding, and
`files_changed=false`; the core regression additionally covers supplying both
forms together. Direct escapes, indirect escapes, and unresolved links remain
unread; no Apply was run and no Agent file changed.

The v1.8.17 Core-selection dogfood reused that real-home Snapshot and prepared
isolated immutable Plans with Core budgets of 10, 25, and 50. All four
oversized Agents were fallback-dominated: the budget-10 Plan selected 40 Core
Skills with one observed-loaded signal and 39 stable fallbacks. The previous
summary exposed only aggregate counts and the 361 KiB full Plan exposed opaque
Roster IDs, so an Agent could not review which Skills remained Core without an
unrelated lookup. The new bounded summary is 9.4 KiB and exposes five named
selections with reasons per Agent; `plan --show` persists all 40 exact named
selections and reasons. The ordinary 60-, 80-, and 120-column views keep one
name, reason, and remaining count visible for Codex, Claude, Pi, and Hermes.
The new binary also read a v1.8.16 stored Plan without complete-selection data
and preserved its aggregate evidence. Every inspection and Plan reported
`files_changed=false`; no Apply was run and no Agent file changed.

The v1.8.18 same-Snapshot A/B reused the immutable official v1.8.17 Snapshot
`scan_000000000000407f18ce4a370d209368` and large-Roster Finding
`finding_000000000000414818ce4a39aa046681`; it did not rescan. The stored old
budget-10 Plan selected four positive-signal Core entries and 36 stable
fallbacks. The new algorithm selected the same 40-entry budget as four
target-Agent entries, 36 explicitly attributed cross-Agent entries, and zero
fallbacks. Exact-ID comparison replaced 6, 10, 10, and 10 fallbacks for Codex,
Claude, Pi, and Hermes respectively. Both Plans bind the same Snapshot and
Finding and report `files_changed=false`; no Apply was run. The new Plan stays
typed `cross_agent_dominated_core_selection` with `review_required: true`.

A separate current-home sensitivity run used four explicitly reviewed
canonical source roots: 231 independent Skills, 871 placements including those
read-only sources, 548 default exposures, and five sampled Agent session roots.
Its budget-10 Plan selected two target-Agent, 14 cross-Agent, and 24 fallback
Core entries, so fallback still dominated all four affected Rosters. Every
cross-Agent selection persisted its source Agent IDs and
`evidence_scope: cross_agent`; no name-based transfer occurred. The Plan reduced
proposed exposure from 548 to 82, reported zero canonical deletions, and kept
`files_changed=false`. No Apply was run. The ordinary 60-, 80-, and 120-column
views preserved `target Agent`, `cross-Agent`, and `elsewhere loaded` while
shortening long Skill names first.

The v1.8.19 Agent decision-loop dogfood used an isolated state directory and
four explicitly reviewed source roots against the current real home. Its
read-only Snapshot contained 236 independent Skills, 763 placements, 564
default exposures, and 173 Findings. The Usage Finding first page now placed
all eight coverage records and 35 observed non-Exposure rows before inferred
Exposure, omitted the invalid Plan action, and replayed its full-detail next
page with the same home and state context. The suggested unfiltered Findings
page improved from three titles across three categories to all eleven rollup
titles across five categories; nine pages still enumerated exactly 173 unique
Finding IDs.

The same Snapshot exposed eight large-Roster source-dependency pairs. The
decision view grouped them into the two actual Skills, `repo-learning` and
`ego-browser`, named all four affected Agents, and returned the two dependent
non-Agent source paths. It exposed confirmation-gated Core protection only
after the production per-Agent Recommendation constraints accepted the
complete identity set; a 100-dependent-Skill plus bootstrap regression proves
an over-budget set is typed unavailable and has no Plan template. No choice was
made, no Apply ran, and no real Agent file changed.

The v1.8.20 fact-and-routing dogfood reused a retained read-only Snapshot of
the real home: 8 supported Agents, 180 independent Skills, 676 placements, and
548 default exposures. Structural inventory coverage and session evidence
coverage remained separate facts instead of allowing a present Skill root to
imply observable usage. One exact-duplicate Plan now reported physical sources
3→1, placements 6→6, default exposure 5→5, and two relinks; this rejected the
earlier implication that consolidation also reduced exposure. With three Ready
Plans retained for the same current Snapshot, Status selected the first item
from its existing bounded lifecycle ordering and returned a context-preserving,
read-only `plan --show` action instead of suggesting a Scan that would stale
those Plans. The 40-column terminal view wrapped the opaque Plan ID and avoided
printing a context-free command. No Plan was applied and no Agent file changed.

## Real-Agent cold-routing canary

The v1.8.20 pre-release cold-routing canary held the Pi harness, model
(`seal/deepseek-v4-flash-0731-baidu`), natural Chinese task, isolated Library,
CLI Snapshot, permissions, and oracle constant. The target Skill had no default
Agent exposure and deterministic CLI preflight ranked it first. With the
governance-first Bootstrap, the Agent classified the task as unrelated and
never called Find. After only the general route trigger and minimal
`find -> read returned SKILL.md` steps were moved to the front, a fresh session
called Find, read the exact returned cold path, and produced the oracle result.

The harness allowed only SkillRoster Find plus reads of the isolated fixture and
the repository Bootstrap. The observed transcript stayed within those paths,
and the filesystem sandbox denied writes under the real home. This is one
canary, not the paired multi-task success claim tracked by Issue #129. It
identifies the Bootstrap entry contract as the first defect and does not justify
a new semantic engine or a routing-recall claim.

The frozen inputs, hashes, event chain, and safety limitation are recorded in
[the Pi cold-routing canary ledger](acceptance/cold-routing-canary-v1.8.20.md).

The subsequent [sealed Pi paired pilot](acceptance/pi-cold-routing-pilot-v2.md)
ran all eight Core and On-demand arms. Cold retrieval and target loading passed
4/4, while the overall task-success gate failed because two Core controls also
failed. The result is preserved as a failed holdout rather than tuned or rerun.

## Evidence boundary

These checks establish deterministic routing and safe local governance; they do
not claim model quality, token or labor savings, production performance, or
access to arbitrary future Agent log formats. Final support additionally
requires the official release tag workflow, published artifact checksums, and
the installation checks recorded in the Release notes.
