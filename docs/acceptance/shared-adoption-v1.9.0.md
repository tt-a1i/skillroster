# Shared adoption and Roster planning: v1.9.0 candidate

This follow-up builds on committed baseline `7bb9684`. CLI remains **1.9.0**;
the bundled Bootstrap is **1.8.54**, with the complete 1.8.53 package retained
for official upgrade and Undo recognition. No public release, publication,
deployment or promotion was performed. Published Formula references and hashes
remain unchanged.

## Result and implementation

- Large-Roster Findings cover one Agent each. An entirely shared Codex root
  no longer blocks a separate mutable Hermes Roster.
- Finding-derived requests retain immutable placements and existing exposure
  with unknown native eligibility in Core. They report the exact minimum Core
  budget. Retention does not prove runtime eligibility or permit changes to
  those paths. An explicit skipped branch remains bounded and unread; depth,
  non-Unicode, missing, drifted and legacy unknown boundaries still block.
- Check and Report expose an immediately available Plan when one exists,
  while keeping high-severity safety Findings visible. Shared adoption remains
  a separate explicit request, never a default authorization.
- `adopt_shared_skills` hosts verified ordinary real packages from the physical
  shared root into the active state directory's Library. A whole Agent root
  alias becomes a real independent directory with selected Core links and
  retained excluded Skill entries. Hidden/system, provider/cache, external and
  incomplete packages remain at their original locations. Observed direct
  consumers retain exposure; unobserved consumers are a disclosed boundary.
- The workflow reuses immutable Plans, the existing lock, filesystem operations,
  journal, Receipt and bounded Undo. Exact root entries, original alias text,
  package evidence, projected native rules and operation scope are checked
  before Apply. Undo restores package locations and original root symlinks;
  user edits block restoration rather than being overwritten.
- Implicit Roster hosting validates the exact relocated package and its
  consumers. Separate copies with the same Skill identity remain separate
  evidence and do not acquire a fictitious relocation or native-policy change.

New orchestration is in `src/app/shared_adoption.rs`,
`src/app/roster_guidance.rs` and `src/app/library_visibility.rs`;
preserved-branch validation is in `src/roster_plan/discovery_scope.rs`.
`src/app.rs` decreased from 15,590 to 15,218 lines relative to the baseline.
There is no new dependency, storage table or lock.

## Real-home isolated preview

The optimized macOS ARM64 binary used an independent temporary `--state-dir`.
Only Check, Report, MCP read tools and Plan previews ran against real directories.
No real Init, Apply, Undo, source authorization or host configuration write ran.
All 5,525 guarded file/link/directory records were identical before and after,
including the shared store, known Agent Skill roots, the default SkillRoster
state boundary and Codex/Claude/Hermes configuration files. External symlink
targets were not traversed by the guard. Temporary inspection state was removed
after collecting evidence; its preview IDs are not reusable Apply handles.

The final observation contained **268 Skills, 514 Placements and 233 default
exposures**. These are current local observations, separate from the earlier
baseline inventory. Both requested previews reached `state: ready`:

| Preview | Exposure before → after | Known frontmatter bytes | Known entrypoint bytes |
| --- | --- | --- | --- |
| Hermes only, Core budget 50 | Hermes 77 → 50; total 233 → 206 | 82,678 → 74,132 | 1,900,850 → 1,689,681 |
| Explicit shared adoption, Core budget 15 | Codex 86 → 20; total 233 → 167 | 82,678 → 64,977 | 1,900,850 → 1,509,123 |

The shared preview hosted 47 packages, separated one root alias and disclosed
three affected Agents. Its 190 operations were previewed, never executed on the
real machine. The independent Hermes preview contained 29 operations, including
22 implicit Hosted packages. Both retained four unknown byte measurements.
These quantities are measured local UTF-8 metadata bytes per exposed Placement,
not session tokens. Core fallback selections remain explicitly review-required.

Representative real Check output:

```text
Review Findings · 3 of 92
  high    Skill links escape an approved root · blocked
  medium  Large Codex Roster needs review · blocked
  medium  Large Hermes Roster needs review · plan available
Shared Skills: explicit adoption preview in report --full.
Read-only · no Agent files changed
```

The shared preview was requested through `plan --stdin --json`, retaining the
same temporary state and Snapshot:

```json
{
  "schema_version": 1,
  "scan_id": "<current snapshot_id from check>",
  "adopt_shared_skills": {"core_budget": 15, "protected_skill_ids": []}
}
```

The real temporary store ended with **1 Scan, 1 Report, 2 Plans, 0 Receipts**.
One first Check took 1.9765 seconds, cached Check 0.0321 seconds, Hermes planning
0.0873 seconds and shared adoption planning 2.5077 seconds. These are single-run
observations, not general performance guarantees. MCP Inspect accepted null
arguments; Chinese Find loaded complete verified `humanizer-zh` instructions
while preserving shared-read-only authority. English retrieval also remained
read-only. The real protocol frames are retained locally.

## Verification and limits

The final `bash scripts/ci-full-check.sh` completed successfully:

| Gate | Result |
| --- | --- |
| Rust formatting and strict Clippy | Passed |
| All enabled Rust tests | 710 passed, 0 failed; 1 existing opt-in scale test ignored |
| Node harness | 152 passed |
| npm distribution tests | 3 passed |
| README, installation and release-archive surfaces | All three passed |

Deterministic temporary-directory tests cover independent Agent Findings,
minimum budgets, retained unknown/external branches, exact alias text restoration,
same-identity content variants, complete package support-file drift, root
retarget/replacement, added shared entries, native destination rules, On-demand
loading after adoption and user-edit preservation during bounded Undo. Existing
raw-operation, shared-root, physical alias, source-identity and recovery tests
remain enabled.

The native binary was staged through the checksum-verifying npm generator.
All five packages packed; the real ARM64 optional package installed offline,
and offline npx Check plus piped MCP initialize/list/inspect/load passed.
Null parameters and blank input lines produced only the expected response
frames. Foreign-target archives in that local probe were synthetic packaging
fixtures and were never executed or published. Release first-use scenarios
(empty, small, duplicate and large) and the governance smoke also passed.

Entry-bound external-link preservation currently requires Unix; unsupported
platforms reject that adoption Plan before Ready. Public delivery still requires
the established cross-platform release workflow. The local full-check result
and final source/binary digests are recorded in ignored
`dist/shared-adoption-candidate/verification-summary.json`; detailed diagnostics
stay local and are not release archive inputs.

## Minimum-toolchain verification follow-up

Publication was deferred on 2026-11-14. PR #398 remains open; no v1.9.0 tag,
GitHub Release, Homebrew update or npm publication was performed.

Cloud validation exposed a Rust 1.85-incompatible let-chain and two older
Clippy `format_collect` warnings in MCP test helpers. Equivalent supported
syntax fixes both without raising the minimum Rust version. Review also found
that Check could reuse a Snapshot after changing Home or explicit roots with
the same state directory. Snapshots now bind normalized Home, Agent roots and
source roots; changed scope refreshes, equivalent paths/order reuse, and old
Snapshots refresh once. Explicit MCP Snapshot requirements keep their existing
optimistic read boundary.

The full `scripts/ci-full-check.sh` was then executed with
`RUSTUP_TOOLCHAIN=1.85.0`: **713 Rust tests passed, 0 failed, 1 existing opt-in
test ignored; 152 Node tests and 3 npm tests passed**, including strict
Clippy/formatting and all surface verifiers. The 14 human-experience tests
include the three new scope-binding regressions and actual temporary-terminal
confirmation/cancellation. Shared adoption tests execute Apply, Receipt,
post-Apply Scan, On-demand loading and exact Undo in synthetic temporary homes.

The current debug build also repeated isolated real-home Check, MCP inspection,
Chinese verified loading and both Ready Plan previews. Real-home Apply and Undo
remain outside this verification. Detailed logs are local under ignored
`dist/v1.9.0-validation-20261114/`; the earlier optimized measurements above
remain a separate run rather than a performance claim about this debug build.
