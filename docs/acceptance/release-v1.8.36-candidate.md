# SkillRoster v1.8.36 candidate

## Release notes draft

SkillRoster 1.8.36 closes two Agent continuation and local-read authority gaps.

- Commands that require a first Snapshot now fail with the stable
  `snapshot_required` error plus one context-bound, read-only
  `scan --summary --json` continuation. The Agent no longer has to reconstruct
  Home, state, explicit roots, source roots, or the running executable.
- Revoking a durable source-root permission immediately invalidates a current
  Snapshot that used it. Persistent root identity drift has the same effect.
- `find --load` rechecks durable read authority before returning content, so an
  old Snapshot cannot continue reading an external `SKILL.md` after revoke or
  replacement.
- Home, `status`, and typed failures expose bounded invalidating permission IDs,
  exact totals, and a Scan continuation. The new invalidation fields and typed
  failures do not repeat external paths or return Skill content.
- New Snapshots record only durable permissions that actually authorized a
  Skill read. Existing v1.8.35 payloads remain compatible through conservative
  inference from retained durable-read Placements; unused permissions and
  temporary one-Scan `--source-root` overrides do not cause false invalidation.

This patch changes Agent continuation metadata, Snapshot read-authority facts,
and readiness validation. SQLite remains at schema 12, the JSON envelope remains
at schema 1, and bundled Bootstrap content remains at version 1.8.29. No Agent
or Skill files are changed by these checks.

## Preparation evidence

Candidate preparation starts from exact `origin/main` revision
`082fbcead88eb8166c50312c6635f18075af5df4`.

[#325](https://github.com/tt-a1i/skillroster/pull/325) fixed the fresh-state
continuation at exact head `aae16d40188b49413226d29c231a37c186635b87`
and merged as `6420b38dd7df1c59ee09f39af69c8e25e4d00809`.

[#327](https://github.com/tt-a1i/skillroster/pull/327) fixed source-root
Snapshot authority at exact head
`631ca74b0cbbe526659e17ef034d49ebb82eeadc` and merged as the candidate base.
The original dogfood fixture reproduced a successful old-Snapshot
`find --load` after revoke; the corrected binary returned
`source_root_snapshot_rescan_required`, reported `rescan_required`, and did not
return the external Skill content. Independent Spec/Safety and
Standards/Compatibility reviews were Clean after their findings were fixed.

The exact-main post-merge
[CI run 33282916200](https://github.com/tt-a1i/skillroster/actions/runs/33282916200)
passed change scope, Linux x86_64, Windows x86_64, macOS arm64, macOS x86_64,
and the aggregate CI gate. The final pre-commit local gate for #327 passed 321
Rust unit tests, 8 acceptance tests, 101 CLI tests, and 152 Node harness tests,
plus strict Clippy, formatting, installation-surface validation, archive README
validation, and `git diff --check`.

The source candidate and Cargo package are versioned as 1.8.36. Public install
examples, the checked-in Formula, website current-release label, and README
release evidence deliberately remain at v1.8.35 until the v1.8.36 tag,
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
