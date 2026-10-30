# SkillRoster v1.8.48 release candidate

CLI 1.8.48 bundles Bootstrap 1.8.52. Candidate acceptance is pending.
The current public release remains v1.8.47 until publication.

## Changes

- Verify Library content again before planning and applying changes; require a fresh Scan after verified Undo.
- Add optional single-Agent Setup previews and refuse writes when shared physical targets prevent isolation.
- Improve bounded routing metadata, required Skill identity checks, hint recovery and verified equivalent-copy fallback.
- Distinguish unknown source modification status, malformed update requests, package preservation and permissions.
- Preserve discovery context during state rebuild; return JSON help/version as successful operations.
- Clarify source update fields and managed/hosted placement choices.

## Acceptance

Prior local candidate: 662 Rust tests passed, one scale measurement ignored.
Release-version checks, Rust 1.85, four native archive builds, WSL2 and downloaded
archive checks must pass at the frozen release commit before publication.
Local assessment material and real Skill contents are excluded from Git and artifacts.

## Limits

Controlled fixtures do not establish independent-user acceptance, complete host
session visibility or general retrieval quality. WSL1 mutation and native Linux
arm64/Windows arm64 remain outside the release matrix. No real Agent installation
is changed by acceptance. Bootstrap updates remain optional and require inspection.
