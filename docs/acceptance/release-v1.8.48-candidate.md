# SkillRoster v1.8.48 candidate preparation

Date: 2026-10-28. Intended CLI **1.8.48**, bundled Bootstrap **1.8.51**.

This is a candidate preparation record, not a publication receipt. No v1.8.48
candidate acceptance, formal tag build, public artifact, checksum, Homebrew
availability, or installed-user upgrade is claimed here. The current published
release remains v1.8.47 until publication and independent readback succeed.

## Source and changes

The preparation starts from merged main
`c1507fd4764bb90bb9e2f1119f56b1110e42a6d8`, containing
[PR #392](https://github.com/tt-a1i/skillroster/pull/392).
The release source SHA must be recorded after the version preparation is reviewed
and committed; the base SHA above is not the candidate's final SHA.

- [#390](https://github.com/tt-a1i/skillroster/issues/390): ordinary Find reports
  incomplete package fingerprints accurately, including the 16 MiB package
  budget, instead of sending fresh oversized packages into a futile rescan loop.
  Verified loading still refuses incomplete package contents.
- [#391](https://github.com/tt-a1i/skillroster/issues/391): Find exposes load
  readiness and read-only recovery for insufficient direct-selection evidence.
  Ranking a Skill first does not authorize loading it; CJK and hinted weak-match
  protections remain in place.
- After a package is repaired and rescanned, candidate retrieval filters SQLite
  FTS results to the current Snapshot before applying its result bound. Historical
  identities no longer displace the repaired current identity.
- [#388](https://github.com/tt-a1i/skillroster/issues/388) is only partially
  addressed: the isolated oversized-package failure is covered, but the original
  multi-placement Archify environment has not been revalidated by this release
  preparation. No claim is made that every original placement is repaired.
- [#389](https://github.com/tt-a1i/skillroster/issues/389) remains unresolved.
  The `.coral-skill-version` installer contract has not been verified, so this
  marker is not excluded from routing identity or integrity checks. A regression
  preserves this fail-closed boundary; it is not a metadata-equivalence fix.

Bootstrap content is unchanged, so its version remains 1.8.51. No release
workflow permissions, Formula, public install command, or user environment is
changed by this preparation.

## Validation and remaining gates

The merged bug-fix validation belongs to PR #392. It does not substitute for
validation of the final versioned release SHA. Preparation checks and eventual
workflow results must remain separate from public-delivery verification.

Pending for the final release SHA:

1. Locked formatting, strict Clippy, all-target/all-feature Rust tests, and the
   repository's applicable full validation guards.
2. Candidate workflow acceptance on Linux x86_64, Windows x86_64, macOS arm64,
   macOS x86_64, and the checksum-pinned WSL2 boundary; download and verify all
   four archives and their adjacent checksums.
3. Explicitly authorized annotated formal tag and its accepted tag build.
4. GitHub Release publication with the four accepted archives and four checksums,
   followed by independent anonymous downloads and extracted-archive first-use
   verification. Record exact tag/SHA, workflow links, asset hashes, and outcomes.
5. Update Formula and public documentation only after publication; verify
   Homebrew delivery independently. Do not infer installed-user upgrades.

No real Agent configuration, Skill package, local CLI installation, or user
computer was modified by this candidate preparation.
