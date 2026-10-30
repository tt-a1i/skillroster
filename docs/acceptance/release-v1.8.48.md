# SkillRoster v1.8.48 release receipt

CLI **1.8.48** bundles Bootstrap **1.8.52**. Published 2026-10-30 16:38:50 UTC
(2026-10-31 00:38:50 Asia/Shanghai).

[Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.48) contains
four native archives and four adjacent checksum files.

## Changes

Single-Agent Setup previews, content revalidation before mutation, fresh inventory
after verified Undo, bounded routing metadata and verified exact-copy fallback,
required Skill identity guidance, accurate source-update diagnostics and preserved
state-rebuild discovery context. See PR #395 and the release notes for details.

## Source and acceptance

[PR #395](https://github.com/tt-a1i/skillroster/pull/395) merged as
`d2ceb46777897681c83dbc82f8be111b3a8d6272`; its tree exactly matches accepted
candidate rc.4 (`c7699dd3cab2d34b3fc8aec7b5fc567c2d47f819`).

- [PR CI](https://github.com/tt-a1i/skillroster/actions/runs/41086943224): passed.
- [Candidate](https://github.com/tt-a1i/skillroster/actions/runs/41086940123): passed.
- [Merged-main CI](https://github.com/tt-a1i/skillroster/actions/runs/41088549516): passed.
- [Formal build](https://github.com/tt-a1i/skillroster/actions/runs/41089615375): passed.

Both candidate and formal workflows validated the exact commit with Rust 1.85,
ran native tests and built Linux x86_64, Windows x86_64, macOS arm64 and macOS
x86_64 archives, verified extracted-archive first use and Plan/Apply/Undo, and
accepted the Linux archive in checksum-pinned Ubuntu WSL2. Local complete checks
passed 662 Rust tests and 152 Node tests, with one existing ignored scale measure.
Earlier candidates exposed Unix-only test assumptions about path separators and
executable suffixes; portable assertions corrected these without changing product
logic. Those candidates were superseded, not published as Releases.

## Archive hashes

| Archive | SHA-256 |
| --- | --- |
| `skillroster-1.8.48-aarch64-apple-darwin.tar.gz` | `a35484e60c8044f18c259acfb72920279c36b24c55196859f5e14ec5d17ee299` |
| `skillroster-1.8.48-x86_64-apple-darwin.tar.gz` | `ecba1233b41fa3949d94f08fbc4659387aefa0f8fe5887addf633a7d28465793` |
| `skillroster-1.8.48-x86_64-unknown-linux-gnu.tar.gz` | `af1c6751e61405c928bdbfffb14a394cb4f32d87bbbfb9bf361ea247120a565c` |
| `skillroster-1.8.48-x86_64-pc-windows-msvc.zip` | `4dca54b5b8ffecc85ba814f6da42d496f61a0f99eb3b1ad1c74ab68436269559` |

## Delivery and limits

All eight public asset downloads are compared byte for byte with formal workflow
artifacts. The public macOS ARM executable is additionally checked with the
repository governance smoke and four isolated first-use scenarios.

The repository Formula pins the release source. The separate Homebrew tap and
its bottles have an independent delivery process; this receipt does not certify
new bottles. Controlled tests do not establish independent-user acceptance,
complete live session catalogs or general retrieval quality. WSL1 mutation and
native Linux arm64/Windows arm64 remain outside the matrix. No real Agent files
or installed user CLI are changed by these acceptance checks.
