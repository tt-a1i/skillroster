# SkillRoster v1.8.34 candidate

## Release notes draft

SkillRoster 1.8.34 makes the documentation inside every immutable platform
archive trustworthy for the lifetime of that archive.

- Linux, Windows, macOS arm64, and macOS x86_64 archives now package one
  version-neutral guide instead of copying the repository README from the
  candidate tag.
- The guide tells users to verify the adjacent checksum, identify the exact
  binary with `skillroster --version`, and use the public Releases page for
  current version evidence.
- Packaging rejects a linked source path on Unix and a reparse-point source or
  ancestor on Windows before reading or copying the guide.
- CI rejects hard-coded release versions and requires LF checkout bytes on
  every platform. Each packager then verifies the extracted README against the
  checked-in source.

This patch changes release packaging and its validation only. SQLite remains
at schema 12, the JSON envelope remains at schema 1, and bundled Bootstrap
content remains at version 1.8.29. There is no database migration or change to
the local-only, explicit-confirmation, Receipt-backed mutation model.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`9ab82a7c82bb2fdd371c88a471301f2afa730a54`, after
[#317](https://github.com/tt-a1i/skillroster/pull/317) closed
[#314](https://github.com/tt-a1i/skillroster/issues/314). That pull request
passed change-scope, Linux, Windows, macOS arm64, macOS x86_64, and aggregate CI
gates. Independent Spec and Standards reviews were Clean after Unix symlink and
Windows junction findings were fixed.

Before the version bump, PR #317 validation
[run 33274488000](https://github.com/tt-a1i/skillroster/actions/runs/33274488000)
passed the exact-SHA repository gate at `9cdde70edab4e88cee8c36a2469de5da7cfa78f4`
with release version 1.8.33, all four build/governance jobs, and the WSL2 Linux
archive smoke. All four downloaded checksums verified, and every extracted
README matched the checked-in Git blob byte for byte. This proves the packaging
contract merged by #317; it is not evidence that the 1.8.34 candidate has run.

That cross-platform readback caught and fixed one defect which the first green
packaging-validation run did not: Windows checkout had converted the guide from
LF to CRLF, so the archive matched its local checkout but not the Git blob. The
final contract fixes the file to `eol=lf` and verifies the effective Git
attribute before packaging.

The source candidate and Cargo package are versioned as 1.8.34. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.33 until the v1.8.34 tag,
artifacts, checksums, and Homebrew package actually exist.

## Candidate gates

The candidate is not accepted until one exact final source revision passes:

- `cargo fmt --all --check`;
- `cargo clippy --locked --all-targets --all-features -- -D warnings`;
- `cargo test --locked --all-targets --all-features`;
- the public CLI acceptance suite and Node harnesses;
- `git diff --check`, installation-surface validation, archive README
  validation, and the CI change-scope self-test;
- four platform build and governance jobs plus the WSL2 governance smoke;
- downloaded checksum verification and an external four-archive comparison
  against the checked-in README Git blob.

Candidate preparation does not create or push a tag, publish a GitHub Release,
update Homebrew, or mutate an existing public release asset. Those operations
remain separately evidenced after candidate acceptance.
