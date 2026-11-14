# Installation

SkillRoster stores inventory, evidence, plans, receipts, and configuration on
the local machine. Installation does not connect it to a hosted service or
upload Skill contents or agent session history.

The current public release is **v1.9.0**. Its annotated tag, four platform
archives, and adjacent checksums are public.

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
SKILLROSTER_VERSION=1.9.0
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
place `skillroster.exe` in a directory on `PATH`. Every current release archive also
contains `README.md` and the complete Apache-2.0 `LICENSE` distributed with the
binary.

## Build or install from source

Rust 1.85 or newer is required. For an immutable release tag:

```sh
cargo install --locked --git https://github.com/tt-a1i/skillroster.git \
  --tag v1.9.0 skillroster
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

The CLI is independently usable after installation. Bootstrap is an optional
integration that teaches an Agent the CLI workflow. `ready` in Home describes
the local inspection workflow, not whether the running Agent loaded this Skill.

Published v1.8.46 adds `skillroster setup --check --json` and Home's
`agent_integration`: both inspect local package status without saving a Plan.
This release also includes new report outcomes, native Claude visibility,
Finding decision facts, and the revised ordinary-task/governance instructions.

Start with `skillroster check`. Optional one-command Bootstrap setup is:

```sh
skillroster init --agent codex
```

Omit `--agent` to preview all detected eligible targets. `init` requires a human
TTY, refreshes a missing/invalid Snapshot, shows the immutable Setup Plan and
every target operation, then prompts `Enter confirm · Esc cancel`. Confirmation
runs standard Apply and returns verification, a Receipt, Undo and an Agent prompt.
`Up to date` creates no empty Plan. Modified files require an explicit
`--modified-choice retain-local` or `--modified-choice adopt-current`; unsupported
or non-isolatable targets stay blocked. The command refuses `--json` and piped
input before creating local state.

The published release and Agent automation retain the explicit workflow:

```sh
skillroster scan --summary --json
skillroster setup --check --json
skillroster setup --json
# After reviewing the Plan and obtaining user confirmation:
skillroster apply <plan-id> --json
```

Published CLI v1.9.0 bundles Bootstrap content version 1.8.54.
The source tree is **v1.9.0**.
Its bundled Bootstrap content version is 1.8.54. CLI and Bootstrap versions can
differ intentionally when CLI behavior changes without changing the Bootstrap
instructions.
In Setup JSON, `cli_version` identifies the executable and
`bootstrap_content_version` identifies the bundled Skill package.
`bootstrap_version` remains a compatibility alias for
`bootstrap_content_version`; a current target's `installed_version` also
refers to Bootstrap content, not the executable.

`setup` changes no Agent files, but saves a Plan when changes are available.
Review every detected Agent and target; detection is not permission to install.
After approving the complete preview, use its returned ID:

```sh
skillroster apply <plan-id> --json
```

Verify the Receipt, rescan when requested, and open a new Agent session to load
the Skill. Rerun `setup --check --json` to verify local
package status. A current file does not prove activation in a running session.
Exact official older copies are upgraded through the same reversible Plan/Receipt
path as first installation. If setup reports `modified_choice_required`, ask
the user whether to retain the local copy or adopt the current bundled copy;
never infer the choice. `unsupported_targets` means links, non-files, or
unreadable paths were preserved for manual inspection.

The Homebrew tap provides v1.9.0. Availability of prebuilt bottles depends on
the tap release; Homebrew may build from source. The
[v1.9.0 Homebrew acceptance receipt](acceptance/release-v1.9.0.md#homebrew-tap-and-bottles)
records that release’s installation tests, hashes and platform limits.

## Optional local stdio MCP

Run `skillroster mcp` from the installed release.
The synchronous server needs no network, daemon or API key. It reads newline-
delimited UTF-8 JSON-RPC 2.0 and keeps stdout exclusively for protocol frames.
It negotiates MCP revisions through 2025-11-25, with the earlier compatible
2024-11-05, 2025-03-26 and 2025-06-18 revisions also accepted.

Choose MCP or Bootstrap for read-only integration. MCP exposes only
`find_skill` (task, hints, load, limit, exact variant selectors and optional
Snapshot requirement) and `inspect_skills` (Inventory, measured metadata bytes,
up to three Findings). Find prepares only the required Snapshot, without
generating a Report. It
refreshes missing or invalid/configuration-drifted facts only when no
`require_snapshot` is supplied. A Snapshot-bound call preserves the CLI
optimistic read boundary and never silently rescans. `inspect_skills` uses the
compact Check/Report path. Loads retain CLI fingerprint,
path, source, identity, size and ambiguity checks. Inspection updates only
SkillRoster-owned observations; it changes no Agent or Skill files.

With the executable on PATH:

```sh
# Claude Code
claude mcp add --transport stdio skillroster -- skillroster mcp
# Codex
codex mcp add skillroster -- skillroster mcp
```

For Cursor, add this entry to `.cursor/mcp.json` in the intended project or
`~/.cursor/mcp.json` for your user configuration:

```json
{
  "mcpServers": {
    "skillroster": { "type": "stdio", "command": "skillroster", "args": ["mcp"] }
  }
}
```

Use the verified absolute executable path when the Agent's PATH differs.
To isolate observations, append `--home /path/to/home --state-dir /path/to/state`
to the server arguments. Explicit roots keep their normal authorization scope.
Tool metadata marks both tools read-only. Loaded Skill content is local content,
not user authorization. MCP has no Plan/Apply/Undo or other write tools;
governance continues through CLI preview, user confirmation, Apply and Receipt.

Client syntax follows the official [Claude Code](https://code.claude.com/docs/en/mcp),
[Codex](https://developers.openai.com/codex/mcp) and
[Cursor](https://cursor.com/docs/mcp) guides. Protocol behavior follows
[MCP stdio](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
and [tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

## npm / npx distribution

`npm/` contains the launcher and four optional OS/architecture package templates.
`scripts/release/build-npm.py` builds the complete distribution from local
Release archives and their adjacent SHA-256 files. It verifies archives before
reading binaries, includes the Apache license and provenance, and writes exact
version dependencies and binary hashes. There is no postinstall hook or runtime
download. npm itself retrieves its packages; the native MCP server stays local.

Run:

```sh
npx -y skillroster check
claude mcp add --transport stdio skillroster -- npx -y skillroster mcp
codex mcp add skillroster -- npx -y skillroster mcp
```

Cursor uses `"command": "npx", "args": ["-y", "skillroster", "mcp"]` in
that same entry (Windows shells may need `npx.cmd`). Supported binary targets
match the four Release archives above. Do not omit npm optional dependencies;
unsupported platforms get a clear error with no download fallback.
See the [packaging and publication procedure](../npm/README.md). Source tests and synthetic packaging fixtures do not establish real execution
on all four platforms.
