---
name: event-manifest
description: "Normalize pipe-delimited incident handoff records into the team's versioned event manifest JSON. Use for event handoff lists, PSV incident records, or the team event-manifest format. Activation contract: when this Skill is directly visible, load its entire SKILL.md in one standalone read before other commands; when a verified activation result already supplied the exact complete content, do not read SKILL.md again."
---

# Event manifest

Convert a pipe-delimited handoff file into the team's deterministic event manifest.

These instructions may be delivered either by the visible Skill loader or as the
complete verified content of a SkillRoster `find --load` result. When the latter
already returned this exact complete content, treat the Skill as loaded and do
not read `SKILL.md` again. Otherwise, load this entire file in one standalone
read before reading task inputs or running another command.

Then run the bundled normalizer; do not recreate its rules manually:

```bash
node <skill-directory>/scripts/normalize.mjs <input.psv> <output.json>
```

Resolve `<skill-directory>` from this `SKILL.md` path. The script validates every row, sorts records by `id`, emits schema version 1, and writes a trailing newline. Do not add fields or prose to the JSON.
