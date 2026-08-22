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

On Windows, compare `Get-FileHash .\skillroster-*.zip -Algorithm SHA256` with
the checksum file. Extract the archive, then place `skillroster` (or
`skillroster.exe`) in a directory on `PATH`.

## Build or install from source

Rust 1.85 or newer is required. For an immutable release tag:

```sh
cargo install --locked --git https://github.com/tt-a1i/skillroster.git \
  --tag v1.0.0 skillroster
```

For a local checkout, run `cargo install --locked --path .`. Confirm the
installation with `skillroster --version` and `skillroster --help`.

## Homebrew

The repository includes a source-building Formula. Until a public tap exists,
clone the release tag and install the checked-in Formula:

```sh
git clone https://github.com/tt-a1i/skillroster.git
cd skillroster
git checkout v1.0.0
brew install --formula ./Formula/skillroster.rb
brew test skillroster
```

The repository is public, so cloning the source and downloading Release
archives do not require GitHub authentication. This is a checked-in Formula,
not yet a published Homebrew tap. Never paste a GitHub token into the Formula
or a command line.
