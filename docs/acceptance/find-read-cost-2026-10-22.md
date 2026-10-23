# Ordinary Find read cost

Date: 2026-10-22. Base: `acb4710`; CLI version remains 1.8.46.

## Observed regression

Adding placement-bound content choices made ordinary Find repeat full-package
fingerprinting. For a healthy Skill with one placement, path discovery, drift
detection, choice rendering and Top-1 comparison actions traversed the same
package four times. Ten singleton matches required 31 traversals. The JSON also
included a detailed choice bundle and comparison action when no choice existed.

An isolated fixture contained 100 uniquely named Skills, each with one 1 MiB
supporting file. The exact-name query returned one match; the broad query used
the default ten-result limit. Each query ran five times against the same
Snapshot before and after the change, without rescanning or rewriting packages.

| Fixture and query | Median before | Median after | Response before | Response after |
| --- | ---: | ---: | ---: | ---: |
| 100 MiB support, exact | 329.7 ms | 118.0 ms | 2,975 B | 1,135 B |
| 100 MiB support, broad | 2,314.5 ms | 791.3 ms | 18,956 B | 6,968 B |
| Empty support, exact | 36.6 ms | 40.8 ms | 3,021 B | 1,149 B |
| Empty support, broad | 44.6 ms | 54.6 ms | 19,254 B | 7,108 B |

These are local debug-binary measurements with operating-system caches and host
workload uncontrolled. They establish a package-size-dependent regression and
its improvement in this fixture, not production latency or universal speedup.
The empty-file control did not show a latency improvement.

## Change

- Reuse advisory package inspection results by canonical entrypoint within one
  Find invocation. Every placement retains its own authority, approved-root,
  physical-directory, entrypoint and expected-digest checks. Complete same-name
  variant checks still include identities outside the displayed preview.
- Preserve original logical-package inspection when a `SKILL.md` file symlink
  resolves outside its containing package. Physical-path reuse cannot silently
  change which package is fingerprinted.
- Discard observations at command end. Actual verified loading independently
  rereads and validates the package; it cannot use an advisory cached digest to
  authorize instructions.
- Build each displayed multi-placement identity's content choice once. Omit
  singleton choices. Equivalent multiple copies keep their comparison facts,
  but require no explicit-placement action.
- Offer exact-placement actions only for divergent content. In a mixed same-name
  group, a single-content identity keeps its Skill-ID action, while an identity
  with conflicting contents offers the necessary placement actions directly.

All twenty measured after responses matched the corresponding before response
after removing only the per-run ID, deliberately omitted singleton choice bundle
and redundant comparison action. Ranking, paths, source facts, warnings and
Snapshot IDs were unchanged.

## Reproduction

The standalone standard-library benchmark creates a disposable Home, state store
and source directory, runs Scan once, then measures ordinary exact and broad
queries. It cleans up the fixture and has no timing gate:

```sh
cargo build --locked --offline
python3 scripts/benchmark-find.py
python3 scripts/benchmark-find.py --support-bytes 0
```

`--binary`, `--skills`, `--support-bytes` and `--trials` allow explicit comparisons.
Use the same build profile and machine when comparing binaries. Its JSON reports
the CLI version, fixture sizes, individual timings, medians, response bytes and
match/choice/action counts. Generated temporary paths vary, so independent runs'
response byte counts can differ slightly.

## Validation and boundary

- `bash scripts/ci-full-check.sh`: passed; 556 Rust tests passed with one existing
  ignored test, 152 Node tests passed, formatting, strict Clippy, README evidence,
  installation surface and release archive checks passed.
- Build and public help passed.
- A new unit regression first populates an advisory observation, changes only a
  supporting file, then confirms actual loading refuses package drift. Fresh
  observations also detect the change.
- Nine public content-selection regressions passed, including singleton loading,
  drift on the next Find, equivalent-copy detail and actual replay of all three
  useful commands in a mixed same-name/content-conflict group.
- Benchmark smoke checks passed with three Skills and both nonempty and empty
  supporting files; a zero Skill count is rejected. The default 100-Skill,
  100 MiB-support benchmark also completed successfully.
- Independent review found no actionable issue in the read-reuse and recovery
  changes. Adversarial replay detected support drift in the eleventh and twelfth
  undisplayed same-name variants; explicit drifted-placement loading returned no
  instructions while a healthy sibling retained its comparison action.
- `git diff --check`: passed.

This is local optimization and validation. It makes no new claim about model
task success, token savings, merge, publication or installed-binary upgrade.
