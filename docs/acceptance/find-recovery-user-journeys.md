# Find and recovery: scripted user-flow replay

Date: 2026-10-28. Baseline: `c1507fd4764bb90bb9e2f1119f56b1110e42a6d8`.
The comparison uses that exact source and this patch, not two released versions.
Both executables report CLI 1.8.47; version strings alone do not identify the code.

An independent reviewer replayed public CLI flows in disposable synthetic Homes
and state directories. Every suggested command was executed using its returned
`argv` unchanged, including executable, Home, state, task, hints, and Snapshot
binding. Fixture repair was explicit test setup, never an automatic CLI action.
No real Agent files or user installation were changed.

## Observed trajectories

| Workflow | Before | After |
| --- | --- | --- |
| Fresh Home: Find → suggested Scan → Find → Load | Expected initial Snapshot refusal, executable Scan, then successful load | Same successful first-use path |
| Chinese discovery with two same-name deployment Skills | Both offered comparison loads fail; a faithful but weak hint produces two more guaranteed-failing suggestions | Inspection remains available; no knowingly failing comparison load is offered. After the caller independently selects the capability, both explicit variant loads succeed |
| One healthy copy and one copy with 17 MiB of assets | Report continuation eventually points to layout findings, with zero matching findings | Inventory continuation finds the incomplete-package finding; the healthy explicit variant still loads |
| Oversized package → repair assets → Scan → Load | Already recovers after #392 | Recovery remains successful, using the same state/history |
| Unreadable entrypoint → Scan → Find → repair permission → Scan → Load | One actual redundant rescan still reports false drift | The persistent permission failure is explicit, not stale content; repair requires fresh evidence, then verified load succeeds |
| Same declared source/version with different instructions | Explicit placement loads work, but discovery omits the known content-choice blocker | Discovery says content selection is required; explicit placement loads work and unselected loading remains blocked |

The Chinese task stayed `查看 staging 服务发布进度和 Pod 状态` throughout.
`--hint 'deployment status pod'` is deliberately insufficient direct evidence.
Only after inspecting the fixture's capability did the scripted caller select
`--hint deployment-helper`; the CLI never synthesized an exact-name bypass from
Top-1. Both the original task and that deliberate hint remain in the emitted
comparison-load commands.

Across these fixed fixtures, 21 baseline advertised commands were replayed and
4 failed; 17 patched advertised commands were replayed and none failed. The
baseline also had one observed empty-layout detour and one redundant permission
rescan. These are specific observed paths, not statistical success rates,
performance measurements, or an AI-agent capability benchmark. Expected refusal
probes—ambiguous or unavailable content and missing Snapshot—are not counted as
bad advice. Harness setup scans are not counted as product friction.

## Reproduce through public CLI tests

The tests create disposable fixtures and invoke the built CLI, rather than
mocking internal ranking or loading functions:

```sh
cargo test --locked --test find_load_readiness
cargo test --locked --test find_package_availability
cargo test --locked --test find_variant_availability
```

- [`find_load_readiness.rs`](../../tests/find_load_readiness.rs) covers both weak
  CJK and hinted routes, returned-action replay, declared content choice, the
  128 KiB load limit, and healthy versus oversized comparison actions.
- [`find_package_availability.rs`](../../tests/find_package_availability.rs)
  covers repeated Scan, permission/encoding/size failures, deletion/retarget
  drift, and repair → Scan → Load without replacing state. Permission denial is
  tested as an unprivileged Unix user; privileged runners cannot model it using
  mode bits alone.
- [`find_variant_availability.rs`](../../tests/find_variant_availability.rs)
  covers mixed healthy/incomplete variants before and after Report, executes
  the inventory continuation and healthy load, and preserves drift precedence.

This demonstrates bounded discovery, truthful diagnosis, executable continuation,
and verified retrieval—not successful completion of the user's eventual task.
The `.coral-skill-version` producer contract remains unverified; this patch does
not silently discard it from identity or claim to resolve #389. The original
multi-placement Archify environment in #388 still has not been revalidated.
