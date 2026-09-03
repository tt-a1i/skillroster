# Installation

SkillRoster stores inventory, evidence, plans, receipts, and configuration on
the local machine. Installation does not connect it to a hosted service or
upload Skill contents or agent session history.

The current public release is **v1.8.44**. Its Formula, annotated tag, four
platform archives, and adjacent checksums are public.

WSL users need WSL2 and should use the Linux archive. WSL1 lacks the atomic
no-replace rename primitive required for safe Apply and Undo, so mutation fails
closed there instead of falling back to a race-prone path operation.

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
SKILLROSTER_VERSION=1.8.44
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
contains `README.md` and the complete Apache-2.0 `LICENSE` distributed with the
binary.

## Build or install from source

Rust 1.85 or newer is required. For an immutable release tag:

```sh
cargo install --locked --git https://github.com/tt-a1i/skillroster.git \
  --tag v1.8.44 skillroster
```

For a local checkout, run `cargo install --locked --path .`. Confirm the
installation with `skillroster --version` and `skillroster --help`.

## Homebrew

Install from the official SkillRoster tap. Homebrew adds the tap automatically:

```sh
brew install tt-a1i/skillroster/skillroster
brew test skillroster
```

To add the tap before installing, run `brew tap tt-a1i/skillroster`, then
`brew install skillroster`.

The source repository and the
[Homebrew tap](https://github.com/tt-a1i/homebrew-skillroster) are public, so
installation does not require GitHub authentication. This is an upstream tap,
not a Homebrew/core Formula. Never paste a GitHub token into a Formula or a
command line.

## Upgrade and verify the executable your Agent uses

Pick one installation method to own future CLI upgrades. For Homebrew:

```sh
brew update
brew upgrade tt-a1i/skillroster/skillroster
```

An upgraded package does not prove that `skillroster` selects it. A previous
Release binary in `~/.local/bin`, a Cargo install in `~/.cargo/bin`, or a shell
alias/function can take precedence. In the shell that launches your Agent,
check the selected command, all resolutions, and the Homebrew copy separately:

```sh
command -v skillroster
type -a skillroster
skillroster --version
"$(brew --prefix)/bin/skillroster" --version
```

`type -a` is supported by Bash and Zsh. In PowerShell, use
`Get-Command skillroster -All` to inspect command precedence instead. Compare
the CLI version with the release you intended to install, not with Bootstrap
content version: those versions can intentionally differ.

If the selected command is still old, inspect its ownership before changing
anything. Do not delete every result from `type -a` or overwrite an alias,
wrapper, package-manager link, or shell configuration automatically. For a
confirmed standalone binary that you choose to retire, move that exact file
to a recoverable backup outside command lookup. If existing callers need its
old absolute path, a symlink at that path can point to the verified stable
`$(brew --prefix)/bin/skillroster` entrypoint, rather than a versioned Cellar
path. Keep the backup until the replacement is verified. Never point that
entrypoint back to the retired path or create a link to a missing target.

Open a fresh shell (or run `hash -r` in Bash / `rehash` in Zsh), then repeat the
path and version checks. Restart the Agent that was launched with the old
environment and ask it to run the checks too. A successful terminal check does
not establish which binary an already-running Agent or saved absolute-path
command uses. This verification does not authorize changes to Agent Skill
files; Bootstrap upgrades still require the reviewed Setup Plan below.

## Install or upgrade the Agent bootstrap Skill

After installing a new CLI version, refresh the local Snapshot and inspect all
detected bootstrap targets:

```sh
skillroster scan --summary --json
skillroster setup --json
```

Published CLI v1.8.44 bundles Bootstrap content version 1.8.29.
The source tree is **v1.8.44**.
Its bundled Bootstrap content version is 1.8.29. CLI and Bootstrap versions can
differ intentionally when CLI behavior changes without changing the Bootstrap
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
