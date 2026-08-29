# Release process

SkillRoster releases contain only binaries and repository documentation built
from the public source tree. Workflows must never upload local Skill libraries,
agent sessions, inventories, plans, receipts, configuration, credentials, or
other user data.

Every archive packages `docs/release-archive/README.md` as its root
`README.md`. That guide is intentionally version-neutral because an immutable
candidate tag cannot truthfully claim which release is currently public. The
packaging jobs on all four platforms verify that the archived README matches
the checked-in source exactly; the normal full CI gate rejects a hard-coded
release version in that source.

## Candidate build

1. Update `Cargo.toml` and `Cargo.lock` to the intended version.
2. Run `cargo fmt --all --check`,
   `cargo clippy --locked --all-targets --all-features -- -D warnings`, and
   `cargo test --locked --all-targets --all-features` locally.
3. Push an annotated pre-release tag such as `v1.0.0-rc.1`, or dispatch the
   **Release candidate** workflow after it exists on the default branch. It
   builds Linux x86_64, Windows x86_64, macOS arm64, and macOS x86_64 archives
   without creating a GitHub Release.
4. Download all four workflow artifacts and verify every adjacent `.sha256`
   file. The workflow smoke-tests `skillroster --version` and a fixture-backed
   Plan/Apply/Undo cycle on each operating system, then runs the Linux archive
   inside checksum-pinned Ubuntu WSL2. Unix packaging also runs the staged
   archive binary through the synthetic retained-ID collision regression before
   the archive is created. Review all five successful jobs.

Candidate artifacts expire after 14 days. The workflow token has only
`contents: read`; checkout credentials are not persisted.

## Publish

After candidate acceptance, create and push an annotated `vX.Y.Z` tag whose
version matches `Cargo.toml`. The tag runs the same locked build. Review the
four successful jobs and checksums, then create the GitHub Release manually and
attach the eight files (four archives plus four checksum files). Include
user-visible changes, known limitations, and the tested platform matrix in the
release notes.

Publishing a tag or Release is a separate, explicitly authorized operation.
The workflow intentionally does not grant `contents: write` or publish on its
own.

## Automation choices

The workflows use GitHub-maintained
[`actions/checkout@v6`](https://github.com/actions/checkout) and
[`actions/upload-artifact@v7`](https://github.com/actions/upload-artifact).
Runner labels follow GitHub's
[hosted-runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners):
`ubuntu-24.04`, `windows-2025`, `macos-15`, and `macos-15-intel`. The
top-level `contents: read` declaration follows GitHub's
[least-privilege workflow guidance](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#permissions).
Rust is installed with `rustup`, avoiding an additional third-party setup
action.
