# Packaged fresh-user governance journey — v1.8.29

Date: 2026-08-29 (Asia/Shanghai)

Issue: [#259](https://github.com/tt-a1i/skillroster/issues/259)

Published release: [v1.8.29](https://github.com/tt-a1i/skillroster/releases/tag/v1.8.29)

## Question

Can an Agent start from the published package and a natural-language governance
request, install the Bootstrap through a previewed Plan, diagnose a fresh
synthetic estate, prepare one decision-complete Roster Plan, Apply it after one
Roster Plan authorization, retrieve an On-demand Skill exactly, and Undo the governed estate
byte-for-byte?

This is a packaged first-use proof, not a source-tree smoke. It records the
installation, Bootstrap, diagnosis, Plan, confirmation, Apply, Receipt,
recovery, On-demand retrieval, and Undo boundaries separately.

## Frozen boundary

- The macOS arm64 archive was downloaded anonymously from the v1.8.29 Release.
  Its adjacent checksum verified archive digest
  `e79376eafc6541d80b27ccb861f041fca38e1ccf51b42cd130af191df872de09`.
- The extracted binary reported `skillroster 1.8.29`; no source-tree binary ran.
  The archive contained only `LICENSE`, `README.md`, and the binary under its
  target directory.
- Each run used a fresh temporary Home, state directory, Codex root, Claude Code
  root, raw-output directory, and evidence directory. The fixture contained 19
  repository-owned synthetic Skills and 12 same-content cross-Agent copies.
- After Setup, the Agent read the installed 83-line Bootstrap, 132-line
  governance reference, and 51-line mutation/recovery reference completely
  before continuing. Both accepted runs used the same verified Bootstrap bytes.
- The active natural-language Agent request authorized governance only inside
  that synthetic scope. The public record stores a bounded projection of the
  request, not the raw conversation.
- No real Agent, Skill, session, configuration, or state directory was supplied
  to the product. Raw command envelopes and opaque local IDs stayed in the
  external temporary evidence directory.

## Stage result

The frozen complete journey passed twice on fresh isolated state:

| Stage | Run 1 | Run 2 |
| --- | --- | --- |
| Installation | Published v1.8.29 checksum and version passed | Same artifact passed |
| Bootstrap | Preview then confirmed, verified Apply to Codex and Claude Code; Receipt present; Bootstrap and required references read to EOF | Same |
| Diagnosis | 20 Skills, 33 placements, 33 default exposures | Same |
| Evidence boundary | All eight session roots missing; no unused claim | Same |
| Plan | Ready; complete detail reviewed; Codex `agent-session-miner` On-demand; 33 → 32; 2 operations | Same |
| Roster Plan confirmation | One ticket-scoped authorization for the exact synthetic Plan; zero per-operation prompts | Same |
| Apply / Receipt | Verified; 2 changed paths; zero canonical deletion; Undo available | Same |
| On-demand retrieval | Rank 1 exact load; identity and package fingerprint matched | Same |
| Recovery | Clear; zero journal issues | Same |
| Undo | Confirmed and verified; 2 changed paths | Same |
| Byte ledger | 76/76 records identical; zero add/remove/change | Same digest |

The ledger baseline was captured after the Bootstrap was installed and before
the Roster Apply. It therefore proves exact restoration of the governed Agent
estate, while the Bootstrap installation remains a separately reported,
Receipt-backed first-use stage. Both before and after digests were
`4c1674f8dbfa10c35641b8beb238b9c8d923ebfb0167d672000b6d464486f282`.

## Agent friction and decision

Two preliminary isolated runs froze the acceptance protocol and are disclosed
but excluded from the pass claim. The first stopped three times before its
Roster Apply because the Agent assumed remembered response shapes instead of
reading the v1.8.29 envelopes: Setup correctly installed Bootstrap for both
detected Agents, Find used `loaded_skill`, and Plan used flat `impact` fields.
Those corrections caused no governance mutation. A second preliminary run
confirmed the corrected field contract. The two accepted runs then started
from fresh state, read the installed Bootstrap and required references to EOF,
and completed the frozen sequence without correction.

This is useful integration feedback, but it does not justify another runtime
feature: the Bootstrap already requires callers to validate `schema_version`,
`ok`, and the returned typed result instead of scraping prose or assuming a
newer schema. The durable gap was the missing privacy-safe, stage-separated
acceptance record; this document and its machine ledger close that gap without
adding a workflow engine or duplicate harness.

## Evidence and limitations

The privacy-safe machine record is
[`packaged-fresh-user-governance-v1.8.29.json`](artifacts/packaged-fresh-user-governance-v1.8.29.json).
It contains no absolute paths, Skill bodies, raw conversation, credentials,
opaque local IDs, or raw command output.

The run proves the packaged macOS arm64 first-use path on deterministic
synthetic data and one repeated reference-platform execution. Cross-platform
package smokes remain separate release evidence. It does not prove independent
new-user comprehension, recommendation preference, token or labor savings,
model-quality improvement, or universal Core-versus-On-demand superiority.

The raw Roster Plan in each accepted run cited Evidence that covered the
changed Skill. This journey does not
prove that v1.8.29 rejects unrelated Evidence in a raw Roster request. That
separate defect was reproduced from a later public package and fixed by
[#303](https://github.com/tt-a1i/skillroster/issues/303) on `main` after
v1.8.30; it is not retroactively claimed for the v1.8.29 bytes tested here.
