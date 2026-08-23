# Pi cold-routing paired pilot

Date: 2026-08-24

Issue: [#129](https://github.com/tt-a1i/skillroster/issues/129)

Scope: Pi reference harness only; not Codex or Claude Code acceptance.

## Result

The sealed holdout was executed once and **failed its overall task-success gate**. It still produced positive, narrower evidence for SkillRoster's cold-routing mechanism:

| Gate | Result |
| --- | --- |
| Arms actually executed | 8/8 |
| Core task success | 2/4 — failed |
| On-demand task success | 2/4 — failed |
| On-demand retrieval and target load | 4/4 — passed |
| Retrieval attempts | 1 per On-demand arm |
| Safety | 8/8 — passed |
| Contract violations | 0/8 — passed |

The architecture and Chinese-naturalization pairs passed in both arms. The domain-terminology and session-mining Core controls failed, so their On-demand task failures cannot be attributed to cold routing. All four On-demand arms used the frozen Bootstrap routing surface, followed its routing contract, retrieved the expected target, and loaded it from zero default exposure.

## Frozen evidence

- Formal model: `seal/gpt-5.6-sol`. DeepSeek Baidu 0731 was used only as a fast development canary and is not counted as gate evidence.
- Implementation revision: `5aef302731b2d0a53df915ba386addf1a65ca3c8`.
- Seal first-add revision: `ea0a6cc791d1c270bd0a016e2a33d1b17c331c32`.
- Seal payload digest (`seal_sha256` field): `d1a9441e18df7f6860463f178d1fb203d99fc3794f5eedd95f529a0266bc1e5d`.
- Seal file digest: `96ae718ea221508801767ef0261753f76152bf00241a87cd610428dd3b95ca4f`.
- Holdout suite snapshot: `a58844c1ccb95c21976ec0efa561b295de830e9e660fcd8b6d0f6070a7b70d80`.
- Training v10 previously passed 8/8 arms; its snapshot was `3371eb579ed300e275df7d8b8c8283e2994e6153047ada37eeb7fd3cf24a36fc`. Training is not holdout evidence.

The seal binds the manifest, complete Bootstrap package, CLI binary and source tree, runner/gate, target packages, materialized workspaces, evaluation contracts, model profile, and fixed arm schedule. Its Git first-add check prevents re-signing the same suite ID. Pi authentication and private model configuration are excluded.

## Failure adjudication

Post-run diagnosis does not alter the failed result:

- Domain terminology was a scorer false negative. Both arms grouped multiple retired terms under one explicit retirement relation; the frozen regex accepted only the first item in each list.
- Session mining had one real shared omission: both arms generalized away a required concrete technical term. A second Core-only miss expressed the intended event-coalescing fact without matching the frozen lexical pattern.

Four arms recorded seven nonfatal policy denials, all reads of nonexistent files blocked before access. There were no unsafe writes or input mutations. Eight ledgers were produced, and temporary Pi authentication files were removed. Transcripts, generated workspaces, and private configuration are intentionally not committed.

The machine-readable, redacted summary is in [pi-cold-routing-pilot-v2.json](artifacts/pi-cold-routing-pilot-v2.json).
