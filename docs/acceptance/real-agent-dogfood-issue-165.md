# Byte-level zero-change ledger — Issue #165

Date: 2026-08-24 (Asia/Shanghai)

This addendum strengthens the read-only real-agent dogfood from Issue #163. It
uses the independent Node standard-library ledger helper in
`tests/harness/byte-ledger/ledger.mjs`; raw ledgers and command output remain
outside the repository. The public artifact contains only aggregates,
digests, and boundary statements.

## Run and scope

- Candidate: `skillroster 1.8.24`, source `26a60ee6b76a9cb3c76e0d71b2f76e1bf2281b4e`
- Sequence: before ledger → read-only Scan → Report → two Find queries →
  Status → after ledger → redacted comparison
- Approved scope: 18 canonical non-symlink Skill roots derived from the 21
  Scan-included Skill roots; three Agent roots were symlink aliases of the
  shared root and were refused as roots by the helper's fail-closed policy.
- Explicit configuration inputs: four regular files supplied in a separate
  newline-delimited list
- State and evidence were isolated; no Apply, Setup, Undo, Plan persistence,
  delete, purge, roster/config/Skill mutation, or external-target read ran.

## Byte-ledger result

Before and after both contain 3,944 records: 2,797 regular files, 1,140
directories, and 7 symlinks. Regular-file bytes total 70,262,720. The before
and after ledger digest is the same (`db32a5511ea6658cfe3236ff3328869b0dccf9aa0ffeb4340414c516c2eab2d2`),
with zero added, removed, or changed records. The four explicit configuration
files also match by content digest.

Seven symlinks resolve outside the approved root set. Their link identity and
target spelling were recorded, but their targets were not followed, read, or
hashed. Those target bytes are explicitly out of scope.

## CLI evidence

The read-only flow reported 251 independent Skills, 887 placements, 521
default exposures, and 177 Findings. Observed usage covered three Agents, but
coverage remained incomplete (five limited roots and three missing roots), so
absence is still unknown rather than “never used”. Final Status reported
`files_changed=false`, recovery `clear`, zero journal issues, and zero pending
Plans.

## Acceptance boundary

The approved Skill/config estate covered by this ledger was byte-stable across
the flow. This does not prove that every file in Home, dynamic sessions/logs,
caches, repository files, isolated state, or external symlink targets was
unchanged. It also does not add semantic intent quality, routing superiority,
or a governance authorization claim.

The redacted machine record is
[`real-agent-dogfood-issue-165.json`](artifacts/real-agent-dogfood-issue-165.json).
