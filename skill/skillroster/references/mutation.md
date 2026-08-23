# Mutation and recovery

## Setup

Run `skillroster setup --json` to install or upgrade this complete Bootstrap
package. An exact official older package produces a recoverable Plan. When
`state` is `modified_choice_required`, show affected Agent targets and ask for
`retain-local` or `adopt-current`; never choose for the user. Unsupported links
or non-files remain blocked.

## Apply

Before `skillroster apply PLAN_ID --json`, show the complete bounded Plan impact
and obtain one explicit confirmation. Do not replace the Plan with filesystem
commands or bypass drift checks. Report verification, changed-path count,
Receipt ID, and canonical deletion count.

“Complete” means every affected Agent, Skill, placement, operation group,
exclusion, risk, and before/after delta is accounted for by count. It does not
mean dumping every internal operation. Load stored Plan detail only for exact
questions.

## Undo and recovery

Explain the bounded Receipt impact and obtain one explicit confirmation before
`skillroster undo RECEIPT_ID --json`.

Stop mutating on ambiguity, drift, unsupported scope, or recovery required.
Recovery becomes the only mutating next action. Use `skillroster status --json`
for pending Plans, the last Receipt, retention, and recovery state, and
`skillroster lifecycle recovery --json` for an unresolved Receipt.

Export or purge local lifecycle data only when requested. Purge changes
controlled SQLite history, not Agent files.
