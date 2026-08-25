<p align="center">
  <img src=".github/assets/skillroster-readme-hero.png" alt="SkillRoster routes one local Skill library into focused rosters for multiple AI agents" width="100%">
</p>

<h1 align="center">SkillRoster</h1>

<p align="center">
  <strong>One library. The right roster for every agent.</strong>
</p>

<p align="center">
  Local-first Skill governance for AI agents.<br>
  See what is installed, reduce default exposure, find the right Skill on demand, and make reversible changes.
</p>

<p align="center">
  <a href="https://github.com/tt-a1i/skillroster/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/tt-a1i/skillroster?style=flat-square&color=4ADE80"></a>
  <a href="https://github.com/tt-a1i/skillroster/actions/workflows/ci.yml"><img alt="CI status" src="https://img.shields.io/github/actions/workflow/status/tt-a1i/skillroster/ci.yml?branch=main&style=flat-square&label=CI"></a>
  <a href="https://github.com/tt-a1i/homebrew-skillroster"><img alt="Homebrew tap" src="https://img.shields.io/badge/Homebrew-tap-FBB040?style=flat-square&logo=homebrew&logoColor=111111"></a>
  <a href="https://www.rust-lang.org"><img alt="Rust 1.85 or newer" src="https://img.shields.io/badge/Rust-1.85%2B-000000?style=flat-square&logo=rust"></a>
  <a href="LICENSE"><img alt="Apache 2.0 license" src="https://img.shields.io/badge/License-Apache--2.0-60A5FA?style=flat-square"></a>
</p>

<p align="center">
  <a href="#start-in-30-seconds">Install</a> ·
  <a href="#what-it-sees">See it work</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="docs/product-spec.md">Product spec</a> ·
  <a href="docs/installation.md">All platforms</a>
</p>

---

## Your agents do not need every Skill

Skills accumulate across Codex, Claude Code, Pi, OpenCode, Hermes, Cursor,
Gemini CLI, and GitHub Copilot. Copies drift. Links break. Narrow Skills occupy
every default context. Usage evidence gets mistaken for certainty.

SkillRoster gives your agent a deterministic local CLI for understanding that
estate before changing it.

| See the whole estate | Right-size each Roster | Change without fear |
| --- | --- | --- |
| Inventory Skills, placements, links, sources, exposure, and bounded usage evidence. | Keep broadly useful Skills in Core and leave narrower ones searchable On-demand. | Preview immutable Plans, require confirmation, record Receipts, and Undo owned changes. |

It is **not** another marketplace, model, or MCP server. The AI agent interprets
your intent; SkillRoster supplies bounded facts and executes approved changes.

## Start in 30 seconds

Install the current release with Homebrew:

```bash
brew install tt-a1i/skillroster/skillroster
skillroster --version
```

Then ask your agent:

> Use SkillRoster to inspect my local Skills, explain the biggest problems, and
> propose a safer setup. Do not change files until I approve the complete Plan.

Or begin directly in the terminal:

```bash
skillroster scan --summary
skillroster report
```

Agents add `--json` for one stable machine-readable document. Release archives,
Cargo installation, Windows instructions, and checksum verification are in the
[installation guide](docs/installation.md).

## What it sees

This is a real read-only v1.8.28 dogfood result from one changing local estate,
not a benchmark or a universal inventory size:

```text
SkillRoster · Report

  Independent Skills     252
  Placements             892
  Default exposure       525
  Observed-use Agents    3
  Session sample         sampled 5/8 · complete 0/8

  Top Findings
  high    layout     Skill links escape an approved root
  medium  exposure   Large default Rosters need review
  medium  overlap    Exact duplicate Skill placements

Read-only · no Agent files changed
Review evidence before planning changes
```

The important part is not the large numbers. It is the boundary: SkillRoster
reports incomplete coverage instead of turning missing observations into an
“unused” claim. See the full [release acceptance record](docs/acceptance/release-v1.8.28-candidate.md).

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

## The Agent-facing loop

```bash
# Observe
skillroster scan --summary --json
skillroster report --findings --limit 20 --json

# Retrieve one complete, fingerprint-verified Skill
skillroster find "review this pull request" --load --limit 1 --json

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

SkillRoster v1.8.28 implements the local governance loop: discovery, normalized
inventory, conservative usage evidence, bounded reporting, local retrieval,
immutable planning, Apply/Undo, recovery, lifecycle controls, and eight direct
agent adapters.

- [Latest release](https://github.com/tt-a1i/skillroster/releases/latest)
- [Release and platform evidence](docs/acceptance/release-v1.8.28-candidate.md)
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
