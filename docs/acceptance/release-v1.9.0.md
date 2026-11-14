# SkillRoster v1.9.0 release receipt

CLI **1.9.0** bundles Bootstrap **1.8.54**. The
[GitHub Release](https://github.com/tt-a1i/skillroster/releases/tag/v1.9.0)
was published on 2026-11-14 at 08:09:49 UTC with four native archives and
four adjacent checksum files. All five npm packages and both Homebrew bottles
were subsequently read back from their public distribution endpoints.

## Source and validation

The annotated tag resolves to `5111bba8662adc3518c6f67ed8dcba6618479ab1`,
merged through [PR #400](https://github.com/tt-a1i/skillroster/pull/400).
Its tree matches merge commit `70b63de`.

- [Candidate and published artifact build](https://github.com/tt-a1i/skillroster/actions/runs/43572393471): passed.
- [Formal tag build](https://github.com/tt-a1i/skillroster/actions/runs/43573347216): passed at the same source commit.
- [Four-platform PR CI](https://github.com/tt-a1i/skillroster/actions/runs/43572394770): passed.

The exact-source full gate passed formatting, strict Clippy, 723 Rust tests
(one opt-in scale measurement ignored), 152 Node harness tests, three npm
packaging tests and all three surface consistency scripts on Rust 1.85.
Both release runs passed native macOS ARM/Intel, Linux x86_64 and Windows
x86_64 tests, synthetic Plan/Apply/Undo, extracted archive first use and
checksum-pinned Ubuntu WSL2 acceptance. Published archives come from the
accepted candidate run; the formal tag independently validates the same source.

Earlier candidates exposed hosted macOS timer coalescing in the synthetic
split-Escape PTY input. Test delivery now uses a bounded 10 ms spin while
retaining its 25 ms qualification, five-attempt limit and queued-byte checks.
Ten parallel repetitions passed; removing the production input flush still
failed with two queued bytes. Production terminal handling was unchanged.

## Public archive hashes

| Target | Archive SHA-256 |
| --- | --- |
| aarch64-apple-darwin | `14c82c16ee102937dcc52e0b3470dc7dde53643c3de26db2786545c5bf0d258e` |
| x86_64-apple-darwin | `185c5c80ee825481e4ef94676159313ccbd10066480dd840a109fad98653b8f6` |
| x86_64-unknown-linux-gnu | `445079f56e281f767ec78f2e2b16b399b22624355670b2782a008142d7a0e23c` |
| x86_64-pc-windows-msvc | `d7018b83e8d7ea91680a2d81298cc6351fde8b72a4041a809c12ec4f4db56352` |

All eight public files were downloaded again and their archive hashes verified.
The public macOS ARM executable passed empty, small, duplicate and large
inventory first-use scenarios, including Bootstrap installation and Undo in
temporary homes. Additional check/MCP tests passed initialize, blank lines,
`cursor: null`, `arguments: null`, inspection and verified Chinese-task Skill
loading with English hints. MCP emitted only its four expected response frames
and no stderr.

## Homebrew tap and bottles

[Tap PR #24](https://github.com/tt-a1i/homebrew-skillroster/pull/24) was integrated
by the existing SHA-guarded pr-pull workflow at PR head
`e50985f025963e8975e466e0567bfff9bc448944`. Tap main is
`0af0aa236a2382a96392dd9a93cadc7dbabdd155`.

- [macOS/Linux build, bottle installation and Formula tests](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/43573444828): passed.
- [Bottle publication](https://github.com/tt-a1i/homebrew-skillroster/actions/runs/43574068708): passed.
- [Public bottles](https://github.com/tt-a1i/homebrew-skillroster/releases/tag/skillroster-1.9.0): downloaded and verified against Formula hashes.

Source archive SHA-256: `4deac30a8cbf0fafc9cd2a502bf472c72a60e818560c0a9990762f1a071bd360`.

| Bottle | SHA-256 |
| --- | --- |
| arm64_tahoe | `345dbd036f56b29f51259bb0ddd5c38c5b579fbe0fdfc18ba213b01c931b3742` |
| x86_64_linux | `af8006baeef95c3b46f39f4eb70cbbc9e542f77652fc22e07a16ff9fe627b93c` |

The public macOS bottle executable also passed all four first-use scenarios and
the check/MCP test above. Linux bottle execution was verified on the Linux
runner; Intel macOS uses the native archive or a Formula source build.

## npm delivery

The public [`skillroster`](https://www.npmjs.com/package/skillroster/v/1.9.0)
launcher and four `@tt-a1i/skillroster-{darwin-arm64,darwin-x64,linux-x64,win32-x64}`
packages report version and latest tag **1.9.0**. All five public tarballs were
downloaded and their SHA-512 integrity compared with the original packed files.
Platform binaries and provenance were generated from the checksum-verified
archives listed above. Packages contain no postinstall download hook.

A fresh npm cache installed the public main package and native optional package:

```text
$ npm exec --yes --package skillroster@1.9.0 -- skillroster --version
skillroster 1.9.0
check_ok: true
mcp_response_ids: [1, 2, 3, 4]
verified_skill_loaded: true
stdio_stderr_bytes: 0
```

The check and MCP calls used only a temporary synthetic home and state directory.
Registry propagation initially returned transient 404s; publication was not
repeated. npm installation was executed on macOS ARM; other platform binary
execution is covered by the native release matrix, rather than a claim of
local cross-platform npm execution.

## Limits

Controlled acceptance does not establish independent-user acceptance or exact
runtime token consumption. Shared adoption preserves external symlink entries
on Unix; Windows safely refuses that branch. WSL1 mutations and native Linux
arm64/Windows arm64 remain outside the matrix. Real Agent directories and the
existing user state were not modified by release verification. Local Cargo
artifacts were cleaned after use, releasing 314.4 MiB.
