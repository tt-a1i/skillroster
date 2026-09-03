# SkillRoster v1.8.45 candidate

This is source preparation, not a published release receipt. The public
release and Homebrew Formula remain v1.8.44. No v1.8.45 tag, public Release,
or Homebrew update is established by this document.

## User-visible changes

- Semantic-overlap analysis retains at most 25 candidates while preserving
  the previous ranked output. The recorded dense 5,000-Skill synthetic
  workload changed from 7,752 ms to 1,558 ms, with process peak RSS changing
  from 339,918,848 to 38,338,560 bytes. These are single-machine measurements,
  not a universal latency promise; pair enumeration remains quadratic.
- Recovery acceptance now injects storage-full/I/O errors and terminates
  Apply subprocesses at journal and target-publication checkpoints. A fresh
  process must preserve recovery evidence and refuse further mutation.
  These tests do not simulate physical power loss or hardware cache behavior.
- Copy and Replace preserve supported observable metadata or fail closed.
  Recursive copies finalize directory modes after copying children; Windows
  copy handles retain directory rename protection. Replace/Undo retain their
  mode/readonly guarantees and refuse observable metadata drift.

The exact platform metadata matrix, unsupported layouts, experiment setup,
and regression limitations are in [hardening evidence](../hardening-evidence.md)
and [acceptance evidence](../acceptance.md#synthetic-overlap-scale-baseline-release-hardening-round).

## Compatibility

CLI source version is 1.8.45; bundled Bootstrap content remains 1.8.29.
SQLite and JSON envelope schema versions are unchanged. Unsupported ACL,
xattr, stream, or ownership layouts may now be refused instead of silently
losing metadata. Windows legacy replacement Receipts lacking original
security evidence need explicit manual recovery; other Undo operations and
Unix Receipts are not invalidated by that policy. Preservation of timestamps,
hard-link topology, privileged/invisible metadata, and Windows SACL settings
is not claimed.

## Source and verified fix

Preparation starts at `main@eb38952c49d8c4992129ea126ab44a2f9e480887`, the
merge of [PR #373](https://github.com/tt-a1i/skillroster/pull/373). It closes
issues #370, #371, and #372. The fix was independently reviewed on Standards
and Spec axes at exact head `8c9b0c0228f6c5acb9fddd56b32c180332cd74e3`.
Both axes had zero remaining findings.

That head passed the complete local gate: 341 unit tests, 8 acceptance tests,
122 CLI tests, 152 Node tests, strict Clippy, formatting, and installation /
archive documentation-surface checks. Its
[PR CI](https://github.com/tt-a1i/skillroster/actions/runs/33754750269)
passed Linux x86_64, Windows x86_64, both macOS architectures, and CI gate
using Rust 1.85. This evidence belongs to the fix head, not to an as-yet
unverified candidate build or future tag.

## Remaining publication gates

- Verify and review this version-preparation change at its exact commit.
- Build candidate archives with the exact-SHA strict gate, four platform
  jobs, and WSL2 smoke; independently verify checksums, README, and LICENSE.
- Resolve [historical asset policy #55](https://github.com/tt-a1i/skillroster/issues/55).
  The 21-release / 168-attachment inventory and backup are complete, but
  retirement versus repackaging still requires a decision. No historical
  attachment has been removed or replaced by this preparation.
- Publish the final tag and GitHub Release, update Homebrew from verified
  source/artifact checksums, and read back public installation results.

Independent user-pilot work is outside this round. Version preparation does
not change the user's installed binary, state, Agent files, or Skill library.
