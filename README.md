# SkillRoster

> One library. The right roster for every agent.

SkillRoster is a local-first Agent Skill Manager. It helps people ask an AI agent to inspect, organize, search, right-size, and safely change the Skills installed across local agent harnesses.

The primary caller is an agent. A person should be able to say:

> Use SkillRoster to check whether my agents have too many Skills and propose a safer setup.

The agent then calls the `skillroster` CLI and returns an evidence-backed plan for human review.

## Product principles

- **Agent-first:** one product name, one CLI token, and one bootstrap Skill: `skillroster`.
- **One-confirmation Apply:** the Agent prepares and explains a complete Plan; the person approves the whole scope once.
- **Two intentional interfaces:** stable JSON for Agents and a polished, accessible terminal experience for people.
- **Local-first:** inventory, usage evidence, configuration, plans, and receipts stay on the user's machine by default.
- **One library, many rosters:** Skills have one canonical local source; each agent receives a curated view instead of the entire catalog.
- **Evidence before change:** distinguish installed, exposed, invoked, useful, duplicated, stale, and unsafe.
- **Reversible by default:** mutating operations follow `plan -> apply -> undo` and produce receipts.
- **No MCP in v1:** the initial product is a Rust CLI plus one thin bootstrap Skill.

## Workflow

```bash
skillroster scan --json
skillroster report --json
skillroster find "database migration" --json
skillroster plan --stdin --json
skillroster apply <plan-id> --json
skillroster undo <receipt-id> --json
skillroster setup --json
skillroster status --json
skillroster lifecycle export --output ./skillroster-export.json --json
skillroster lifecycle purge --raw-days 180 --json
skillroster lifecycle recovery --json
```

Agents use JSON mode; people can omit it for concise terminal output. Read-only
commands never change Agent files. Plan stores an immutable preview, while Apply
and Undo use fingerprints, journals, receipts, and recovery blocking.

Agent-authored Plans are declarative: they reference the latest Snapshot and
Evidence IDs, then request Roster states, managed/hosted Library placement, or a
source update. Raw filesystem operations are rejected. Library consolidation
keeps canonical content recoverable and replaces verified duplicate placements
with links; every applied sequence is bounded by a Receipt and can be undone.

## Status

Pre-alpha. A working local governance path is available for fixture and macOS
development use, including read-only analysis, immutable Plans, Receipt-bounded
Apply/Undo, lifecycle retention, and recovery inspection. Full acceptance is
still in progress. Cross-platform release artifacts and real-environment
acceptance on Linux and Windows remain release gates rather than claimed support.

See [docs/product-spec.md](docs/product-spec.md) for the complete requirements, [docs/implementation-plan.md](docs/implementation-plan.md) for delivery checkpoints, and [CONTEXT.md](CONTEXT.md) for canonical vocabulary.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- --help
```
