# Pi cold-routing canary ledger

Date: 2026-08-24 (Asia/Shanghai)  
Scope: diagnostic canary for Issues #129 and #130; not the full paired pilot.

## Frozen inputs

| Input | Value |
|---|---|
| Pi | `0.84.2` |
| Model | `seal/deepseek-v4-flash-0731-baidu` |
| CLI commit | `e16d563` |
| CLI binary SHA-256 | `0baa23cd2f099b9b4a94c84fd59f3df8dd446dc47ccc320a578bfcd88f6da539` |
| Snapshot | `scan_000000000001166718ce7ac23556d9d0` |
| User-message SHA-256 | `70eb69bba67a41e1f549318b1c76578e3ba0bff79dbfa4b3555ffbaf4ddb4846` |
| Target Skill SHA-256 | `9b83a71b5934c8fc744bf0f6b3ea35702b15b0cb36e643b4532e08922477fe65` |

The target was present only in the isolated source Library. CLI preflight
ranked it first; no supported Agent root exposed it.

The invoked CLI path was
`/Users/tushaokun/code/skillroster/target/debug/skillroster`, resolved by
prepending its directory to `PATH`. The recorded binary digest was captured
immediately after the run; that development binary was not retained and later
test builds may replace it. Pi used isolated `PI_CODING_AGENT_DIR`,
`SKILLROSTER_STATE_DIR`, and `SKILLROSTER_HOME` values, `--offline`, a fresh
`--session-dir`, explicit Bootstrap `--skill`, and only the `read,bash` tools
under `sandbox-exec`.

## Paired observation

The governance-first Bootstrap at `origin/main` had SHA-256
`11c9074ec790c4ce7820aa9bb8fe112b2fba607f847c3efbfca268e6686f54cf`.
Its fresh transcript (SHA-256
`1a4776006335d0c0941b892dfc12ea8d0edbf912e457c4118a82646ecf15b384`)
ended at `no_retrieval_call`.

The route-first Bootstrap had SHA-256
`fc1318f0a8587ce193ab1f54462374508026cf8c50a382b613dc18b48f9d09f2`.
Its fresh transcript (SHA-256
`32111bed04c5e24ade7b082b82c7fe9803cefb129d44146a30614a9a860ad80a`)
loaded that exact repository file, then produced this evidence chain:

1. `skillroster find` received the complete original Chinese message and the
   faithful hint `convert a Lunar Registry record into its canonical label`.
2. JSON reported `task_hint_reciprocal_rank_fusion`, `files_changed: false`,
   and `lunar-registry` at rank 1.
3. Pi read the exact returned cold `SKILL.md` path.
4. The deterministic oracle matched `selene:a3:e9`.

Classification: `task_succeeded`; retrieval and load both correct.

The raw transcripts were intentionally not committed because they include model
reasoning and harness context. Their hashes bind the locally retained originals;
the committed [redacted event artifact](artifacts/cold-routing-canary-v1.8.20.json)
preserves the review-relevant inputs, tool sequence, structured Find facts,
oracle result, and limitations.

## Safety boundary

Pi ran in a fresh session with only `read` and `bash`. A tool-call extension
blocked Bash except SkillRoster Find and limited reads to the isolated fixture
plus the repository Bootstrap. The observed transcript stayed within those
paths. A macOS sandbox denied writes beneath `/Users/tushaokun`; no real Agent,
Skill, repository, or configuration file changed. The temporary extension used
string-prefix path checks, so this canary does not establish a general sandbox
security claim. The full pilot must canonicalize permitted paths first.
