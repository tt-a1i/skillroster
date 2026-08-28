# SkillRoster v1.8.29 candidate

## Release notes draft

SkillRoster 1.8.29 brings the fail-closed and reversible boundaries promised by
the product model to the public package built from current `main`.

- Apply, compensation, and Undo bind filesystem operations to retained approved
  root handles, so replacing an ancestor with a symlink cannot redirect an
  already-approved mutation outside that root.
- Receipt journals synchronize both file contents and directory entries across
  create, rename, and removal boundaries, with recovery and fault-injection
  coverage for interrupted operations.
- Unix local state defaults to owner-only directories and files; Windows state
  receives current-user-only ACLs. Existing overbroad state is repaired or
  rejected according to the documented safety boundary.
- Identity-bearing non-Unicode paths fail closed instead of being converted to
  lossy strings that could collide in placement IDs or fingerprints.
- File replacement and Undo preserve Unix mode bits and Windows file
  attributes, including readonly. Ownership, ACL, and extended-attribute
  preservation are not newly claimed.
- Changes to bundled Bootstrap Markdown now trigger full CI, and candidate/tag
  validation runs the strict repository gates against the exact event SHA.
- Roster operations and Finding behavior use typed domain values rather than
  ad hoc JSON or prose classification. Suggested-action context and application
  error taxonomy now have focused module owners without changing their public
  responsibility boundary.
- The bilingual README surfaces the controlled 120-Skill governance result:
  default exposure 200 to 36, duplicate placements 80 to 0, verified Receipt,
  and exact Undo. These are controlled-fixture outcomes, not token, labor,
  performance, model-quality, or universal-superiority claims.

The release keeps SQLite schema 10, JSON envelope schema 1, local-only
operation, explicit confirmation, and Receipt-bounded Apply/Undo. Strict JSON
consumers must continue to tolerate additive typed error and coverage fields.
Snapshots created before the Unicode identity contract require a new complete
Scan before exact identity decisions. Existing user-owned Skill contents are
not migrated or deleted by upgrading the binary.

## Preparation evidence

The public baseline is v1.8.28. Candidate preparation starts from exact
`origin/main` revision `b7f6d47d9746dc3d6735b9e328748a467d603085` in a clean
worktree. Cargo, installation examples, presentation fixtures, the website
source, and bundled Bootstrap content are versioned as 1.8.29. The checked-in
Homebrew Formula remains at the last published release until v1.8.29 artifacts
exist and their independently verified checksum is available.

The implementation already merged after v1.8.28 is represented by pull requests
#233, #235, #237, #239, #241, #243, #245, #247, #249, #251, and #253. The
outcome-first README clarification in #254 is also part of this candidate.

## Candidate gates

The candidate is not accepted until one exact final source revision passes:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and both Node routing harnesses;
- `git diff --check` and the CI change-scope self-test.

Candidate preparation does not create or push a tag, publish a GitHub Release,
change the Homebrew tap, or mutate any public release asset. Those remain a
separate explicitly authorized publication gate. Final candidate SHA, CI run,
four-platform archives, adjacent checksums, WSL evidence, anonymous download
readback, governance smoke, and Homebrew installation evidence will be recorded
only after they exist.

The release WSL boundary is WSL2. WSL1 rejects Apply and Undo when its kernel
cannot provide atomic no-replace rename; SkillRoster does not substitute a
race-prone check-then-rename fallback.
