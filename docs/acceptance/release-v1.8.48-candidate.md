# SkillRoster v1.8.48 release candidate

CLI 1.8.48 bundles Bootstrap 1.8.52. Candidate rc.4 and formal release acceptance passed; see [release receipt](release-v1.8.48.md).
The accepted release is v1.8.48.

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

Candidate rc.1 exposed two Windows-only test assertions comparing path strings
with mixed separators. rc.2 compares path components while preserving the exact
selected-copy and stale-placement refusal assertions. No product logic changed.

Candidate rc.2 reached the JSON help regression on Windows and exposed a
Unix-only executable-name expectation. rc.3 accounts for the `.exe` suffix
without changing CLI behavior or relaxing JSON/exit-status/side-effect checks.

Candidate rc.3 exposed one more Unix separator assumption in routing metadata
placement selection. rc.4 uses path components there and in the adjacent Library
error-path assertion. All product behavior and refusal conditions remain intact.
