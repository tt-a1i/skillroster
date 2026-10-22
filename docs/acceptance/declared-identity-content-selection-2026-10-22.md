# Declared identity content selection

Date: 2026-10-22. Base: `128f72f`; CLI version remains 1.8.46.

## Reproduced defect

Two trusted local packages declared the same source and version but contained
different names and instructions: `research` and `deploy`. Scan retained one
logical Skill ID and only the first package's searchable metadata. Find could
rank `research` with exact-name evidence and load the lexically earlier deploy
placement, returning deploy instructions with successful identity checks.
The other package's name could also be absent from search.

The existing `declared_identity_divergent_content` Finding described the package
conflict but did not prevent this implicit load. Source/version identity remains
the documented identity rule; the fix preserves that logical ID and records the
distinct content observations underneath it.

## Implemented contract

- Scan binds each placement to its routing-content digest and retains each
  distinct observation's name, metadata and searchable text. Roster and usage
  records keep the logical Skill ID. Legacy Snapshots require one new Scan.
- FTS selects candidate IDs from all observations. Ranking scores each observation
  separately; it cannot combine one copy's name with another copy's description
  or task evidence. Find identifies the observation that supplied the evidence.
- Different routing-content digests block implicit loading with
  `declared_identity_content_ambiguous`, without returning instructions. Changes
  only to root `.gitignore` retain routing equivalence; supporting-file changes
  require selection even when `SKILL.md` is identical.
- Find exposes bounded content groups and current per-placement read states.
  `--variant-placement-id` selects an exact placement within the current Top-1
  capability group. Optional Skill and placement selectors must agree.
  Selection returns the actual name and placement, with ranking evidence scoped
  to `ranked_capability_group`; comparison does not choose canonical content or
  authorize a Plan.
- Exact selection preserves Archived, source authority, Snapshot, physical path,
  entrypoint, package digest, size and encoding checks. Failure never falls back
  to a healthy sibling. Missing files and escaped roots retain their existing
  error codes; an approved-root retarget still fails the physical-path check.
- Returned read-only commands preserve the executable, Home, state directory,
  source roots, original task, hints and Snapshot. Truncated choices lead to the
  same Snapshot's Finding and full detail, rather than repeated rescanning.
- Same-name Findings include supplementary observations with their actual names
  and paths. Every attached routing-digest and path Evidence resolves in the
  state store. Overlapping names with the same logical-ID set share one covering
  Finding instead of colliding on a stable Finding ID.

The bundled Bootstrap content is 1.8.51. Its routing reference explains the
comparison recovery. The exact 1.8.50 package is retained as an upgrade fixture;
Setup/Apply/Undo restores all old package files byte for byte. Public-release
installation claims remain at the existing published versions.

## Regressions and independent replay

`tests/declared_content.rs` exercises eight public CLI scenarios:

1. Both names remain searchable, implicit loads are blocked, and explicit reads
   return the chosen content without combining partial metadata matches.
2. Supporting-file differences require selection; `.gitignore` alone does not.
3. Selectors enforce Top-1 membership, matching IDs and current bytes, including
   replay of returned commands after entrypoint or support-file drift.
4. Eleven content groups retain total counts and expose preview truncation.
5. Same-name Skill selection cannot bypass divergent content within one ID.
6. Finding recovery returns exact read commands with the correct context and
   does not offer an automatic Plan.
7. A legacy Snapshot missing content observations requires a new Scan.
8. A supplementary same-name observation produces a usable Finding with
   resolvable Evidence and correctly replayable comparison commands.

Independent replay confirmed the original research/deploy mismatch is closed in
both directions. It also found and verified the fix for a supplementary-name
Finding gap: adding a second logical `deploy` identity now produces the expected
same-name Finding, bound only to actual deploy placements. Separate scope review
replayed non-Top-1 and Archived refusal, symlink retarget refusal, healthy exact
selection and truncated-group recovery without another actionable finding.

## Validation

- Formatting and strict all-target/all-feature Clippy passed.
- All-target/all-feature Rust tests: 554 passed, one existing ignored test.
- Node harness: 152 passed.
- Build, public help and Find help passed. Human Find output explicitly reports
  that two content choices require placement selection.
- Full-check execution reached installation-document validation after the Rust
  and Node suites passed. Its source Bootstrap version assertion caught one
  stale documentation value. After correcting that value, README evidence,
  installation-surface and release-archive checks each passed.
- `git diff --check` passed.

These are local implementation, fixture and CLI checks. They do not establish a
new live-model task outcome, token savings, merge, release, deployment or upgrade
of the user's installed binary. No real user Skill packages or Agent configuration
were changed by the isolated regressions.
