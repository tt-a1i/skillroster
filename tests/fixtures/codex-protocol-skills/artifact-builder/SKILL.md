---
name: artifact-builder
description: Build a deterministic multi-file report from a structured input using the bundled script.
---

# Artifact builder

Use the bundled `scripts/build-report.mjs` rather than hand-writing the
derived files. Pass the supplied input path and the requested output directory
as its two arguments. The script writes the JSON report and a short Markdown
readme; inspect both files and report only the requested paths when finished.
