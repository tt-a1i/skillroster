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
skillroster --source-root /absolute/trusted/source scan --json
skillroster report --summary --json
skillroster report --findings --limit 20 --json
skillroster report --findings --category usage --json
skillroster report --finding <finding-id> --limit 20 --json
skillroster report --finding <finding-id> --full --json
skillroster find "database migration" --json
skillroster find "诊断命令性能回归" --hint "diagnose command performance regression" --json
skillroster find "分析本地表格" --hint "analyze standalone spreadsheet file workbook data" --json
skillroster plan --stdin --json
skillroster plan --show <plan-id> --json
skillroster apply <plan-id> --json
skillroster undo <receipt-id> --json
skillroster setup --json
skillroster status --json
skillroster lifecycle inspect --json
skillroster lifecycle export --output ./skillroster-export.json --json
skillroster lifecycle exclude codex --json
skillroster lifecycle purge --raw-days 180 --json
skillroster lifecycle recovery --json
```

Agents use JSON mode; people can omit it for concise terminal output. Read-only
commands never change Agent files. Plan stores an immutable preview, while Apply
and Undo use fingerprints, journals, receipts, and recovery blocking.
See [docs/local-data-lifecycle.md](docs/local-data-lifecycle.md) before purging
Plan/Receipt history or deleting the local database.
See [docs/installation.md](docs/installation.md) for verified Release, Cargo,
and Homebrew installation paths.

Run `setup` after the first Scan and after each CLI upgrade. Missing or exact
official older bootstrap Skills become a recoverable Plan. A locally modified
copy is never replaced implicitly: the Agent must show the affected targets and
ask the user before retrying with `--modified-choice retain-local` or
`--modified-choice adopt-current`. Links, non-files, and unreadable targets are
preserved and reported as blocked.

`--root AGENT=PATH` adds an Agent placement root and therefore contributes to
that Agent's default exposure. `--source-root PATH` approves a non-exposed
canonical source directory for the current Scan. Neither option crawls outside
the exact supplied path.

Agent-authored Plans are declarative: they reference the latest Snapshot and
Evidence IDs, then request Roster states, managed/hosted Library placement, or a
source update. Raw filesystem operations are rejected. Library consolidation
keeps canonical content recoverable and replaces verified duplicate placements
with links; every applied sequence is bounded by a Receipt and can be undone.
The initial Plan response is a bounded decision summary. Its `diff_summary`
shows the semantic Roster, Library, and filesystem deltas (or bounded line
facts for a source update). Exact operations and complete internal ID lists
remain in the immutable local Plan and are available on demand through
`plan --show`, so large governance scopes do not flood the Agent's context.

For an exact-duplicate Finding, the Agent submits only the choices it owns:

```json
{"schema_version":1,"finding_library_changes":[{"finding_id":"finding_...","canonical_placement_id":"placement_...","requested_state":"managed"}]}
```

SkillRoster resolves the current Snapshot, Evidence, Skill, and complete
placement set from that immutable Finding and rejects stale or mismatched input.

For a large default-Roster Finding, the Agent chooses a Core budget instead of
copying every Skill and placement into the request:

```json
{"schema_version":1,"finding_roster_changes":[{"finding_id":"finding_...","core_budget":50,"protected_skill_ids":[]}]}
```

The CLI preserves declared and protected Core Skills, ranks positive local
usage evidence, and moves only the remainder to On-demand. Missing usage is not
treated as evidence for archiving. A placement without exact owned canonical
content blocks the semantic Plan; the Agent follows the typed decision to
confirm source roots or resolve a dependent source link, then rescans instead
of applying a partial scope.

Finding drilldown is compact by default: each paged `item` carries one
traceable Evidence ID, subject, path, and decision facts. Complete duplicate ID
collections, placement records, and Evidence records stay behind explicit
`report --finding ID --full --json`. Unsafe escaping links return a trust
decision and observed targets instead of suggesting an automatic filesystem
Plan.

The three-Finding summary also includes compact Finding-group rollups with
deduplicated affected Skill and placement counts. It leads to
`report --findings`, a compact paged list that can be filtered by category or
severity. This gives Agents both the complete problem scale and a bounded path
to every Finding without loading the exhaustive report. Only a selected
Finding leads to Evidence inspection or planning.

## Status

SkillRoster 1.0 implements the complete local governance loop,
including read-only analysis, versioned immutable Plans, Receipt-bounded
Apply/Undo, lifecycle controls, recovery, and eight direct Agent adapters.
Platform support is claimed only after the corresponding release workflow and
artifact smoke test are recorded in the release acceptance evidence.

See [docs/product-spec.md](docs/product-spec.md) for the complete requirements, [docs/implementation-plan.md](docs/implementation-plan.md) for delivery checkpoints, and [CONTEXT.md](CONTEXT.md) for canonical vocabulary.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- --help
```

## License

SkillRoster is licensed under the [Apache License 2.0](LICENSE).
