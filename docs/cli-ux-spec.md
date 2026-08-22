# CLI Experience Specification

SkillRoster has two first-class callers: Agents consume stable JSON, while people may run the same commands directly in a terminal. The two surfaces share facts and safety semantics but never parse or imitate one another.

This specification adapts the useful interaction principles documented in [the Mole CLI research](research/mole-cli-design.md) without adopting Mole's Go implementation, full-screen dashboard, health score, or non-interactive cleanup behavior.

## 1. Experience goals

The CLI must feel trustworthy before it feels clever. In ten seconds, a person should understand:

1. what SkillRoster inspected;
2. what matters most;
3. whether anything changed;
4. what the safe next action is.

Presentation quality is part of product acceptance. The first complete release explicitly excludes interactive and full-screen TUI behavior: no alternate screen, panel navigation, persistent keyboard state, or terminal dashboard. It is not permission to add a GUI, HTML report, daemon, or a second navigation model.

## 2. Output modes

### Agent JSON

`--json` is the only Agent contract. It writes exactly one versioned JSON document to stdout and never emits prompts, cursor control, ANSI sequences, progress, or logs there. Agents must pass the flag explicitly; piping output does not silently change the schema.

### Human TTY

When stdout is an interactive terminal, commands may use semantic color, Unicode status marks, aligned columns, a single updating progress line, and bounded confirmation prompts. Progress writes to stderr so command data remains separable.

### Plain text

Non-TTY output, `TERM=dumb`, CI, or non-empty `NO_COLOR` disables animation, cursor control, and color. Plain output preserves the same headings, status words, IDs, totals, and next actions. Color and icons may reinforce meaning but never carry it alone.

No-argument invocation prints a compact, non-interactive home summary: product name, current state, whether recovery is required, and the three most relevant commands.

## 3. Visual grammar

Every human command follows the same five-part rhythm:

1. **Header:** `SkillRoster · <Command>` and optional current object ID.
2. **Context:** Snapshot, Agent coverage, time window, or Plan state.
3. **Body:** comparable aligned facts, ranked matches, Findings, or operations.
4. **Safety line:** explicit read-only, changed, reversible, blocked, or recovery-required status.
5. **Summary:** completion state, primary numbers, durable ID, and one next action.

Use a restrained semantic palette:

- accent: headings and current selection;
- green: verified success;
- amber: uncertainty, preview, or user attention;
- red: blocked, failed, or recovery required only;
- muted: Evidence details, paths, hints, and secondary counts.

Do not color low observed usage red. Do not show an aggregate health score. Severity always includes text such as `High`, `Medium`, or `Low`.

## 4. Responsive behavior

Human output must be checked at 60, 80, and 120 columns.

- At 120 columns, comparable Agent, Roster, count, and Evidence fields may share a row.
- At 80 columns, use one primary fact per row and wrap supporting Evidence below it.
- At 60 columns, remove decorative separators, shorten labels, and place paths on their own indented line.
- Long paths are middle-truncated in summaries; JSON and Finding drill-down retain the full value.
- Display width accounts for CJK and wide Unicode characters.

When Unicode rendering is unavailable, `✓`, `!`, and `×` fall back to `OK`, `WARN`, and `ERR`.

## 5. Progress and interruption

Long read-only work may show one TTY-only progress line:

```text
⠋ Scanning Hermes · 5/8 agents
```

Progress must describe real stages or measured counts. It must not display 100% until postconditions are complete. Non-TTY mode emits occasional static stage lines only when useful; JSON mode emits none.

Success, cancellation, interruption, timeout, and compensated failure all end with a final summary. Cursor state is restored on every exit path. A partial result states what completed, what did not, and whether any files changed.

## 6. Command experiences

### `status`

Show state-store health, latest Snapshot age, pending Plans, last Receipt, and recovery state. Do not duplicate the governance report.

### `scan`

Show Agent coverage as it progresses, then independent Skill count, placement count, inaccessible roots, and Snapshot ID. The summary ends with `Read-only · no Agent files changed`.

### `report`

Lead with four counts: independent Skills, placements, default exposure, and observed-use coverage. Show the three highest-priority Findings, then category totals. Evidence detail remains behind `report --finding`.

Agent callers use `report --summary --json` by default. The compact payload
contains those four metrics, total Finding count, category totals, and at most
three complete Finding summaries. Full `report --json` is an explicit
exhaustive view, not the bootstrap workflow default.

`report --finding ID --json` returns at most 20 affected IDs, placements, and
Evidence records per collection. Its `page.next_offset` is the only pagination
cursor; callers request another page only when the decision still lacks
relevant evidence. Counts and `primary_evidence_id` remain available in the
compact first view. An actionable exact-duplicate Finding also returns a
bounded `planning` object with at most five owned canonical candidates and the
semantic `finding_library_changes` request shape; the complete placement set
remains CLI-owned rather than Agent-copied.

### `find`

Show ranked matches with Skill name, current Roster state, source, concise match
reasons, and same-name variant count. Preserve the original task separately
from repeatable `--hint` retrieval text. A CJK task without hints receives an
actionable lexical-retrieval warning when English metadata may be missed.
Empty results are calm, successful output. Find never implies activation.

### `plan`

Show before/after Roster counts, affected Agents and Skills, operation categories, risk, reversibility, exclusions, and blocked preconditions. A ready Plan ends with its immutable ID and digest; it performs no mutation.

### `apply` and `undo`

Human TTY mode repeats the exact Plan or Receipt impact before confirmation. JSON mode never prompts because the calling Agent already obtained user approval. The final summary always includes verification, changed-path count, Receipt ID, and exact Undo availability.

### `setup`

Show detected Agents and exact bootstrap Skill targets. Installation remains a normal Plan followed by Apply.

## 7. Confirmation language

Prompts name the object and consequence; generic `Continue?` is prohibited.

```text
Apply plan_01K... to 8 Agent Rosters?
12 links create · 7 replace · 9 Skills archive
No canonical Skill content will be deleted.

Enter confirm · Esc cancel
```

Cancellation is a successful no-change outcome. Drift or ambiguity is a blocked outcome, not another prompt and never an invitation to force execution.

## 8. Reference screens

```text
SkillRoster · Scan

  OK    Agents checked       8 / 8
  OK    Independent Skills     137
  OK    Placements              212
  WARN  Findings                 31

  High      4  broken or unsafe exposure
  Medium   18  duplicate or divergent copies
  Low       9  stale or unknown lifecycle

------------------------------------------------------------
Scan complete · snap_01K...
Read-only · no Agent files changed
Next: skillroster report
```

```text
SkillRoster · Apply plan_01K...

  12 links create · 7 replace · 9 Skills archive
  8 Agents affected · reversible · drift check passed

  WARN  No canonical Skill content will be deleted.
  Undo after Apply: skillroster undo <receipt-id>

Apply this Plan to 8 Agent Rosters?  Enter confirm · Esc cancel
```

## 9. Acceptance

- Snapshot tests cover styled 60/80/120-column output and plain fallback for representative commands.
- `NO_COLOR`, `TERM=dumb`, non-TTY, and Unicode fallback preserve every semantic fact.
- JSON contract tests prove stdout contains one parseable document with no presentation bytes.
- Spinner and cursor tests prove non-TTY output is static and interrupted TTY output restores the terminal.
- Read-only, no-change, applied, compensated-failure, recovery-required, and cancelled summaries each state their filesystem effect explicitly.
- Human output never contradicts the JSON generated from the same domain result.
