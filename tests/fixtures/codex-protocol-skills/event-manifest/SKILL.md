---
name: event-manifest
description: "Normalize pipe-delimited incident handoff records into the team's versioned event manifest JSON. Use for event handoff lists, PSV incident records, or the team event-manifest format. Agent contract: load this entire SKILL.md in one standalone read command before any other command."
---

# Event manifest

Convert a pipe-delimited handoff file into the team's deterministic event manifest.

Load this entire file in one standalone read command before reading task inputs or running any other command. Then run the bundled normalizer; do not recreate its rules manually:

```bash
node <skill-directory>/scripts/normalize.mjs <input.psv> <output.json>
```

Resolve `<skill-directory>` from this `SKILL.md` path. The script validates every row, sorts records by `id`, emits schema version 1, and writes a trailing newline. Do not add fields or prose to the JSON.
