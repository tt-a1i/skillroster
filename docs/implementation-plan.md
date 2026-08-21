# Implementation Plan

This plan delivers the complete product in vertical checkpoints while preserving one binary, one database, and one bootstrap Skill. A checkpoint is complete only when its stated evidence exists; it is not marketed as a smaller product.

## Architectural shape

Keep `src/main.rs` limited to CLI parsing and command dispatch. Add focused modules only when needed:

```text
src/
  cli.rs            command and stable output envelope
  app.rs            thin command orchestration
  model.rs          IDs, states, Findings, Plans, Receipts
  fs.rs             safe normalization and filesystem operations
  sqlite.rs         schema, migrations, and direct queries
  scan.rs           discovery and fingerprints
  harness/          eight direct read/normalize adapters
  query.rs          report, find, and status
  change/           plan validation, apply journal, recovery, undo
skill/
  skillroster/SKILL.md
tests/
  fixtures/         synthetic Agent homes and session samples
```

Adapters discover and normalize Agent-specific facts. They do not implement eight separate mutation systems. All writes pass through `change`, and only one Apply or Undo may hold the process-level write lock at a time. Do not add an ORM, repository trait, Tokio runtime, generic plugin interface, vector database, or virtual filesystem. A private filesystem seam inside `change` is allowed for fault-injection tests.

## Persistence model

Keep the initial schema focused on five groups:

- **Snapshots:** scan runs, supported Agents, roots, logical Skills, and physical placements.
- **Evidence:** paths, digests, source metadata, exposure, normalized usage events, and coverage.
- **Rosters:** per-Agent desired membership and Core, On-demand, Explicit-only, or Archived state.
- **Analysis:** immutable reports, Findings, and Evidence links.
- **Change:** immutable Plans and operations, then Receipts and observed operation results.

A path is not a Skill identity. A Skill ID names the logical Skill, a placement ID names one physical appearance, and a versioned content digest proves exact content identity. Plans record intended operations; Receipts record what actually happened. These records must not be collapsed.

SQLite uses WAL for concurrent read-only queries. It cannot make filesystem operations atomic. Apply therefore persists each before-state before mutation and uses `prepared -> applying -> applied`, with explicit `failed_rolled_back` and `recovery_required` outcomes.

## Checkpoint 1: Fact Scan

- Define opaque IDs and the versioned JSON response envelope.
- Initialize and migrate `~/.skillroster/skillroster.db`.
- Implement known-root discovery for the eight Agents.
- Parse `SKILL.md`, source metadata, symlinks, and exposure configuration.
- Persist snapshots, roots, Skill identities, placements, and Evidence.
- Version the digest algorithm, including file order and ignored paths.
- Add fixtures for exact copies, collisions, broken links, and escaping links.

**Exit evidence:** `scan --json` matches every expected fixture and makes no Agent filesystem change.

## Checkpoint 2: Diagnosis

- Implement inventory, layout, exposure, overlap, routing, and lifecycle facts.
- Assign stable report, Finding, and Evidence IDs.
- Support `report --finding <id> --json` without rescanning.
- Separate deterministic duplicate proof from semantic duplicate candidates.

**Exit evidence:** every reported claim resolves to a path, record, or fingerprint.

## Checkpoint 3: Usage Evidence

- Parse supported local session/event formats in place.
- Normalize Exposed, Matched, Loaded, Applied, and Outcome stages.
- Label every claim Observed, Inferred, or Unknown.
- Emit rates only when the observable-session denominator is reliable.
- Implement 180-day summaries, monthly aggregation, exclusions, export, and purge.

**Exit evidence:** fixtures prove raw prompts and responses are not copied.

## Checkpoint 4: Library and Rosters

- Implement Observed, Managed, and Hosted governance states.
- Implement Core, On-demand, Explicit-only, and Archived Roster states.
- Create `~/.skillroster/library` only through an approved Plan.
- Model links and exposure changes without assuming identical Agent behavior.

**Exit evidence:** proposals preserve canonical content and explain every link change.

## Checkpoint 5: Search and Routing

- Index names, descriptions, declared triggers, and normalized Skill text with FTS5.
- Implement ranked `find <task>` results with paths and match reasons.
- Maintain a representative task-to-Skill evaluation set.

**Exit evidence:** Top-3 recall is at least 95% without task-success regression.

## Checkpoint 6: Safe Change

- Validate Agent-authored declarative Plan input against current snapshot IDs and fingerprints.
- Store immutable Plans and require explicit Apply; provide no `--force`, `--yes`, or `--auto-fix` bypass.
- Journal each operation and compensate failed cross-filesystem sequences.
- Produce Receipts and implement bounded Undo as a new reverse Receipt.
- Refuse drift, ambiguity, escaping targets, and undeclared canonical deletion.
- Block new writes while any Receipt requires recovery.

**Exit evidence:** covered fixture states round-trip through Apply and Undo exactly, including injected failure after every operation step.

## Checkpoint 7: Complete Acceptance

- Implement `setup` and the one bootstrap Skill.
- Verify all eight adapters in representative real environments.
- Verify macOS, Linux, and Windows release artifacts.
- Run the three user-value scenarios: discover disorder, confirm a healthy setup, and retrieve an On-demand Skill.
- Compare unmanaged, manual, and relevant manager baselines.
- Document installation through Homebrew, Cargo, and Windows release binaries.

**Exit evidence:** every completion gate in `docs/product-spec.md` passes.

## Testing policy

Tests cover identity, path boundaries, normalization, adapter fixtures, migrations, search ranking, Plan determinism, Apply journaling, Undo, and regressions in these areas. Low-risk CLI wiring and documentation do not require dedicated tests. Mutation tests use temporary directories and assert both final state and recovery state.
