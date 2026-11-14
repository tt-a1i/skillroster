<p align="center">
  <img src="docs/assets/skillroster-hero.png" width="100%" alt="SkillRoster — one library, the right skills for every agent. A shared capability library feeds curated Codex, Claude Code, and Pi Rosters; other Skills stay discoverable on demand. Preview, Apply, Undo.">
</p>

<p align="center">
  <a href="README.md">中文</a> · <strong>English</strong>
</p>

<h1 align="center">200 Skills installed. Just the essentials in view.</h1>

<p align="center">
  Keep everyday Skills in <strong>Core</strong>; find the rest <strong>On-demand</strong>.<br>
  One command shows what is exposed and how much local metadata it carries.<br>
  <strong>Preview the Plan. Confirm changes. Undo with a Receipt.</strong>
</p>

<p align="center">
  <a href="https://github.com/tt-a1i/skillroster/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/tt-a1i/skillroster?style=flat-square&color=4ADE80"></a>
  <a href="https://github.com/tt-a1i/skillroster/actions/workflows/ci.yml"><img alt="CI status" src="https://img.shields.io/github/actions/workflow/status/tt-a1i/skillroster/ci.yml?branch=main&style=flat-square&label=CI"></a>
  <a href="https://github.com/tt-a1i/homebrew-skillroster"><img alt="Homebrew tap" src="https://img.shields.io/badge/Homebrew-tap-FBB040?style=flat-square&logo=homebrew&logoColor=111111"></a>
  <a href="https://www.rust-lang.org"><img alt="Rust 1.85 or newer" src="https://img.shields.io/badge/Rust-1.85%2B-000000?style=flat-square&logo=rust"></a>
  <a href="LICENSE"><img alt="Apache 2.0 license" src="https://img.shields.io/badge/License-Apache--2.0-60A5FA?style=flat-square"></a>
</p>

<p align="center">
  <a href="#start-in-30-seconds">Get started</a> ·
  <a href="#when-skillroster-helps">Use cases</a> ·
  <a href="#the-governance-result-at-a-glance">See the evidence</a> ·
  <a href="docs/installation.md">Installation guide</a>
</p>

## When SkillRoster helps

You use more than one coding agent. Skills pile up in different directories, the same name starts hiding different versions, and every agent sees capabilities you only need occasionally. SkillRoster gives your agent the local facts to propose a setup you can review.

| Your situation | Ask your agent | What you get |
| --- | --- | --- |
| **Skills scattered across agents** | “Find duplicate Skills, broken links, and same-name versions.” | An inventory with paths and evidence to inspect before choosing what to keep. |
| **Too many default Skills** | “Help me choose the everyday Skills for Codex; keep the rest available on demand.” | A per-agent Roster Plan with the proposed exposure changes for your review. |
| **Cleanup feels risky** | “Show me what will change and how I can undo it.” | A complete Plan before any Agent-file change, followed by a verified Receipt and bounded Undo. |

**One Library, different Rosters.** Your Library is the logical collection of known Skills; each Roster is an agent’s curated view. Canonical files can stay where they are. The banner illustrates possible selections, not prescribed roles for Codex, Claude Code, or Pi.

With one agent and a small, clear native catalog, keeping your setup is a valid outcome. SkillRoster is most useful for cross-agent cleanup, version comparison, and changes you need to undo. Missing usage is not evidence of uselessness; Core entries filled by name order need review against your upcoming work.

Ordinary tasks can use existing capabilities and search for specialized Skills when useful. No optional match need not interrupt the task. Claude's local exposure counts distinguish manual invocation, disabled entries, and name-only listings; they do not measure a running session's complete catalog or tokens. [Scope and validation](docs/practical-governance.md)

## Start in 30 seconds

Once the CLI is installed, check in one call. Add Bootstrap integration only if
wanted:

```bash
skillroster check
skillroster init --agent codex
```

`check` is read-only: it scans when needed, then shows core metrics, up to three
review Findings and next steps. `init` previews installation or upgrade targets;
Enter confirms and Esc cancels. Success returns a verified Receipt, Undo and a
copyable Agent prompt. Inspection needs no initialization.

Install v1.9.0 through `brew install tt-a1i/skillroster/skillroster`
or [Release/Cargo/Windows installation](docs/installation.md).

Or connect the optional local read-only MCP server without installing Bootstrap
into each Agent's default directory:

```bash
claude mcp add --transport stdio skillroster -- skillroster mcp
codex mcp add skillroster -- skillroster mcp
```

Then ask:

> Use SkillRoster to inspect my Skills. Keep everyday capabilities in Core and the rest available on demand. Show a Plan and wait for my confirmation before changing files.

MCP offers search and diagnostics; changes still use CLI Plan, confirmation,
Apply and Receipt. [npx packaging and Cursor configuration](docs/installation.md#optional-local-stdio-mcp)
are provided; run `npx skillroster check` to inspect. Existing users can inspect Bootstrap with `setup --check --json`.

## The governance result at a glance

On the same deterministic 120-Skill inventory, the public CLI acceptance path
actually runs Scan, Report, Plan, Apply, and Undo instead of loading prepared
results:

| Controlled arm | Default exposure | Duplicate placements | Verifiable recovery |
| --- | ---: | ---: | --- |
| Unmanaged | 200 | 80 | None |
| Careful manual governance | 64 | 10 | No Receipt |
| After SkillRoster Apply | **36** | **0** | Receipt verified; Undo restored the Agent tree byte-for-byte (200 / 80) |

This shows that SkillRoster can reduce default exposure, remove duplicate
placements in this inventory, keep On-demand retrieval, and bind the change to
a verified, reversible Receipt. The complete three-arm procedure, including the
careful-manual control, is in the [repeatable acceptance record](docs/acceptance.md#executed-three-arm-value-comparison).

These controlled-inventory counts do not measure token, labor or task-quality
gains. The new footprint reports measured local UTF-8 bytes with unknowns
listed separately; it does not estimate runtime tokens.

A historical [read-only scan of a real environment](docs/acceptance/release-v1.8.28-candidate.md) also records 252 independent Skills across 892 placements. Its session coverage was incomplete: missing usage evidence is not proof that a Skill is unused.

## How it works

```mermaid
flowchart LR
    P[Person] --> A[AI agent]
    A --> B[skillroster bootstrap Skill]
    B --> C[Rust CLI]
    C --> S[Snapshot + Evidence]
    S --> F[Findings]
    F --> L[Immutable Plan]
    L -->|one confirmation| R[Apply + Receipt]
    R -. bounded Undo .-> C
    C --> D[(Local Library)]
    D --> V[Per-agent Rosters]
```

Three ideas keep the model simple:

| Concept | Meaning |
| --- | --- |
| **Library** | The complete logical collection of known local Skills. |
| **Roster** | The curated view exposed to one agent; it is not another copy of the Library. |
| **On-demand Skill** | A valid Skill omitted from default exposure but retained for local search and exact loading. |

The primary caller is an agent. Semantic judgment stays with the model; identity,
filesystem boundaries, persistence, validation, and mutation stay with the CLI.

<details>
<summary>CLI reference for Agent integrations</summary>

```bash
# Observe
skillroster scan --summary --json
skillroster report --findings --limit 20 --json

# Retrieve one complete, fingerprint-verified Skill
skillroster find --load --limit 1 --json -- "review this pull request"

# Preview bootstrap installation across detected agents
skillroster setup --json

# Review and execute an Agent-authored governance decision
skillroster plan --stdin --json
skillroster plan --show <plan-id> --json
skillroster apply <plan-id> --json
skillroster undo <receipt-id> --json

# Inspect recovery and retained local state
skillroster status --json
```

The CLI also supports Finding drilldown, exact same-name variants, confirmed
source roots, lifecycle export and retention controls. Read the
[product specification](docs/product-spec.md) for the complete contract and the
[local data lifecycle](docs/local-data-lifecycle.md) before purging history.

</details>

## Safety is product behavior

- **Read-only first.** Scan, Report, Find, Setup preview, and Status do not
  modify Agent files.
- **Evidence before action.** A Finding describes an observed condition; it
  never authorizes a change by itself.
- **One explicit confirmation.** The agent explains one complete Plan before
  Apply.
- **Fail closed on drift.** Changed, ambiguous, unreadable, or unsupported
  targets block mutation instead of producing a partial success.
- **Receipts and recovery.** Every successful mutation is journaled, verified,
  and bounded by an Undo Receipt.
- **Local by default.** Inventory, fingerprints, bounded usage observations,
  Plans, and Receipts stay on the machine; raw conversation text is not stored.

## Supported local agents

| Codex | Claude Code | Pi | OpenCode |
| :---: | :---: | :---: | :---: |
| ✓ | ✓ | ✓ | ✓ |

| Hermes | Cursor | Gemini CLI | GitHub Copilot |
| :---: | :---: | :---: | :---: |
| ✓ | ✓ | ✓ | ✓ |

Support is capability-aware: discovery does not imply that every harness allows
the same activation or mutation mechanism. SkillRoster reports those boundaries
instead of pretending the adapters are interchangeable.

## Project status

The current public release is **v1.9.0**. The local governance loop covers discovery, reporting, search, planning, Apply/Undo, recovery, and eight direct Agent adapters. Published artifacts and controlled acceptance are documented separately from independent-user evidence.

- [Latest release](https://github.com/tt-a1i/skillroster/releases/latest)
- [v1.9.0 release and platform evidence](docs/acceptance/release-v1.9.0.md)
- [v1.8.48 release and platform evidence](docs/acceptance/release-v1.8.48.md)
- [Acceptance ledger](docs/acceptance.md)
- [Product brief](docs/product-brief.md)
- [Canonical vocabulary](CONTEXT.md)

## Development

Rust 1.85 or newer is required.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- --help
```

Core logic and high-risk mutation boundaries are tested. See
[AGENTS.md](AGENTS.md) for repository conventions.

## License

SkillRoster is available under the [Apache License 2.0](LICENSE).
