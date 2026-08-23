# SkillRoster Product Specification

## 1. Product promise

SkillRoster lets a person ask an AI Agent to inspect and govern local Agent Skills. The Agent invokes one bootstrap Skill, calls a deterministic Rust CLI, explains evidence-backed problems, and executes only a user-approved, reversible Plan.

> One library. The right roster for every agent.

The first complete release must answer four questions reliably:

1. What Skills exist across the supported Agents, and where did they come from?
2. Which Skills are exposed, duplicated, stale, broken, risky, or unsupported by usage evidence?
3. Which Skills should be Core, On-demand, Explicit-only, or Archived for each Agent?
4. How can the approved organization be applied and undone without losing user-owned files?

## 2. Supported environment

The first complete release directly supports:

- Codex
- Claude Code
- Pi
- OpenCode
- Hermes
- Cursor
- Gemini CLI
- GitHub Copilot

Supported platforms are macOS, Linux, and Windows. WSL is treated as Linux. A platform or Agent is not declared supported until its adapter passes fixtures and a real-environment acceptance run.

SkillRoster scans only known directories for these eight Agents, Skill roots from Codex plugins proven by either explicit local enablement or a valid local remote-plugin install marker, plus paths explicitly provided by the user. It never crawls the entire home directory or treats an arbitrary plugin cache as installed. Explicitly disabled plugins remain excluded. Discovered Codex plugin Skills are Observed, searchable, and provider-managed read-only: they cannot become mutation targets or canonical Library sources. Invalid install markers and ambiguous cached plugin versions fail closed. Every Scan reports included, excluded, missing, and inaccessible roots. `--root AGENT=PATH` is an Agent placement root and contributes to exposure; `--source-root PATH` is an approved canonical source with no Agent identity and no default exposure. Source trust is explicit and scoped to the resolved canonical directory; a path and a symlink alias to that same directory have identical trust semantics.

## 3. Primary interaction

```text
Person
  -> supported Agent
      -> skillroster bootstrap Skill
          -> skillroster CLI --json
              -> local filesystem, sessions, and SQLite
          <- facts, Findings, Evidence, Plans, Receipts
      <- concise diagnosis and confirmation request
```

### 3.1 Agent-first responsibility boundary

The person delegates intent in natural language and should not need to learn or compose a CLI workflow. The calling Agent owns intent interpretation, semantic comparison, prioritization, recommendation, and natural-language presentation. It translates the person's request into structured SkillRoster calls, asks only for genuine preference or authorization decisions, and explains the resulting Evidence and Plan.

The CLI owns deterministic discovery, normalization, hashing, indexing, statistics, validation, mutation, and recovery. It returns bounded, decision-complete facts and typed options so the Agent does not need to rediscover filesystem state or scrape prose. It validates the Agent's structured choices but does not replace model reasoning with embedded intent classifiers, semantic policy trees, or a second prompt framework. Human-readable terminal output supports audit, diagnosis, and fallback use; versioned JSON is the primary Agent integration contract.

Use this placement test for every capability: local truth, stable identity, reproducibility, safety, and reversible execution belong in the CLI; context-sensitive meaning and open-ended judgment belong in the Agent. The person retains preference and material-change authority. The binary contains no model API or second Agent.

## 4. Capability requirements

### 4.1 Inventory and identity

A read-only Scan discovers Skill roots, `SKILL.md` entry points, source metadata, links, configuration exposure, and supported local session sources. It normalizes paths without following links outside approved roots silently.

Skill identity uses this precedence:

1. declared source plus version or revision;
2. normalized content hash;
3. name and semantic similarity as candidate evidence only.

Ambiguous Skills remain separate. SkillRoster never merges based only on a matching name or Agent interpretation.

### 4.2 Analysis

`report` covers seven categories:

- **Inventory:** counts, locations, ownership, sources, and versions.
- **Layout:** copies, links, broken references, escaping links, and collisions.
- **Exposure:** which Agent and scope can see each Skill.
- **Usage:** evidence windows and the Exposed, Matched, Loaded, Applied, and Outcome stages.
- **Overlap:** exact duplicates and evidence for likely semantic overlap.
- **Routing:** default exposure, searchability, and missing task-to-Skill routes.
- **Lifecycle:** update drift, stale sources, management state, and archive candidates.

Usage evidence is labeled `observed`, `inferred`, or `unknown`. Missing evidence never becomes proof that a Skill is useless. Raw prompts and responses are parsed read-only in place and are not copied into SkillRoster storage. Recent session selection happens after bounded discovery; large active JSONL files contribute bounded complete-line tails, large monolithic JSON files contribute bounded complete nested objects from their tails, and the fixed byte budget is spread across multiple recent sessions. Reports distinguish missing, inaccessible, sampled-but-limited, and complete observable session roots. A usage percentage is emitted only when the observable-session denominator is complete; bounded samples support observed event counts and stages, not an “unused” claim.

### 4.3 Library and Rosters

The Library is logical and does not require moving every Skill. Each discovered Skill begins as Observed. After approval it may become:

- **Managed:** SkillRoster manages exposure, links, or configuration while the canonical files remain in place.
- **Hosted:** canonical files are explicitly migrated into `~/.skillroster/library`.

Each Agent Roster classifies Skills as Core, On-demand, Explicit-only, or Archived. On-demand Skills remain searchable without being linked into every default Skill directory. For large-Roster Core selection, protected, declared, and bootstrap Skills come first; target-Agent usage outranks usage from another local Agent for the exact same stable Skill identity, which outranks stable fallback order. Cross-Agent evidence never transfers by name, path, or fuzzy similarity and always identifies its source Agent. `find` returns paths and Evidence so the Agent can read the selected `SKILL.md` directly; temporary activation links are unnecessary.

### 4.4 Planning, mutation, and recovery

Read-only commands never mutate Agent configuration or Skill contents. Agent recommendations enter the CLI through `plan --stdin` as structured target states. Most requests cite current Scan, Skill, and Evidence IDs directly. Exact-duplicate consolidation instead cites the immutable Finding plus the Agent's two semantic choices; the CLI derives the bound Snapshot, Evidence, Skill, and complete placement set. Large-Roster governance cites its immutable Finding, a per-Agent Core budget, and optional protected Skills. The CLI validates the complete oversized scope and derives every Core or On-demand Roster change; it never infers Explicit-only or Archived from missing usage evidence. A smaller custom Core budget that introduces preservation blockers fails closed: `plan --stdin` returns bounded typed blocker evidence with the affected Agent, stable Skill identity and name, and the narrowest `--source-root` directories already observed as escaping-link targets. It sorts and deduplicates exact targets and removes a descendant only when its ancestor was itself observed; sibling targets never synthesize a broader parent. Ordinary JSON lists at most ten blocked Skills and ten source roots; the human terminal lists at most five of each. When the complete set is larger, `error.details.detail.path` is a SkillRoster-owned JSON file with every omitted identity, directory, and the complete repeatable `--source-root` argv. It does not read an untrusted target, imply trust, or create a partial Plan.

The CLI validates targets, ownership, conflicts, and current fingerprints, then stores an immutable Plan. `apply <plan-id>` executes only the approved scope. Cross-filesystem operations use a journal and compensating steps; the CLI never claims atomicity it cannot provide.

Logical placements reached through symlinked Agent roots may resolve to one
physical filesystem object. A Plan operates on that object at most once while
retaining every logical Agent/placement impact fact. If the linked logical
placements request incompatible Core and non-Core exposure, planning fails
closed with a typed conflict. No Ready Plan may contain duplicate destructive
sources or operation targets. Before deriving operations, planning revalidates
each captured physical source against the current logical entrypoint and stores
those logical-entrypoint-to-physical-source bindings in the immutable Plan.
Apply revalidates the complete binding set before entering Applying. A
physical object shared with any provider-managed placement remains read-only,
and a non-Agent source link that would be broken blocks the Plan. These
blockers expose stable reasons, IDs, paths, and next actions in `error.details`;
Agent callers never need to parse the human message.

Apply fails closed when paths, fingerprints, links, or configuration have drifted. Every successful mutation writes a Receipt. `undo <receipt-id>` is bounded to that Receipt and refuses ambiguous restoration. Canonical deletion is outside normal Apply, requires separate confirmation, and should prefer recoverable archive.

### 4.5 Source updates

SkillRoster records source, revision, and content hash when available. A source-update request submitted through `plan --stdin` first produces a diff and Plan; there is no silent update path. Local modifications stop automatic replacement and require an explicit choice to retain local content, adopt upstream content, or preserve both.

## 5. CLI contract

The first complete command surface is:

```bash
skillroster scan [--json]
skillroster report [--summary | --full | --findings [--category <category>] [--severity <severity>] | --finding <id> [--full]] [--limit <n>] [--offset <n>] [--json]
skillroster find <task> [--hint <text>]... [--json]
skillroster plan --stdin [--json]
skillroster plan --show <plan-id> [--json]
skillroster apply <plan-id> [--json]
skillroster undo <receipt-id> [--json]
skillroster status [--json]
skillroster setup [--modified-choice retain-local|adopt-current] [--json]
```

`setup` detects supported Agents and returns a preview for installing or
upgrading the single bootstrap Skill. Exact official older copies are eligible
for automatic planning. Unknown content is treated as a local modification and
requires an explicit retain/adopt choice before any Plan is created. Links,
non-files, and unreadable targets are blocked. In JSON mode setup never prompts
or mutates; every write remains a normal Plan completed through Apply and
recoverable through Undo. Repeating an unchanged setup preview on the same
Snapshot reuses an equivalent Ready Plan instead of accumulating duplicate
pending Plans. Terminal Plans, newer Snapshots, and changed setup inputs are
never reused.

`find` preserves the user's original task and accepts repeatable Agent-authored
retrieval hints. Hints let the semantic caller supply a cross-language or
capability paraphrase while the CLI remains a deterministic local lexical
index. Without hints, Find uses one lexical channel. With hints, it ranks the
original task and the task-plus-hints expansion independently, then applies
deterministic reciprocal-rank fusion over a bounded candidate pool. The JSON
`ranking_strategy`, `task_channel_rank`, and `augmented_channel_rank` facts make
that decision inspectable. Rank position remains discriminating within these
small candidate pools: a high-ranked Agent hint match outranks weak lexical
overlap that merely appears in both channels, while the strongest original-task
match remains within the default top three when it has protectable evidence.
Smaller limits intentionally return only the corresponding stable prefix.
Hinted retrieval uses a fixed internal pool of up to 100 capabilities, matching
the public maximum limit. For the same Snapshot, task, and hints, changing
`--limit` only bounds the returned matches: each smaller result is a prefix of
a larger result. A hint can therefore surface English
metadata without letting its raw token count set one global cutoff that
discards a strong native-task result. Each task or hint also remains a separate phrase for
exact name, description, and declared-trigger evidence. The scanner reads
ordinary and folded YAML description scalars. Ranking uses
top-level `triggers` and the semicolon-separated string
`metadata.skillroster-routing-triggers` as the same declared retrieval
evidence. The latter must be explicitly quoted and remains valid under the
Agent Skills metadata string-value contract; non-string and nested forms are
ignored. Ranking normalizes conservative ASCII plurals and segments contiguous
Han text into overlapping two-character lexical units after removing a bounded
set of common Chinese stop-units. A query containing Han text expands scoring
only to the already-scanned, locally routable Skill set so natural CJK
paraphrases can reach CJK metadata even when SQLite FTS has no whole-token
candidate; Archived Skills remain excluded. Match reasons expose CJK
description and full-text bigram counts. Ranking also treats explicit use and
do-not-use description clauses as positive and exclusion evidence, and removes
the low-confidence tail relative to the strongest result. Same-name Skill
identities occupy one ranked capability result and are
reported as explicit variants rather than silently treated as equivalent.
For an ambiguous result, `variant_finding` binds the complete routable identity
set to a same-Snapshot divergent-content Finding and provides its read-only
drilldown. Without a compatible current Report it returns a typed reason and
the minimum read-only analysis action. A drifted routable variant requires a
new Scan before any Finding can be linked; stale Finding IDs are never returned.
The corresponding divergent-content Finding carries the affected placements
and a bounded `choose_same_name_variant` resolution that keeps each digest,
path, Agent, provider, root, and governability fact together. It offers no Plan
until the caller has compared the variants and chosen canonical content.
Variant details keep provider, governance, and path facts bound to one identity;
the response preserves the total count and marks bounded detail truncation.
Provider-managed results include their plugin identity and a non-governable
marker so Agents can route to them without proposing filesystem governance.

All JSON responses use a versioned envelope:

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "report",
  "run_id": "run_...",
  "result": {},
  "warnings": [],
  "error": null,
  "suggested_actions": []
}
```

IDs for Agents, Scans, reports, Skills, placements, Evidence, Findings, Plans, operations, and Receipts are opaque and stable within the local state store. Names and paths are never write-operation identities. Errors carry a stable code, human-readable message, retryability, and relevant IDs or paths. In JSON mode stdout contains exactly one JSON document and never an interactive prompt, progress bar, ANSI sequence, or log line. Terminal output is a polished human interface; Agents consume JSON rather than scraping styled text. The normative human-output, accessibility, progress, confirmation, and responsive-layout requirements live in [cli-ux-spec.md](cli-ux-spec.md).

Selector-free `report --json` returns the bounded three-Finding Summary view;
`--summary` is an explicit alias. The exhaustive diagnostic report requires
top-level `report --full --json`.

`report --finding ID --json` returns compact paged Evidence items by default.
Each item contains the Evidence ID, subject, path, quality, and decision facts;
it does not repeat complete affected-ID, placement, and Evidence collections.
`--full` explicitly requests those complete paged records. For escaping links,
the Finding returns a trust-confirmation resolution and observed link targets;
it must not advertise an automatic Plan before the source is confirmed.
Usage Evidence includes the human-readable `skill_name` beside the opaque
`skill_id`, and serializes `agent` with the same canonical public identifier
used by planning commands. Compact and full detail therefore support usage
summaries without a caller-side Skill lookup or Agent-name rewrite.
The usage Finding also exposes one bounded `usage_overview`: five typed stage
counts with explicit units, the complete aggregate session-coverage boundary,
and up to five recent high-signal named Skills. `Exposed` counts placements;
Matched, Loaded, Applied, and Outcome count events. The ordinary terminal view
renders this structure as separate stage, coverage, and observed-Skill blocks
at 60, 80, and 120 columns without printing session paths.

`plan --stdin --json` returns a bounded decision-complete summary: total change counts, operation groups, affected-scope counts with at most ten Skill IDs, before/after impact, risk, reversibility, and the immutable Plan ID. A Finding-derived Roster Plan also persists `selection_evidence`, including forced, target-Agent, cross-Agent, aggregate positive-signal, and stable-fallback Core counts plus at most five named Core selections per Agent. Every selection includes its reason, evidence scope, and source Agent IDs without session paths or raw content. When fallback or cross-Agent selection dominates any affected Agent, the Plan carries typed `uncertainty` with `review_required: true`; absence of usage evidence remains explicitly non-negative. It does not inline filesystem operations, complete Core selections, or complete large ID collections. `plan --show PLAN_ID --json` is the explicit full-detail path; it reads the stored immutable Plan, returns every named Core selection and reason, and never mutates files. Plans created before complete Core selections were persisted remain readable with their original aggregate evidence. A fail-closed `trusted_canonical_sources_required` error is also bounded: at most ten blocked Skills and ten source roots in the envelope, with `error.details.detail.path` as the independent retrieval path when either set is larger.

## 6. Agent presentation contract

The normative conversation flow, one-confirmation Apply behavior, state-dependent primary action, and bootstrap Skill rules live in [agent-experience-spec.md](agent-experience-spec.md).

The bootstrap Skill instructs an Agent to prepare a complete read-only Plan when Evidence is sufficient, then present:

1. a one-sentence diagnosis;
2. independent Skill count, placement count, default exposure, and observed-use count;
3. the three most important Findings plus compact rollups that state the complete scale of every Finding group without loading the exhaustive report;
4. prioritized recommendations with expected measurable impact;
5. uncertainties, evidence quality, and safety risks;
6. one primary next action and whether confirmation is required.

The wording and layout may adapt to the host Agent, language, and conversation. The facts, IDs, Evidence, and confirmation boundary may not be omitted. No HTML report or GUI is generated. Users can ask follow-ups such as “show the 27 duplicates” or “why is this a duplicate”; the Agent resolves them against the same report ID with `report --finding` rather than silently rescanning.

Every summary states whether files were changed. An applied summary also includes the Plan ID, changed-path count, verification status, and Receipt ID. A Ready Plan is presented with one conversational Apply action; after the user confirms once, the Agent executes the whole approved scope without per-operation prompts. SkillRoster does not invent an aggregate health score or unsupported token/performance savings.

The human terminal surface uses concise headers, aligned facts, semantic color, TTY-only progress, strong final summaries, and explicit confirmation. It supports `NO_COLOR`, non-TTY/plain output, narrow terminals, and Unicode fallback. It is not a full-screen TUI and never changes the Agent JSON contract.

## 7. Local data and privacy

SkillRoster uses one SQLite database at `~/.skillroster/skillroster.db`, including FTS5. It has no daemon, cache service, cloud backend, account, RBAC, or telemetry.

- Source sessions are read-only and remain in their original locations.
- Derived event summaries are retained for 180 days by default.
- Usage Observations have a stable identity across Scans. An unchanged rescan
  does not create another raw observation. A changed bounded source window is
  recorded as a new observation, not inferred as an additive event delta.
- Usage event windows use timestamps carried by the session record. Session
  file modification times describe coverage only and must not refresh old
  Skill events; when a record has no trustworthy timestamp, event time remains
  unknown. The local observation time may govern retention for that record but
  is never presented as the event time.
- Older usage is reduced to the maximum observed count per source and month.
  These maxima are conservative source-window facts, not cumulative totals.
- Pre-v9 monthly rows cannot be safely reconstructed by source. They are
  retained separately with `derivation=legacy_scan_aggregate` and must not be
  combined with source-month observations.
- Plans and Receipts remain until explicitly purged.
- Overflow source-confirmation details are versioned local artifacts retained until explicitly purged or local state is deleted.
- `status` exposes storage location, retention state, and retained source-confirmation artifact counts and bytes.
- `status.pending_plans` contains only actionable Ready Plans for the latest
  Snapshot plus any Applying or Recovery-required Plans. Ready Plans from older
  Snapshots remain inspectable history but are not presented as pending work.
  The total count remains exact while the returned list is capped and reports
  whether additional pending Plans were truncated.
- `status` suggests Scan only when no completed Snapshot exists. A healthy
  state with a Snapshot exposes its timestamp and leaves refresh judgment to
  the Agent instead of creating an unconditional Status-to-Scan loop.
- Users can inspect, export, purge, or delete retained local state and rebuild inventory by scanning again.

Reports may identify structural and provenance risks—unknown source, changed content, executable scripts, declaration mismatch, and escaping links—but must not claim malware detection or runtime safety.

## 8. Scope constraints

The complete first release deliberately contains:

- one Rust binary;
- one SQLite database;
- one bootstrap Skill;
- eight direct Agent adapters;
- core-logic and high-risk mutation tests.

It does not contain HTML reports, a GUI, an interactive or full-screen TUI, MCP, cloud services, accounts, telemetry, a daemon, a plugin SDK, generic Agent adapters, a workflow engine, a second Agent, or built-in model calls. Ponytail is a development experiment only and creates no product dependency or product-specific behavior.

An abstraction requires at least two real consumers. Otherwise, prefer direct code.

## 9. Completion and value gates

The product may progress through internal milestones, but `1.0` is complete only when the full loop passes:

- supported adapter fixtures achieve 100% expected discovery;
- Apply and Undo restore 100% of covered fixture states;
- no unapproved canonical deletion occurs;
- every Finding is traceable to paths and Evidence;
- on inventories over 100 Skills, a reviewed proposal can reduce default exposure by at least 50%;
- On-demand Skill routing reaches at least 95% Top-3 recall on the maintained evaluation set;
- routing does not regress the task-success baseline;
- a normal local analysis completes in at most five minutes on the reference large inventory;
- macOS, Linux, and Windows each pass a real-environment acceptance run;
- small, large, and cross-Agent duplicate scenarios pass the Agent presentation review.

Comparison runs cover unmanaged baselines, careful manual governance, and relevant Skill managers. Ponytail may appear only as a development-complexity comparison.

## 10. Delivery milestones

Milestones are implementation checkpoints, not reduced product promises:

1. **Fact Scan:** adapters, identity, SQLite inventory, stable JSON.
2. **Diagnosis:** seven report categories and evidence drill-down.
3. **Usage Evidence:** local session parsers, evidence levels, retention.
4. **Library and Rosters:** Observed, Managed, Hosted and four exposure layers.
5. **Search and Routing:** FTS5, `find`, and routing evaluation.
6. **Safe Change:** Plan, Apply, journaling, Receipt, Undo, and drift refusal.
7. **Complete Acceptance:** bootstrap Skill, three platforms, eight Agents, value comparisons, and release documentation.
