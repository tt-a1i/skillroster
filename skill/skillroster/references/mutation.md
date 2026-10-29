# Mutation and recovery

## Setup

The CLI works independently of Agent integration. Home's `agent_integration`
reports local Bootstrap files, not activation in an already-running session.
Use `skillroster setup --check --json` to inspect detected targets and versions
without creating a Plan. Missing Bootstrap is an optional integration step,
not a blocker for ordinary CLI use. When the user wants Agent integration,
run `skillroster setup --json` to preview installation or upgrade of this
complete package. Show every detected Agent and target before Apply; never
assume all supported Agents are installed or that detection authorizes setup.
For a request limited to one Agent, use `setup --agent ID --json` (or `--check`)
and retain that selector on follow-up Setup calls. Omission includes all eligible
detected targets. Distinguish full `detected_agents` from selected `targets`;
`selected_scope_not_isolatable` requires a scope decision before any Plan.
An exact official older package produces a recoverable Plan. When
`state` is `modified_choice_required`, show affected Agent targets and ask for
`retain-local` or `adopt-current`; never choose for the user. Unsupported links
or non-files remain blocked.

Read each target's `detection_basis`. `existing_skill_root` means the fixed
Agent Skill root already exists. `included_session_root` is current-Snapshot
evidence that the same Agent is present before its first Skill root exists;
show the fixed parent-directory and four-file Plan, and do not infer any other
Agent. `no_supported_agent` means the Snapshot included neither a supported
Skill root nor session root, so do not invent a target. Setup remains preview
only until the person confirms the ordinary Apply action.

## Apply

Before `skillroster apply PLAN_ID --json`, show the complete bounded Plan impact
and obtain one explicit confirmation. Do not replace the Plan with filesystem
commands or bypass drift checks. Report verification, changed-path count,
Receipt ID, and canonical deletion count.

A verified Apply returns `rescan_required: true`. Follow its read-only Scan
action before Report, Find, Plan, Setup, source confirmation, or another Apply;
the Plan's Snapshot predates the verified mutation and is no longer current.
Keep the exact Receipt Undo action available. Status, recovery inspection, and
Undo remain valid before that Scan. Every verified Undo returns its own required
read-only Scan action; follow it before using current inventory facts, including
after unchanged immediate Undo. Undo verification covers the Receipt's bounded
restoration and preserves user edits; a fresh Scan establishes the next inventory
observation. Never infer that Undo made an earlier Snapshot current.

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
