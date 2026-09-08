# Practical governance acceptance

This change follows the review of main `bec3584` and the request to make
SkillRoster useful in ordinary work. Its acceptance boundary is three concrete
behaviors, exercised through the public CLI and the bundled Bootstrap.

1. Claude Code inventory distinguishes filesystem discovery from local catalog
   visibility. Respect `disable-model-invocation` and the documented `on`,
   `name-only`, `user-invocable-only`, and `off` settings. Preserve hidden Skills
   in the inventory and their source bytes. Report the inspected configuration
   scope; local-file observations do not prove a running session's effective
   catalog, managed settings, permission rules, or context-token cost. Unknown
   policy must remain unknown and cannot authorize an exposure-changing Plan.
   Old Snapshots must be rescanned before using their old exposure facts.
2. Skill discovery helps an ordinary task proceed. The Agent may use an already
   visible Skill or its normal tools without a mandatory search. A relevant
   search may retry once; a genuine no-match permits ordinary work to continue.
   An explicitly required Skill, unreadable content, identity drift, ambiguity,
   or a permission denial retains its own blocker. No-match is not an excuse to
   bypass a safety or authorization boundary.
3. Roster proposals disclose when the selected Core entries have no usage
   evidence. Stable name ordering is reproducibility, not a recommendation of
   usefulness. Expose this for every fallback selection, including a minority
   of the proposed Core, with a concrete review question and the existing
   protected-Skill mechanism. Preserve the deterministic ranking and explicit
   budget contract; user preference is not inferred from a new heuristic.

Validate native visibility, legacy-Snapshot handling, and report/Plan continuity
with regression tests. Exercise Scan, Report, Plan, Apply, Find, and Undo on an
isolated cross-Agent estate and compare source bytes. Check the Bootstrap on
ordinary no-Skill work and required-Skill blockers. Keep historical frozen
routing experiments unchanged.

Independent-user comprehension, recommendation acceptance, task-quality gains,
and measured time/token savings remain separate evidence. This change does not
complete the independent pilot in #261 or authorize participant outreach.

Native reference contracts, checked 2026-09-08:
- https://code.claude.com/docs/en/skills#control-who-invokes-a-skill
- https://code.claude.com/docs/en/skills#override-skill-visibility-from-settings
- https://code.claude.com/docs/en/settings#settings-precedence

The local projection inspects `HOME/.claude/settings.json`. For an explicitly
included conventional project `.claude/skills` root, it also reads that root's
adjacent `settings.json` and `settings.local.json`, in that order. It uses the
placement directory name for personal/project override keys. It does not resolve
an active session's cwd, worktree settings inheritance, `CLAUDE_CONFIG_DIR`,
managed policy, plugin catalog, collision precedence, or CLI overrides. An
explicit enabling setting combined with `disable-model-invocation: true` is
unknown because the reference does not establish their precedence. Unknown
entries remain in the possible-exposure count and block affected Roster changes.
Native settings are read with a 256 KiB bound, regular-file checks and no final
symlink following; only visibility overrides and file observations are persisted.
Unreadable or malformed configuration leaves the inventory available, with the
affected root's visibility marked unknown. After fixing the configuration,
rescan before preparing an exposure-changing Plan.

## Validation recorded for this change

`bash scripts/ci-full-check.sh` passes locally: 483 Rust tests and 152 Node
harness tests, with one existing ignored Rust test. Formatting, strict Clippy,
README evidence, installation-surface and release-archive checks also pass.

`native_visibility_public_governance_lifecycle_preserves_sources_and_policy`
uses a temporary cross-Agent estate with 54 placements. The local exposure count
is 52 before the reviewed budget/protection request and 3 after Apply. The two
native-hidden entries remain intact, an On-demand Skill loads completely, a
settings change blocks the old Plan, and Undo restores every original Skill
file byte-for-byte. These are fixture results, not a token or task-quality gain.

Setup tests cover exact released Bootstrap packages 1.8.23 and 1.8.29 through
upgrade and Undo. The 1.8.29 fixture and complete-package digests come from release
tag `v1.8.45`, verified against the starting main tree. Partial or locally modified
packages retain the existing user-choice boundary.

The Bootstrap's ordinary-task, optional no-match, required-Skill and typed-blocker
branches were reviewed against the instruction source. This is an instruction
review, not a new model-driven task-success experiment. Historical frozen routing
protocols and the independent-user pilot remain separate validation.
