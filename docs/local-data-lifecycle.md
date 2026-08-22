# Local Data Lifecycle

SkillRoster keeps governance state in `~/.skillroster/skillroster.db`. It reads
supported Agent sessions in place and stores only derived evidence summaries;
exports do not contain raw prompts or responses.

Session sampling is bounded in memory. Large active files contribute only a
recent complete-line tail, and the byte budget is spread across multiple recent
files. The database stores event stage, quality, time, Skill identity, Agent,
and a source-path digest; it does not store the sampled conversation text.

## Inspect and export

Use `skillroster lifecycle inspect --json` to see row counts, evidence-source
exclusions, and recovery state. Export derived evidence to a new local file:

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

## Delete and rebuild the database

To delete SQLite state and terminal Receipt journals only:

```sh
skillroster lifecycle delete --confirm DELETE-LOCAL-STATE --json
```

The command preserves all Agent roots and `~/.skillroster/library`, refuses an
unresolved recovery state, and removes SQLite WAL sidecars. Run
`skillroster scan --json` to rebuild inventory state.
