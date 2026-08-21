# SkillRoster product brief

## Problem

Agent Skills accumulate across Codex, Claude Code, Pi, OpenCode, Hermes, and other harnesses. Users often cannot answer basic questions reliably:

- Which Skills are installed, and where did they come from?
- Which Skills are exposed to each agent?
- Which Skills have actually been invoked recently?
- Which entries are duplicates, stale copies, broken links, or unsafe mutations?
- Can a rarely used Skill remain searchable without occupying every agent's default catalog?
- Can the system reorganize Skills without losing user-owned files or making rollback difficult?

Existing install and synchronization tools solve only part of this lifecycle. SkillRoster focuses on local governance: visibility, right-sizing, reviewable change, and recovery.

## Primary interaction

The user delegates an intent to an agent. The agent loads the single `skillroster` bootstrap Skill, calls the deterministic CLI, interprets structured results, and asks for confirmation before material changes.

```text
Person
  -> AI agent
      -> SkillRoster bootstrap Skill
          -> skillroster CLI
              -> local inventory, index, plans, and receipts
```

The CLI does not replace the harness and does not execute the user's actual task.

## Canonical vocabulary

### Skill Library

The complete local collection of known Skills, including active and cold entries. A Skill should have one canonical source whenever practical.

### Agent Roster

The curated set of Skills visible to one agent in one scope. A roster is a view of the library, not a second copy of it.

### Cold Skill

A valid Skill retained in the library but omitted from a default roster. It remains discoverable through local search and can be proposed for a task.

### Evidence

Observable facts such as filesystem presence, symlink targets, source metadata, configuration exposure, invocation records, and validation results. Absence of invocation evidence is not proof that a Skill has no value.

### Plan

A deterministic preview of proposed filesystem or configuration changes. A plan identifies exact targets, ownership boundaries, risks, and expected postconditions.

### Receipt

A record of an applied plan containing enough information to verify the result and undo SkillRoster-owned changes.

## V1 scope

1. Discover supported local agent harnesses and their Skill locations.
2. Build a normalized, path-aware inventory without modifying files.
3. Identify copies, symlinks, collisions, broken references, and likely duplicates.
4. Maintain a local search index for cold Skills.
5. Produce usage reports with explicit evidence quality and time windows.
6. Generate per-agent roster proposals.
7. Apply approved changes through bounded adapters and write receipts.
8. Undo SkillRoster-owned changes without deleting canonical Skill contents.
9. Expose machine-readable output suitable for agent callers.

## Non-goals for V1

- No MCP server.
- No hosted marketplace or mandatory account.
- No automatic deletion based only on age or low usage.
- No silent modification of agent configuration.
- No assumption that every harness supports the same activation mechanism.
- No claim that a successful scan proves a Skill is useful or safe at runtime.

## Safety contract

- Read-only commands are the default entry point.
- Plans are immutable inputs to apply operations.
- Apply refuses drifted or ambiguous targets.
- User-owned files are preserved unless the approved plan identifies them explicitly.
- Every successful mutation produces a receipt.
- Undo is bounded to changes recorded in that receipt.
- Structured output is stable enough for agents and CI to parse without scraping prose.

## Initial technical direction

- Rust CLI.
- SQLite with FTS5 for local inventory, evidence, and search.
- Adapter boundary per agent harness.
- JSON output for agent callers; concise terminal output for people.
- One thin bootstrap Skill that teaches agents when to call the CLI.
