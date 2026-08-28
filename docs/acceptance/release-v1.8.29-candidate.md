# SkillRoster v1.8.29 release receipt

## Release notes

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

## Release evidence

The annotated `v1.8.29` tag resolves exactly to
`b996c7e52bd849243ce7772cff30602a00c4270b`. The official tag workflow
[run 33197816927](https://github.com/tt-a1i/skillroster/actions/runs/33197816927)
passed the strict repository gate, Linux, Windows, macOS arm64 and x86_64
build/governance jobs, and the WSL2 Linux governance smoke.

The release delta is recorded in first-parent history from #233 through #297.
That includes the security fixes, typed-module simplification, first-value
acceptance work, routing and recovery corrections found during review, and the
final WSL capability boundary.

The exact tagged revision passed:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and both Node routing harnesses;
- `git diff --check` and the CI change-scope self-test.

The [public GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.29)
contains four platform archives and four adjacent checksum files. All eight
assets were downloaded through their unauthenticated public URLs; every
checksum passed. The public macOS arm64 archive reported `skillroster 1.8.29`,
rendered help, and passed the complete synthetic Scan, Setup, Apply, Receipt,
Undo, recovery-clear, and retained-ID governance smoke.

The [Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster) publishes
v1.8.29 arm64 macOS and x86_64 Linux bottles. Both Homebrew test-bot jobs passed
in [run 33198038445](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33198038445),
and `brew pr-pull` published the bottles in
[run 33199481374](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/33199481374).
The public macOS arm64 bottle was independently downloaded, matched the Formula
checksum, reported `skillroster 1.8.29`, and passed the same governance smoke.

The release WSL boundary is WSL2. WSL1 rejects Apply and Undo when its kernel
cannot provide atomic no-replace rename; SkillRoster does not substitute a
race-prone check-then-rename fallback.
