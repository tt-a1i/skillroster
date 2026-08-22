# Relevant Manager Execution Run

Executed on 2026-08-22 in an isolated temporary home and project. The input was
the repository's ten synthetic routing Skills. No user Skill, session, config,
credential, or Agent root was read. `GITHUB_TOKEN` and `GH_TOKEN` were removed,
and `DISABLE_TELEMETRY=1` plus `DO_NOT_TRACK=1` were set.

## Environment and procedure

- Vercel `skills` 1.5.23; Node 26.3.0; npm 12.0.2.
- SkillRoster 1.0.0 release-candidate working tree.
- The package was fetched once from npm into a temporary cache. Every manager
  operation below used `npx --offline`; local-source installation performed no
  repository fetch or login.

```bash
npx --offline --yes skills@1.5.23 add ./source \
  --skill '*' --agent codex --agent claude-code --yes
npx --offline --yes skills@1.5.23 list
npx --offline --yes skills@1.5.23 remove --all
```

The manager installed ten canonical packages under `.agents/skills`, ten links
under `.claude/skills`, and a `skills-lock.json` containing ten records. Its
list command reported the installed placements. `remove --all` removed all 20
managed placements, preserved all ten local-source packages, and retained the
lock file. Reinstallation reproduced the same layout.

SkillRoster then scanned those exact manager-owned roots:

```bash
skillroster --root codex=$PROJECT/.agents/skills \
  --root claude-code=$PROJECT/.claude/skills --json scan
skillroster --json report
```

It found 10 independent Skills, 20 placements, 20 default exposures, no scan
warnings, and 15 traceable Findings. A versioned Roster Plan kept the ten Codex
canonicals Core and moved the ten Claude Code links On-demand. The Plan had 11
derived operations, zero canonical deletions, and required one Apply. Apply
verified 11 changed paths and reduced default exposure from 20 to 10. `find`
still returned a readable path for an On-demand Skill. Receipt-bounded Undo
verified all 11 reverse changes.

The before/after manifest contained path, file type, link target, mode, and
SHA-256 for files. All 32 entries matched exactly after Undo.

## What the run proved

Vercel `skills` is effective at acquisition, a shared source, cross-Agent
linking, lock records, listing, and removal. SkillRoster adds estate-wide
Findings, per-Agent exposure states, local routing, immutable Plans, canonical
deletion accounting, Receipts, and exact restoration without taking over the
manager's source acquisition role.

The integration run also exposed a real boundary bug: a recoverable move of an
Agent symlink was initially normalized to its canonical target. The Plan failed
closed before mutation. SkillRoster now validates and moves the symlink entry
itself; a dedicated regression test covers Apply and Undo. This is evidence of
the comparison's development value, not a claim that one synthetic run proves
general superiority over other managers.

Ponytail was not part of this product run and remains only a development
complexity fixture.
