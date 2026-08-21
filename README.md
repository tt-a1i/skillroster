# SkillRoster

> One library. The right roster for every agent.

SkillRoster is a local-first Agent Skill Manager. It helps people ask an AI agent to inspect, organize, search, right-size, and safely change the Skills installed across local agent harnesses.

The primary caller is an agent. A person should be able to say:

> Use SkillRoster to check whether my agents have too many Skills and propose a safer setup.

The agent then calls the `skillroster` CLI and returns an evidence-backed plan for human review.

## Product principles

- **Agent-first:** one product name, one CLI token, and one bootstrap Skill: `skillroster`.
- **Local-first:** inventory, usage evidence, configuration, plans, and receipts stay on the user's machine by default.
- **One library, many rosters:** Skills have one canonical local source; each agent receives a curated view instead of the entire catalog.
- **Evidence before change:** distinguish installed, exposed, invoked, useful, duplicated, stale, and unsafe.
- **Reversible by default:** mutating operations follow `plan -> apply -> undo` and produce receipts.
- **No MCP in v1:** the initial product is a Rust CLI plus one thin bootstrap Skill.

## Planned workflow

```bash
skillroster scan
skillroster tidy
skillroster fit --agent codex --task "release a service"
skillroster find "database migration"
skillroster plan
skillroster apply <plan-id>
skillroster undo <receipt-id>
```

These commands describe the intended interface; they are not implemented yet.

## Status

Pre-alpha. The repository currently contains the Rust CLI scaffold and the initial product boundary. The next milestone is a read-only inventory prototype.

See [docs/product-brief.md](docs/product-brief.md) for the initial scope and vocabulary.

## Development

```bash
cargo fmt --check
cargo test
cargo run -- --help
```
