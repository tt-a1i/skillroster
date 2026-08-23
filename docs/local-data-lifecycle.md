# Local Data Lifecycle

SkillRoster keeps governance state in `~/.skillroster/skillroster.db`. It reads
supported Agent sessions in place and stores only derived evidence summaries;
exports do not contain raw prompts or responses.

When a bounded planning error omits source-confirmation blockers, SkillRoster
writes one versioned JSON detail artifact under
`~/.skillroster/source-confirmation/`. These derived artifacts contain Skill
identities and local paths, remain visible to lifecycle commands, and are kept
until explicitly purged or local state is deleted. Artifacts are published by
same-directory atomic rename; lifecycle operations require the owned ULID file
name and complete versioned schema before reading or removing them. Unexpected
entries fail closed and remain untouched. The blocker keeps
`files_changed=false` for Agent and Library content while reporting
`state_files_changed=true` and `detail_artifact_created=true` when it retained
this auxiliary local state.

Session sampling is bounded in memory. Large active files contribute only a
recent complete-line or structurally complete nested-object tail, and the byte budget is spread across multiple recent
files. The database stores event stage, quality, time, Skill identity, Agent,
and a source-path digest; it does not store the sampled conversation text.

## Inspect and export

Use `skillroster lifecycle inspect --json` to see row counts, retained
source-confirmation detail counts, evidence-source exclusions, and recovery
state. Export derived evidence and retained source-confirmation details to a
new local file:

```sh
skillroster lifecycle export --output ./skillroster-export.json --json
```

Existing export files are never overwritten.

## Exclude session evidence

Exclude one of the eight supported Agents from future session scans while still
scanning its Skill roots:

```sh
skillroster lifecycle exclude codex --json
skillroster scan --json
```

The Scan reports Codex session roots as `excluded`. Restore scanning with
`skillroster lifecycle exclude codex --remove --json`. Exclusion changes only
SkillRoster's local policy; it never edits Agent files.

## Purge retained state

`lifecycle purge --raw-days 180` aggregates older usage by month and removes the
corresponding raw evidence rows. Plans and Receipts are preserved unless the
caller explicitly selects them and supplies the exact confirmation token:

```sh
skillroster lifecycle purge --plans-receipts \
  --confirm PURGE-PLANS-RECEIPTS --json
```

This removes Undo history and is refused while recovery is required. It never
deletes Agent or Library content.

Purge source-confirmation details independently when their trust decision is no
longer needed:

```sh
skillroster lifecycle purge --source-confirmation --json
```

The purge validates the owned directory and every entry before changing any
selected lifecycle state; links and unexpected entries fail closed.

## Delete and rebuild local state

To delete SQLite state, terminal Receipt journals, recovery artifacts, and
source-confirmation details:

```sh
skillroster lifecycle delete --confirm DELETE-LOCAL-STATE --json
```

The command preserves all Agent roots and `~/.skillroster/library`, refuses an
unresolved recovery state, and removes SQLite WAL sidecars. Run
`skillroster scan --json` to rebuild inventory state.
