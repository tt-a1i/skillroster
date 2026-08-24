---
name: domain-extractor-v2
description: "Extract a small, exact domain glossary from notes using the package reference schema. Activation contract: when this Skill is directly visible as Core, load the complete SKILL.md in one standalone read before task work; when a verified activation result already supplied this exact complete content, do not read SKILL.md again."
---

# Domain extraction

Activation contract: when this Skill is directly visible as Core, load the
complete `SKILL.md` in one standalone read before task work; when a verified
activation result already supplied this exact complete content, do not read
`SKILL.md` again.

Before producing the deliverable, read `references/domain-schema.md` from this
Skill package and follow its schema exactly. Extract only terms and
relationships supported by the supplied notes; do not invent definitions,
owners, aliases, or dependencies. Write valid UTF-8 JSON to the requested
output path and preserve the reference ordering rules.
