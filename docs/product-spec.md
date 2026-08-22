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

SkillRoster scans only known directories for these eight Agents plus paths explicitly provided by the user. It never crawls the entire home directory by default. Every Scan reports included, excluded, missing, and inaccessible roots.

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

The CLI owns deterministic discovery, normalization, hashing, indexing, statistics, validation, mutation, and recovery. The calling Agent owns semantic comparison, explanation, prioritization, and natural-language presentation. The binary contains no model API, second Agent, or prompt framework.

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

Usage evidence is labeled `observed`, `inferred`, or `unknown`. Missing evidence never becomes proof that a Skill is useless. Raw prompts and responses are parsed read-only in place and are not copied into SkillRoster storage. A usage percentage is emitted only when the observable-session denominator is reliable; otherwise the report shows counts, time range, and coverage limits.

### 4.3 Library and Rosters

The Library is logical and does not require moving every Skill. Each discovered Skill begins as Observed. After approval it may become:

- **Managed:** SkillRoster manages exposure, links, or configuration while the canonical files remain in place.
- **Hosted:** canonical files are explicitly migrated into `~/.skillroster/library`.

Each Agent Roster classifies Skills as Core, On-demand, Explicit-only, or Archived. On-demand Skills remain searchable without being linked into every default Skill directory. `find` returns paths and Evidence so the Agent can read the selected `SKILL.md` directly; temporary activation links are unnecessary.

### 4.4 Planning, mutation, and recovery

Read-only commands never mutate Agent configuration or Skill contents. Agent recommendations enter the CLI through `plan --stdin` as structured data referencing Scan IDs, Skill IDs, Evidence IDs, and requested target states.

The CLI validates targets, ownership, conflicts, and current fingerprints, then stores an immutable Plan. `apply <plan-id>` executes only the approved scope. Cross-filesystem operations use a journal and compensating steps; the CLI never claims atomicity it cannot provide.

Apply fails closed when paths, fingerprints, links, or configuration have drifted. Every successful mutation writes a Receipt. `undo <receipt-id>` is bounded to that Receipt and refuses ambiguous restoration. Canonical deletion is outside normal Apply, requires separate confirmation, and should prefer recoverable archive.

### 4.5 Source updates

SkillRoster records source, revision, and content hash when available. A source-update request submitted through `plan --stdin` first produces a diff and Plan; there is no silent update path. Local modifications stop automatic replacement and require an explicit choice to retain local content, adopt upstream content, or preserve both.

## 5. CLI contract

The first complete command surface is:

```bash
skillroster scan [--json]
skillroster report [--finding <id>] [--json]
skillroster find <task> [--json]
skillroster plan --stdin [--json]
skillroster apply <plan-id> [--json]
skillroster undo <receipt-id> [--json]
skillroster status [--json]
skillroster setup [--json]
```

`setup` detects supported Agents and returns a preview for installing the single bootstrap Skill. In JSON mode it never prompts or mutates; the installation is represented by a Plan and completed through the normal Apply path.

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

## 6. Agent presentation contract

The normative conversation flow, one-confirmation Apply behavior, state-dependent primary action, and bootstrap Skill rules live in [agent-experience-spec.md](agent-experience-spec.md).

The bootstrap Skill instructs an Agent to prepare a complete read-only Plan when Evidence is sufficient, then present:

1. a one-sentence diagnosis;
2. independent Skill count, placement count, default exposure, and observed-use count;
3. the three most important Findings, then the remaining Findings grouped by category;
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
- Older usage is reduced to monthly aggregates.
- Plans and Receipts remain until explicitly purged.
- `status` exposes storage location and retention state.
- Users can inspect, export, purge, or delete the database and rebuild it by scanning again.

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
