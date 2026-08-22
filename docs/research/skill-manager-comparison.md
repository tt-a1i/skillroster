# Skill governance baselines and manager comparison

Research snapshot: 2026-08-22. Public factual claims below use first-party
documentation or source at pinned revisions. This is a product-boundary study,
not an endorsement or security audit.

## Executive conclusion

Skill installation and Skill governance are different jobs. Existing tools are
already strong at acquiring Skills, placing them in known Agent directories,
sharing one copy through links, and updating known sources. SkillRoster should
interoperate with those layouts rather than replace their package-management
work.

The unresolved user problem starts *after* months of unmanaged, manual, and
manager-driven changes: identify all independent Skills and placements, explain
duplicates and broken or escaping links, qualify local usage evidence, reduce
default exposure without making Skills undiscoverable, and apply an approved
cross-Agent organization with a receipt and bounded undo. Those are
SkillRoster's differentiators.

The common Agent Skills specification defines a `SKILL.md` package, optional
resources, and progressive disclosure. It says metadata for every Skill is
loaded at startup and full instructions are loaded on activation; it does not
define installation ownership, lifecycle records, usage evidence, rosters, or
undo semantics ([specification](https://agentskills.io/specification)). That
gap explains why filesystem layout and governance need a separate model.

## Baselines

### 1. Unmanaged directories

Users copy or clone Skills independently into each Agent's discovery root.
This has no additional tool dependency, but provenance, revisions, and
ownership are usually implicit. Exact content can be duplicated under several
paths, and a same-named Skill may have diverged. Deletion or replacement has no
general receipt. This is the baseline SkillRoster must diagnose without first
requiring migration.

### 2. Careful manual governance

A disciplined user keeps a canonical local directory and creates per-Agent
symlinks, records sources and revisions, periodically checks links, and
manually curates which Agent sees which Skill. This can be private, efficient,
and immediately consistent, but its quality depends on conventions and
operator memory. A representative first-party repository demonstrates this
pattern with a script that rebuilds `.agents/skills/<name>` links while
preserving conflicting non-link paths
([gohypergiant/agent-skills](https://github.com/gohypergiant/agent-skills/blob/68f6314447139c4644c87450ea44a7e3fbf27df7/scripts/symlink-agent-skills.sh)).

Manual governance is the strongest fair baseline for SkillRoster: the product
must add evidence, repeatability, cross-Agent normalization, and recovery—not
merely automate `ln -s`.

### 3. Relevant managers

| Capability | Vercel `skills` | `skm` | Swival library | mode-io Skill Manager | SkillRoster boundary |
|---|---|---|---|---|---|
| Primary job | Add, use, find, list, remove, update Skills | Declarative global install and linking | Stage, review, then activate Skills for Swival | Local app for shared Skills plus MCP/commands | Diagnose and govern existing local estates |
| Shared source | Canonical `.agents/skills`, links to Agent roots; copy fallback | Store plus per-Agent symlinks or hardlinks | Staged library is inactive; activation copies into project/global roots | Shared local package plus symlink/junction bindings | Logical Library; canonical files may remain in place or be explicitly hosted |
| Source/update record | Project/global lock data and content/tree hashes | YAML config, lock file, remote update checks | Collection URL/ref in staged library | Source-backed adoption/update | Source, revision, content hash, local-modification decision |
| Existing unmanaged estate | Lists discovered installed placements | `list --all` marks managed vs unmanaged | Lists Swival active/library roots | Adopts supported harness folders | Scans all eight supported Agent roots plus explicit paths |
| Exposure model | Install targets/scopes | Package include/exclude filters | Staged vs project-active vs global-active | Enable/disable per harness | Core, On-demand, Explicit-only, Archived per Agent |
| Usage/routing evidence | Keyword discovery; no documented local-session governance surface | No documented local-session governance surface | Progressive disclosure and explicit `$skill` activation | Optional configured LLM scan can send selected context | Local-only staged usage evidence, coverage labels, ranked `find` |
| Change safety | Confirmation options; install/update/remove operations | Idempotent relinking and stale-link removal | Refuses ambiguous collection names; `--force` can replace | Local adoption/enable/disable/update/delete operations | Immutable Plan, drift refusal, journal, Receipt, bounded Undo |
| Privacy/network boundary | Remote acquisition and anonymous telemetry unless disabled | Remote clone/fetch for repository packages | Remote clone for staging | Optional LLM scans send selected Skill context to configured provider | No cloud, account, telemetry, daemon, or model call; user data remains local |

Evidence and interpretation:

- Vercel's CLI documents symlink as the recommended single-source mode, copy as
  fallback, project/global scopes, Agent targeting, JSON listing, discovery,
  update and removal. Its source records source metadata and SHA-256 content
  hashes in `skills-lock.json`; its README also documents anonymous telemetry
  and opt-out environment variables
  ([README](https://github.com/vercel-labs/skills/blob/435076e78988e1e6ec40d00b0b1d76bdbbc5419a/README.md),
  [list source](https://github.com/vercel-labs/skills/blob/435076e78988e1e6ec40d00b0b1d76bdbbc5419a/src/list.ts),
  [lock source](https://github.com/vercel-labs/skills/blob/435076e78988e1e6ec40d00b0b1d76bdbbc5419a/src/local-lock.ts),
  [telemetry source](https://github.com/vercel-labs/skills/blob/435076e78988e1e6ec40d00b0b1d76bdbbc5419a/src/telemetry.ts)).
- `skm` explicitly manages user-level Skills from one YAML configuration,
  clones or links sources into a store, links them to Agent directories, writes
  a lock, removes stale links idempotently, distinguishes managed Skills in
  `list --all`, and checks or applies source updates
  ([README](https://github.com/reorx/skm/blob/cbbe4fb0215ea6f0556ae2e1720298d697caa2e4/README.md)).
- Swival's first-party library design separates downloaded-but-inactive
  collections from project and global active roots, records source/ref, refuses
  ambiguous bare names, and permits explicit replacement with `--force`. It is
  an Agent-specific staging and activation reference, not a general
  cross-harness governance engine
  ([library documentation](https://github.com/Swival/swival/blob/554583ad716800dfad15d247175f3b33615fc223/docs.md/library.md),
  [Skills documentation](https://github.com/Swival/swival/blob/554583ad716800dfad15d247175f3b33615fc223/docs.md/skills.md)).
- mode-io Skill Manager adopts local folders into one shared store, uses
  symlinks on macOS/Linux and junctions on Windows, and enables or disables
  bindings without deleting the package. Its broader scope includes MCP and
  slash commands; its optional LLM scan explicitly sends selected Skill context
  to a configured provider. SkillRoster should not copy that cloud-capable scan
  path or broaden into an extension marketplace
  ([README](https://github.com/mode-io/skill-manager/blob/6ca969cbbc2e6b9a0de719858a68b33b3c37844a/README.md)).

Absence statements in the table are deliberately narrow: they mean the pinned
public command surface and documentation do not describe the capability. They
do not prove that no downstream integration or future release can provide it.

## Product boundary for SkillRoster

SkillRoster should:

1. Treat unmanaged copies, hand-written links, Vercel `skills`, `skm`, and other
   manager layouts as observable inputs. Never silently seize ownership.
2. Preserve the distinction between a logical Skill and its physical
   placements. A manager's shared store is provenance/layout evidence, not
   proof that exposure is appropriate.
3. Recommend a four-level per-Agent roster. On-demand means searchable through
   local `find`, not linked into every default discovery root.
4. Make every write an immutable, fingerprint-bound Plan followed by Apply,
   Receipt, verification, and bounded Undo. Do not add `--force` or `--yes`.
5. Parse supported local session sources read-only and store only derived
   evidence. Never upload Skill contents, sessions, usage, configuration, or
   governance state.
6. Avoid package registry, marketplace, generic plugin SDK, daemon, MCP, or
   built-in model calls. Existing managers remain better acquisition tools.

Ponytail is useful only as a development-complexity fixture. At revision
`2ed6c52c9d7e5e56942508591085fd45dea277d3`, its `skills/` directory contains
six logical Skills while the repository also carries host-specific plugin,
hook, command, rule, and mirrored OpenClaw packaging. Its own installation
instructions vary by host, and it documents Vercel `skills` and Swival options
([README](https://github.com/DietrichGebert/ponytail/blob/2ed6c52c9d7e5e56942508591085fd45dea277d3/README.md)).
That makes it a good cross-Agent discovery and duplicate-packaging test, but it
must never become a SkillRoster dependency, preferred source, or special case.

## Reproducible local comparison

The comparison must use synthetic data only. Do not point any manager at the
operator's real home, Agent directories, sessions, credentials, or private
repositories.

### Fixture

Create an isolated temporary home with eight supported Agent roots and a local
Git repository containing 12 harmless fixture Skills:

- six unique Skills, including three with distinct routing keywords;
- two byte-identical copies under different names;
- one same-name divergent pair;
- one valid shared canonical Skill linked into four Agent roots;
- one broken link and one link escaping the approved fixture root;
- Core candidates, rarely used candidates, and three synthetic session files
  whose expected Matched/Loaded/Applied stages are declared in fixture data.

Pin every tested tool to a commit or released version. Disable network after
local fixture preparation; disable manager telemetry where supported. Set
`HOME`, XDG paths, and tool-specific roots to the temporary directory. Record
an initial manifest containing relative paths, file types, link targets, modes,
and SHA-256 digests.

### Procedure

Run each arm from a fresh copy of the same fixture:

1. **Unmanaged:** inspect and improve the layout using only shell/file tools.
2. **Manual:** create a canonical directory, curate per-Agent links, document
   source/revision and restore steps by hand.
3. **Manager:** run Vercel `skills`, `skm`, or another relevant manager using
   only its documented local-source path; capture commands, prompts, outputs,
   lock/config files, and final manifest.
4. **SkillRoster:** run `scan --json`, `report --json`, three `find --json`
   tasks, submit a complete roster Plan, Apply once, verify, and Undo.

For every arm, answer the same questions without reading another arm's output:
inventory count, placement count, exact duplicates, broken/escaping links,
source/update state, default exposure per Agent, Top-3 routing result, changed
paths, restoration result, and remaining uncertainty. Repeat each automated arm
three times. A human reviewer checks whether every claim resolves to a path,
digest, manager record, or declared fixture expectation.

### Metrics and result table template

`Detection recall` uses declared fixture truth. `Default exposure reduction`
compares placements visible at Agent startup before and after the reviewed
proposal. `Restoration` is byte/link/mode equality with the initial manifest.
Time is secondary and includes operator steps; unsupported capabilities are
reported as `N/A`, never zero.

| Arm / version | Inventory recall | Layout finding recall | Provenance known | Default exposure before -> proposed | Routing Top-3 | Unapproved writes | Apply verification | Undo/restoration | Operator steps | Wall time | Network after setup | Evidence/notes |
|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---|---|
| Unmanaged | TBD | TBD | TBD | TBD | N/A | TBD | N/A | TBD | TBD | TBD | No | |
| Manual | TBD | TBD | TBD | TBD | manual | TBD | manual | TBD | TBD | TBD | No | |
| Vercel `skills` @ version | TBD | TBD | TBD | TBD | TBD | TBD | manager-specific | TBD | TBD | TBD | record | |
| `skm` @ version | TBD | TBD | TBD | TBD | N/A | TBD | manager-specific | TBD | TBD | TBD | record | |
| Relevant manager @ version | TBD | TBD | TBD | TBD | TBD | TBD | manager-specific | TBD | TBD | TBD | record | |
| SkillRoster @ commit | TBD | TBD | TBD | TBD | TBD | 0 expected | receipt | 100% expected | TBD | TBD | No | |

Publish the completed table only with fixture repository revision, exact
commands, raw machine-readable outputs, manifests, and reviewer identity. Do not
generalize one synthetic run into claims about all user installations.
