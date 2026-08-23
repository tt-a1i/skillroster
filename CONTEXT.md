# SkillRoster Domain

SkillRoster governs locally installed Agent Skills. This glossary fixes the product language used by documentation, CLI output, storage, and tests.

## Collection and exposure

**Skill**:
A local capability package whose entry point is a `SKILL.md` file.
_Avoid_: Tool, plugin, prompt

**Skill Identity**:
The stable identity of a Skill, derived first from declared source and version, then from content. Names and semantic similarity identify candidates but never prove identity.
_Avoid_: Skill name, folder name

**Library**:
The complete logical collection of Skills known to SkillRoster, regardless of where their canonical files physically reside.
_Avoid_: Registry, marketplace, central folder

**Roster**:
The curated set of Skills exposed to one supported Agent. A Roster is a view of the Library, not another copy.
_Avoid_: Install list, enabled folder

**Core Skill**:
A Skill intentionally exposed by default because it is broadly and repeatedly useful.
_Avoid_: Hot Skill, always-on Skill

**On-demand Skill**:
A Skill omitted from the default Roster but discoverable through local search for relevant tasks.
_Avoid_: Disabled Skill, cold Skill

**Explicit-only Skill**:
A Skill used only when a person or Agent names it explicitly, usually because it has narrow or material side effects.
_Avoid_: Manual Skill

**Archived Skill**:
A retained, recoverable Skill excluded from normal exposure and routing.
_Avoid_: Deleted Skill, unused Skill

## Governance

**Observed Skill**:
A discovered Skill that SkillRoster can inspect but does not manage or relocate.
_Avoid_: Unmanaged Skill

**Managed Skill**:
A Skill whose exposure, links, or configuration SkillRoster may change through an approved Plan while its files remain at their current canonical location.
_Avoid_: Linked Skill

**Hosted Skill**:
A Skill explicitly migrated into SkillRoster's managed Library root.
_Avoid_: Imported Skill, owned Skill

**Evidence**:
A traceable fact supporting a Finding, such as a path, source declaration, symlink target, exposure record, or local usage event.
_Avoid_: Proof, confidence

**Usage Observation**:
A privacy-preserving aggregate for one Skill, Agent, usage stage, quality, and
session-source path. Its high-water count is stable across Scans; lifecycle
history records only newly observed deltas and never raw conversation text.
_Avoid_: Scan hit, invocation log

**Finding**:
An evidence-backed condition discovered during analysis. A Finding describes a condition; it does not authorize a change.
_Avoid_: Error, recommendation

**Plan**:
An immutable, validated preview of an exact set of proposed local changes.
_Avoid_: Script, action list

**Receipt**:
The durable record of an applied Plan and the information required to verify or undo SkillRoster-owned changes.
_Avoid_: Log, history entry
