# Installation

SkillRoster stores inventory, evidence, plans, receipts, and configuration on
the local machine. Installation does not connect it to a hosted service or
upload Skill contents or agent session history.

## GitHub Release binaries

Download the archive and matching `.sha256` file for your platform from the
[GitHub Releases](https://github.com/tt-a1i/skillroster/releases) page:

| Platform | Archive target |
| --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Windows x86_64 | `x86_64-pc-windows-msvc` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |

Verify the checksum before extracting:

```sh
sha256sum -c skillroster-*.sha256     # Linux
shasum -a 256 -c skillroster-*.sha256 # macOS
```

On macOS or Linux, the archive contains a versioned directory. Extract it and
install the binary from that directory (not from the current directory):

```sh
SKILLROSTER_VERSION=1.8.28
SKILLROSTER_TARGET=aarch64-apple-darwin # choose from the table above
tar -xzf "skillroster-${SKILLROSTER_VERSION}-${SKILLROSTER_TARGET}.tar.gz"
mkdir -p "$HOME/.local/bin"
install "skillroster-${SKILLROSTER_VERSION}-${SKILLROSTER_TARGET}/skillroster" \
  "$HOME/.local/bin/skillroster"
export PATH="$HOME/.local/bin:$PATH"
skillroster --version
```

Persist the PATH entry in the startup file for the shell that launches your
Agent, then restart that Agent so future sessions can resolve `skillroster`.

On Windows, compare `Get-FileHash .\skillroster-*.zip -Algorithm SHA256` with
the checksum file. Extract the archive, open its versioned directory, then
place `skillroster.exe` in a directory on `PATH`. Every release archive also
contains the complete Apache-2.0 `LICENSE` distributed with the binary.

## Build or install from source

Rust 1.85 or newer is required. For an immutable release tag:

```sh
cargo install --locked --git https://github.com/tt-a1i/skillroster.git \
  --tag v1.8.28 skillroster
```

For a local checkout, run `cargo install --locked --path .`. Confirm the
installation with `skillroster --version` and `skillroster --help`.

## Homebrew

The public repository also acts as a source-building Homebrew tap:

```sh
brew tap tt-a1i/skillroster https://github.com/tt-a1i/skillroster.git
brew install tt-a1i/skillroster/skillroster
brew test skillroster
```

The repository is public, so cloning the source and downloading Release
archives do not require GitHub authentication. This is a repository-backed
custom tap, not a Homebrew/core Formula. Never paste a GitHub token into the
Formula or a command line.

## Install or upgrade the Agent bootstrap Skill

After installing a new CLI version, refresh the local Snapshot and inspect all
detected bootstrap targets:

```sh
skillroster scan --summary --json
skillroster setup --json
```

CLI v1.8.28 bundles Bootstrap content version 1.8.28. These versions can differ
intentionally when CLI behavior changes without changing the Bootstrap
instructions.
In Setup JSON, `cli_version` identifies the executable and
`bootstrap_content_version` identifies the bundled Skill package.
`bootstrap_version` remains a compatibility alias for
`bootstrap_content_version`; a current target's `installed_version` also
refers to Bootstrap content, not the executable.

`setup` changes no files. Apply its returned Plan only after review. Exact
official older copies are upgraded through the same reversible Plan/Receipt
path as first installation. If setup reports `modified_choice_required`, ask
the user whether to retain the local copy or adopt the current bundled copy;
never infer the choice. `unsupported_targets` means links, non-files, or
unreadable paths were preserved for manual inspection.
