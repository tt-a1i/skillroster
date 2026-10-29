# SkillRoster Product Specification

## 1. Product promise

SkillRoster lets a person ask an AI Agent to inspect and govern local Agent Skills. The Agent invokes one bootstrap Skill, calls a deterministic Rust CLI, explains evidence-backed problems, and executes only a user-approved, reversible Plan.

> One library. The right roster for every agent.

The first complete release must answer four questions reliably:

1. What Skills exist across the supported Agents, and where did they come from?
2. Which Skills are exposed, duplicated, stale, broken, risky, or unsupported by usage evidence?
3. Which Skills should be Core, On-demand, Explicit-only, or Archived for each Agent?
4. How can the approved organization be applied and undone without losing user-owned files?

## 2. Supported environment

The first complete release directly supports:

- Codex
- Claude Code
- Pi
- OpenCode
- Hermes
- Cursor
- Gemini CLI
- GitHub Copilot

Supported platforms are macOS, Linux, and Windows. WSL2 is treated as Linux;
WSL1 mutation fails closed because it does not provide the no-replace rename
primitive required by SkillRoster's handle-bound recovery model. A platform or
Agent is not declared supported until its adapter passes fixtures and a
real-environment acceptance run.

SkillRoster scans only known directories for these eight Agents, the active SkillRoster-owned Library root, Skill roots from Codex plugins proven by either explicit local enablement or a valid local remote-plugin install marker, plus paths explicitly provided by the user. It never crawls the entire home directory or treats an arbitrary plugin cache as installed. Explicitly disabled plugins remain excluded. Discovered Codex plugin Skills are Observed, searchable, and provider-managed read-only: they cannot become mutation targets or canonical Library sources. Invalid install markers and ambiguous cached plugin versions fail closed. Every new Snapshot records two orthogonal placement facts: `owned_by_agent` describes only whether the placement path is structurally inside an Agent root, while `mutation_scope` is `mutable`, `provider_read_only`, `shared_read_only`, `durable_read_only`, or `untrusted_external`. Placement-path ownership never endorses linked source content. Compatibility field `governable` is true exactly for `mutable`; missing authority fields in legacy Snapshots remain unknown and cannot authorize mutation. Every Scan reports included, excluded, missing, and inaccessible roots. `--root AGENT=PATH` is an Agent placement root and contributes to exposure; `--source-root PATH` is an approved canonical source with no Agent identity and no default exposure. Source trust is explicit and scoped to the resolved canonical directory; a path and a symlink alias to that same directory have identical trust semantics.

### Local catalog visibility

Codex discovery retains the legacy user `.codex/skills` root and includes
`.agents/skills`. The latter is a shared root, not evidence of a Codex-only
installation. Its readable placements and physical aliases have
`mutation_scope: shared_read_only`; escaping or unreadable content retains its
trust blocker. Explicit root/source aliases cannot restore mutation authority.
Shared roots are not Bootstrap or implicit Core write destinations, and an
eligible legacy or explicit mutable destination retains its previous order.

For Codex user Skills, `codex_visibility` describes the supplied Home's
`.codex/config.toml` Skill controls. It is a local rule projection, not a running
Agent's catalog: `skill_configuration_known` covers only the inspected Skill
rules, `host_config_validity` is `unverified`, and `session_catalog_observed` is
false. Other Codex configuration types, CLI overrides, managed settings,
profiles, and project runtime layers are not validated or inferred. `on` means
these observed local rules do not disable the discovered placement; it does
not prove that the complete host configuration can load or a task can use it.
Arbitrary non-user Codex roots retain unknown native policy.

Typed TOML rules are evaluated in order with the last matching rule winning.
Exactly one of `name` or `path` must be present for a rule to participate;
both/neither are inert but invalid field types remain unknown. Names match
verified frontmatter names case-sensitively after trimming surrounding
whitespace. Each name is bound to its verified placement bytes, not the
representative metadata of a deduplicated Skill. Unsupported name syntax
remains unknown when it can affect the rule result. Paths identify `SKILL.md`,
not its directory, resolve relative to
the configuration directory (or supplied Home for `~/`), and match canonical
entrypoint identities. A missing-path disabling declaration also protects that
future Core target. Unknown Skill names cannot silently bypass name rules.
Configuration paths are resolved for metadata identity only; they never
authorize content reads, add approved roots, or grant mutation authority.

Disabled user Skills remain inventory and may be explicitly loaded through
verified Find; their local default exposure is false. Unknown policy retains
possible exposure and blocks changes that depend on resolving it. Core Plans
must preserve native restrictions, including the destination and canonical
source of a new link. Implicit canonical migration and retained-link retargeting
must preserve affected Codex visibility even when only another Agent was named
in the request. Library consolidation and source updates must also preserve
affected Codex visibility. Uncertain projected path-policy changes fail closed.
Scan, Report, and Plan summaries disclose the local-rule scope and unverified
host configuration; their exposure counts do not promise a usable session catalog.

Snapshots freeze configuration digests or absence, configured path identities,
and Skill-root identities. Report, Find, Plan, Apply, and Setup (including
`--check`) require a rescan after those inputs change. Every older Snapshot
requires this new contract even if it contained only Agent-less sources; it
cannot reuse pre-shared-root mutation authority. Neither scanning nor governance
edits Codex configuration. The isolated native evidence and scope are in
[the Codex mechanism record](research/codex-native-visibility.md).

For Claude Code, `default_exposed` combines directory discovery with observed
local frontmatter and `skillOverrides`. Manual-only and off entries remain in
inventory without being counted as listed to the model. `name-only` still counts
as one listed placement; these counts do not estimate tokens. Scan and every
Report list view expose `native_visibility`, including its local-file basis and
unknown count. Unknown entries remain possible exposure and cannot authorize an
affected Roster change. A Core Plan cannot override native visibility controls.
Legacy Snapshots and changed settings require a new Scan before Report, Find,
Plan, or Apply reuses exposure facts. Settings contents are never mutated.
See [the inspected scope and acceptance](practical-governance.md); this local
projection does not observe an active session's catalog or permissions.

For Hermes, `hermes_visibility` projects the supplied Home's
`.hermes/config.yaml` and verified entrypoint bytes. It covers ordinary global
disabled names and supported desktop OS restrictions. `on` means these local
rules allow the placement, not that a running Hermes session offers it;
`host_config_validity` remains `unverified` and `session_catalog_observed` is
false. Off placements stay in inventory and remain available for explicit
verified Find, but do not count as default exposure. Unknown placements retain
possible exposure and cannot authorize affected governance.

The projection does not infer a gateway channel from SkillRoster's process.
Potential channel-specific restrictions, directory aliases, version-dependent
selectors or essential exceptions, organization mirrors, additional-root
precedence, unsupported configuration, and unobserved runtime conditions remain
unknown. Each placement uses its own verified bytes rather than a representative
name selected during identity deduplication. Configuration is read within a
fixed bound; YAML nesting, node count and expanded scalar bytes are also bounded.
Only needed typed controls, digests and path identities are retained, not the
complete configuration or unrelated values.

Snapshots freeze the Hermes local rules, host OS and root identities. Report,
Find, Plan, Apply and Setup/check require a rescan after those inputs change.
Old observations cannot authorize Bootstrap creation from a sessions-only
installation either. Existing immutable Plan details remain inspectable and
offer Scan recovery when their frozen contract is outdated. Core creation,
source updates, Library consolidation, implicit retargeting and Bootstrap
entrypoint replacement must preserve known native restrictions, including
consumers belonging to an Agent absent from the original request. A parent
package move must also preserve the paths of nested Hermes consumers with a
different Skill identity; metadata equivalence alone cannot prove that their
links survive. If no verified replacement preserves those paths, both Plan
preparation and Apply refuse the move. No such operation edits the Hermes
configuration. See the
[pinned mechanism comparison](research/hermes-native-visibility.md) for the
version and surface differences behind this scope.

## 3. Primary interaction

```text
Person
  -> supported Agent
      -> skillroster bootstrap Skill
          -> skillroster CLI --json
              -> local filesystem, sessions, and SQLite
          <- facts, Findings, Evidence, Plans, Receipts
      <- concise diagnosis and confirmation request
```

### 3.1 Agent-first responsibility boundary

The person delegates intent in natural language and should not need to learn or compose a CLI workflow. The calling Agent owns intent interpretation, semantic comparison, prioritization, recommendation, and natural-language presentation. It translates the person's request into structured SkillRoster calls, asks only for genuine preference or authorization decisions, and explains the resulting Evidence and Plan.

The CLI owns deterministic discovery, normalization, hashing, indexing, statistics, validation, mutation, and recovery. It returns bounded, decision-complete facts and typed options so the Agent does not need to rediscover filesystem state or scrape prose. Suggested action argv preserves the exact running executable, effective local state, and discovery overrides, keeping follow-up commands on the same binary, Snapshot, and explicit source-trust boundary. It validates the Agent's structured choices but does not replace model reasoning with embedded intent classifiers, semantic policy trees, or a second prompt framework. Human-readable terminal output supports audit, diagnosis, and fallback use; versioned JSON is the primary Agent integration contract.

Use this placement test for every capability: local truth, stable identity, reproducibility, safety, and reversible execution belong in the CLI; context-sensitive meaning and open-ended judgment belong in the Agent. The person retains preference and material-change authority. The binary contains no model API or second Agent.

## 4. Capability requirements

### 4.1 Inventory and identity

A read-only Scan discovers Skill roots, `SKILL.md` entry points, source metadata, links, configuration exposure, and supported local session sources. It normalizes paths without following links outside approved roots silently. Entrypoint discovery traverses root and category directories within its depth bound. Unless an adapter has a verified different discovery rule, a directory declaring `SKILL.md` ends arbitrary descendant discovery; only direct child directories that also declare `SKILL.md` continue the nested-Skill chain. Repository trees `.git` and `node_modules` are excluded from entrypoint discovery. Build directories named `target` are also excluded except for Hermes, whose verified directory loader does not exclude that name. These exclusions do not weaken package fingerprints: support files remain identity- and drift-bearing under the independent fingerprint bounds.

Hermes directory discovery follows the verified local loader's recursive rule:
non-support descendants of a Skill package remain discoverable; the package's
direct `references`, `templates`, `assets` and `scripts` directories remain
non-exposed support areas. A same-named category outside a package remains
discoverable. Approved directory links may contribute nested placements, within
the existing depth limit and a per-branch physical-directory cycle guard.
Unapproved, unreadable, cyclic or depth-bounded category discovery is incomplete,
not an empty complete inventory. Directory identity changes during enumeration
invalidate the affected candidates.

A placement beneath a directory-link ancestor is shared read-only: changing
that nested logical path would change the shared target. Other aliases of the
same physical package inherit that protection. A direct link to a Skill package
retains its existing placement contract. Read permission does not authorize
mutation of a linked category's contents.

When the Hermes Skill root itself is a directory link, the root and its
placements are shared read-only; Bootstrap Setup and implicit Core creation do
not choose it as a write destination. A normal ancestor path alias alone does
not establish a shared root. Bootstrap operations, including directory creation,
must pass the same read-only placement overlap validation during preparation
and Apply as other mutations. Confirmed regular-file links do not contribute
directory discovery branches.

Fresh Snapshots record the Hermes discovery contract version. A legacy or
incompatible Snapshot with an included Hermes Skill root requires one Scan before
Report, Find, new planning, Apply or Setup, including its check mode. Existing
immutable Plan details remain inspectable; a legacy Ready Plan offers a Scan
continuation and cannot Apply from the old observations.

The directory projection and the local visibility rules above do not constitute
an effective session catalog. Profile and extra-root discovery, channel and
environment filters, organization gates, and name precedence require separate
native observations before they can support a full visibility claim. The pinned
directory source and local-loader evidence are recorded in
[`research/hermes-skill-discovery.md`](research/hermes-skill-discovery.md).

The immutable Scan payload is the historical Snapshot record. SQLite's
normalized placement table is the current projection: when a stable placement
is observed again, the same placement ID moves to the latest Snapshot and every
current Skill, Agent, root, path, kind, link, fingerprint, and exposure fact is
refreshed. The latest payload placement IDs and normalized latest-Snapshot
placement IDs must be identical; historical graphs remain available in their
immutable payloads rather than stale projection rows.

Skill-root discovery and package fingerprints carry explicit completeness facts. A configured root whose depth limit was reached remains Included but is marked discovery-incomplete, making structural coverage unreliable. Each placement fingerprint is `complete`, `bounded`, `unreadable`, or `unknown`; payloads created before this contract default to `unknown`. Only a complete package fingerprint may support an exact-duplicate Finding or any Library/Roster Plan that relies on exact content. Bounded or unreadable packages remain inventory facts and require a new complete Scan before governance. Apply repeats this check so a Ready Plan created by an older binary cannot bypass the boundary.

The current Snapshot schema accepts only Unicode identity-bearing paths. A
non-Unicode configured root fails the Scan with `non_unicode_identity_path`;
discovered non-Unicode Skill directories are skipped and make that root's
structural coverage incomplete. A Unicode Skill containing a non-Unicode
package member remains an inventory fact with an `unreadable` fingerprint and
cannot authorize exact load or governance. SkillRoster never hashes lossy path
text or persists a replacement-character identity. Snapshots record typed
`identity_path_coverage` plus an exact skipped-path count; any unverified or
incomplete value blocks Find, Report, Plan, Apply, and other exact-identity
decisions until a complete rescan. The `sha256-content-unicode-v2` presence
marker prevents pre-contract Snapshots from being reused even when every
currently visible path is Unicode. A future raw-byte or UTF-16 path encoding
requires an explicit versioned schema migration.

Scan records two deliberately separate hashes in one bounded traversal. The complete package fingerprint covers every retained package file, including `.gitignore`, and remains the only hash used for drift checks, exact load, Plan, Apply, Receipt, Undo, and recovery. The versioned routing content identity excludes only the package-root `.gitignore`; it is used for unsourced logical identity and same-name variant grouping because source-control metadata is not Agent Skills payload. `SKILL.md`, scripts, references, assets, and symlink targets remain identity-bearing. A legacy Snapshot without the current content-identity algorithm must be rescanned; the CLI never backfills or infers equality from old package fingerprints.

One routing identity may therefore have multiple complete package fingerprints. Existing Core placements may remain unchanged, but any Roster mutation, migration, retarget, or new exposure that would require choosing one package as canonical fails closed with the affected placement IDs and fingerprint count. Routing equivalence never authorizes package consolidation or metadata loss.

Skill identity uses this precedence:

1. declared source plus version or revision;
2. versioned routing content identity;
3. name and semantic similarity as candidate evidence only.

Ambiguous Skills remain separate. SkillRoster never merges based only on a matching name or Agent interpretation.

A declared source/version identity may have divergent local content. The
Snapshot keeps every distinct routing-content observation with its own name,
metadata and searchable text, and binds each Placement to that routing digest.
These observations share the logical Skill ID used by Roster and usage records;
one observation's metadata is not evidence for another observation's content.
Older Snapshots missing these observations require one new Scan.

### 4.2 Analysis

`report` covers seven categories:

- **Inventory:** counts, locations, ownership, sources, and versions.
- **Layout:** copies, links, broken references, escaping links, and collisions.
- **Exposure:** which Agent and scope can see each Skill.
- **Usage:** evidence windows and the Exposed, Matched, Loaded, Applied, and Outcome stages.
- **Overlap:** exact duplicates and evidence for likely semantic overlap.
- **Routing:** default exposure, searchability, and missing task-to-Skill routes.
- **Lifecycle:** update drift, stale sources, management state, and archive candidates.

Usage evidence is labeled `observed`, `inferred`, or `unknown`. Missing evidence never becomes proof that a Skill is useless. Raw prompts and responses are parsed read-only in place and are not copied into SkillRoster storage. Recent session selection happens after bounded discovery; large active JSONL files contribute bounded complete-line tails, large monolithic JSON files contribute bounded complete nested objects from their tails, and the fixed byte budget is spread across multiple recent sessions. Reports distinguish missing, inaccessible, sampled-but-limited, and complete observable session roots. A usage percentage is emitted only when the observable-session denominator is complete; bounded samples support observed event counts and stages, not an “unused” claim.

New session Scans record every completeness boundary at its trigger point as typed, multi-value Coverage limitations. Each fact freezes its code, root/file/Agent scope, count quality, observed value, active limit, unit, and scanner source; aggregate coverage fields remain compatibility projections. Repeated occurrences aggregate by code within one Agent without persisting session paths, and count quality states whether the reported observation is exact, a lower bound, or unknown. Public Coverage Evidence classifies which unchanged boundaries will recur, which local read failures may resolve, and which roots require verification before a rescan. Scanner caps are not configurable through the CLI. Legacy Snapshots without typed limitations remain `legacy_unknown` and never inherit the current binary's limits or claim a reliable denominator.

Session parsing validates the original UTF-8 bytes and JSON syntax before any
structured observation or name-based inference. Malformed JSONL/structured log
records contribute no usage; valid neighboring records remain observable. A
fully read `.json` file must contain a valid complete document; empty and
whitespace-only documents are parsing failures. Bounded JSON tails may still contribute valid
complete objects under the existing sampling limits. Blank JSONL/log lines are
ignored; valid records without recognized Skill events, including unknown event
shapes, are not parsing failures. Syntax validation does not prove that every
Agent event kind is understood.

Observed parsing failures produce `file_parse_failure`, counted once per
affected file, and invalidate the usage denominator. Raw malformed content and
parser error excerpts are not persisted. Coverage freezes
`parse_contract_version`; a missing or unsupported version remains
`legacy_unknown` even if an older Snapshot recorded empty limitations and a
reliable denominator. Reports, cached summaries, historical Finding details and
exported Coverage Evidence preserve this boundary without rewriting historical
Snapshots. Historical usage summaries are recomputed conservatively; an old
archive candidate whose relevant reliable coverage is no longer established
becomes read-only and requests a rescan. A new Scan can establish the current contract. Historical usage
observations remain historical; new Reports use their own Snapshot's usage.

Finding coverage never mixes filesystem discovery with session observation.
Inventory, layout, exposure, overlap, routing, and structural lifecycle
Findings use `skill_root_scan`; known-missing Skill roots are observed absence,
while inaccessible roots make that structural denominator incomplete. Usage
and usage-dependent archive Findings use `session_usage` and separately name
reliable, sampled-limited, missing, excluded, and inaccessible Agents; those
states are mutually exclusive for every supported Agent.

A semantic-overlap Finding is candidate evidence, never a consolidation
decision. Its compact and full detail carry the same bounded comparison bundle:
the two stable Skill identities, names, descriptions, routing triggers,
summaries, readable placement paths and governance facts, plus the structured
routing-vocabulary Jaccard score, counts, and shared-term preview. Complete
Skill bodies stay at the returned local paths. The calling Agent or person owns
the semantic conclusion, and no Plan action is suggested from this evidence.

Reports disclose semantic overlap candidate bounds as `semantic_overlap_candidates`: the total qualifying pair count, the retained count (at most 25), and whether candidates were truncated. These are analysis-wide bounds, independent of Finding filters or pagination, and appear in both JSON views and terminal summaries and Finding lists. Cached reports without these bounds are regenerated from the same Snapshot; a new Scan is not required.

### 4.3 Library and Rosters

The Library is logical and does not require moving every Skill. Each discovered Skill begins as Observed. After approval it may become:

- **Managed:** SkillRoster manages exposure, links, or configuration while the canonical files remain in place.
- **Hosted:** canonical files are explicitly migrated into the active state directory's `library/` root (by default `~/.skillroster/library`).

Each Agent Roster classifies Skills as Core, On-demand, Explicit-only, or Archived. On-demand Skills remain searchable without being linked into every default Skill directory. For large-Roster Core selection, protected, declared, and bootstrap Skills come first; target-Agent usage outranks usage from another local Agent for the exact same stable Skill identity, which outranks stable fallback order. Cross-Agent evidence never transfers by name, path, or fuzzy similarity and always identifies its source Agent. `find` returns paths and Evidence so the Agent can read the selected `SKILL.md` directly; temporary activation links are unnecessary.
When multiple Agent placements share one physical Skill directory, Recommendation reconciles their final exposure before returning changes. A shared group with only ranking-derived disagreement becomes On-demand for every in-scope Agent; Core is not a quota. Protected, declared-Core, bootstrap, or out-of-scope retained exposure instead keeps the group Core. Per-Agent counts and previews describe this reconciled result, and typed `roster_shared_core_budget_exceeded` blocks forced propagation beyond the requested budget. The Plan layer independently rechecks physical compatibility and remains fail closed for arbitrary direct requests.
The Scan payload freezes each Placement's observed physical mutation identity so Recommendation is deterministic even if the filesystem later drifts. A legacy Snapshot without that fact cannot authorize a recommendation: Report and Plan return typed `roster_physical_identity_rescan_required` / `snapshot_missing_physical_mutation_identity` with one read-only Scan continuation. Plan still resolves the current identity independently and rejects post-Scan drift.

### 4.4 Planning, mutation, and recovery

Read-only commands never mutate Agent configuration or Skill contents. Agent recommendations enter the CLI through `plan --stdin` as structured target states. Most requests cite current Scan, Skill, and Evidence IDs directly. Exact-duplicate consolidation instead cites the immutable Finding plus the Agent's two semantic choices; the CLI derives the bound Snapshot, Evidence, Skill, and complete placement set. Large-Roster governance cites its immutable Finding, a per-Agent Core budget, and optional protected Skills. The CLI validates the complete oversized scope and derives every Core or On-demand Roster change; it never infers Explicit-only or Archived from missing usage evidence. A smaller custom Core budget that introduces preservation blockers fails closed: `plan --stdin` returns bounded typed blocker evidence with the affected Agent, stable Skill identity and name, and the narrowest `--source-root` directories already observed as escaping-link targets. It sorts and deduplicates exact targets and removes a descendant only when its ancestor was itself observed; sibling targets never synthesize a broader parent. Ordinary JSON lists at most ten blocked Skills and ten source roots; the human terminal lists at most five of each. When the complete set is larger, `error.details.detail.path` is a SkillRoster-owned JSON file with every omitted identity, directory, and the complete repeatable `--source-root` argv. It does not read an untrusted target, imply trust, or create a partial Plan.
When large-Roster planning is blocked with `trusted_canonical_sources_required`, its detail exposes a typed opaque `source_confirmation_finding` reference only when one unique escaping-link Finding covers all relevant untrusted-external blocker Skill IDs in that same Report/Snapshot. A read-only `view_source_confirmation_finding` continuation may open that Finding; only its existing exact source-root confirmation actions can grant local read permission. Missing, stale, or ambiguous references remain unavailable and emit no prerequisite action.
When provider, durable-read-only, or multi-fingerprint placements make a Roster demotion unsafe, the Finding groups the blocked changes by stable Skill identity and offers an explicit Core-protection choice. Compact detail never exposes a Plan template from a truncated protected-Skill set. Full detail validates the complete protected set through the production Recommendation path; only a valid set produces an exact `finding_roster_changes` template. Replaying that template keeps the blocked Skills Core and derives changes only for mutable placements. It never promotes read authority into mutation authority or treats a read permission as content endorsement.

When all blocked changes require preserving different package fingerprints,
the Finding reports `package_fingerprint_variants_block_roster_change` instead
of a mutation-scope refusal. `blocked_reasons` lists the distinct reasons from
the complete blocked set even when its per-change or per-Skill preview is
truncated. Mixed blockers retain the applicable source or scope continuation;
read confirmation cannot resolve independent package-preservation blockers.

Before a large-Roster Finding advertises a Ready-Plan template, SkillRoster derives the proposed Plan without persisting it. If multiple Skill identities would claim one name-derived Library target, planning fails closed with every claimant's exact Skill ID and original name. Library target identity uses a conservative ASCII case-fold so the same Plan is safe on supported case-sensitive and case-insensitive filesystems. A true same-name group links the matching same-Snapshot divergent-content Finding when available; distinct names that normalize to one safe directory use a separate typed reason and never claim that a same-name Finding exists. It never chooses a canonical variant by rank, path, or recency. Full Finding detail may instead offer one exact request that keeps every conflicting identity Core, but only after that request passes the production Recommendation, placement-safety, and Plan derivation paths without a duplicate physical target. Validation recursively closes over later target-claim groups and discloses each target, original name, and Skill ID before confirmation. Compact claimant and protected-ID lists are bounded and cannot expose a template from an incomplete closure. The generic physical-operation conflict check remains the final guard.

The CLI validates targets, ownership, conflicts, and current fingerprints, then stores an immutable Plan. Exact-duplicate Library Plans report comparable before/after physical-source, placement, and default-exposure counts plus explicit deltas, so source consolidation cannot be mistaken for Roster reduction. `apply <plan-id>` executes only the approved scope. Cross-filesystem operations use a journal and compensating steps; the CLI never claims atomicity it cannot provide.

Before persisting any Ready Library Plan, and again before Apply enters Applying,
the CLI verifies each affected placement's current complete package identity and
digest against its exact Snapshot observation. This includes entrypoint and
support files and covers explicit and derived Library changes. New operation
fingerprints cannot substitute for that evidence. Changed, unavailable, or
incomplete package evidence fails closed with `library_content_rescan_required`
and a context-preserving read-only Scan continuation; retry requires a fresh
observation. Existing Plans and Receipts remain immutable.

Logical placements reached through symlinked Agent roots may resolve to one
physical filesystem object. A Plan operates on that object at most once while
retaining every logical Agent/placement impact fact. If the linked logical
placements request incompatible Core and non-Core exposure, planning fails
closed with a typed conflict. No Ready Plan may contain duplicate destructive
sources or operation targets. Before deriving operations, planning revalidates
each captured physical source against the current logical entrypoint and stores
those logical-entrypoint-to-physical-source bindings in the immutable Plan.
Apply revalidates the complete binding set before entering Applying. A
physical object shared with any non-mutable placement remains read-only,
and a non-Agent source link that would be broken blocks the Plan. These
blockers expose stable reasons, IDs, paths, and next actions in `error.details`;
Agent callers never need to parse the human message.

Apply fails closed when paths, fingerprints, links, or configuration have drifted. Every successful mutation writes a Receipt. `undo <receipt-id>` is bounded to that Receipt and refuses ambiguous restoration. Canonical deletion is outside normal Apply, requires separate confirmation, and should prefer recoverable archive.

A verified Applied Receipt invalidates the completed Snapshot on which its Plan
was based because the mutation changed inventory facts after that observation.
Until a newer Scan completes, commands that require current inventory facts
fail with typed `snapshot_rescan_required` details and one read-only Scan
continuation. Apply returns `rescan_required: true` and keeps both the required
Scan and exact Receipt Undo actions visible. Status, lifecycle recovery, and
Undo remain available while facts are stale. If no newer Scan completed after
Apply, verified exact Undo consumes the Applied Receipt and restores the
pre-Apply state, so that original Snapshot is current again without a redundant
Scan. If a newer Scan observed the applied state, Undo invalidates that newer
Snapshot and returns its own required Scan continuation. Failed-and-compensated
Apply never invalidates a Snapshot; recovery-required state keeps its existing
recovery boundary.

When Home or Status resumes an ordinary invalidated-Snapshot state, the
read-only Scan remains the primary `next_action`. An original, still-Applied
invalidating Receipt also supplies its exact Undo as a secondary suggested
action; an Undo Receipt cannot itself be undone and keeps only the Scan.
Recovery-required state suppresses both behind lifecycle recovery inspection.

File replacement copies the original through an exclusively created staging
handle and restores its platform permissions before publication. Unix
permission bits and Windows file attributes, including readonly, must therefore
survive both Apply and Undo, including an originally non-writable file. Windows
removal uses a handle-bound disposition that ignores readonly without first
weakening the original path's attributes. Copy-based mutations additionally
validate observable metadata through retained handles: Unix owner/group and
permission bits must survive; unsupported extended attributes and ACLs refuse
the copy. macOS system provenance is accepted only when its exact value survives
unchanged, while extended ACLs and other file flags are unsupported. Recursive
copies finalize directory permissions after copying children. Windows ordinary
file attributes, owner, group, and DACL must match at the destination; a copy
across different inherited ACLs is refused instead of broadening access.
Windows named streams, extended attributes, and non-ordinary file attributes
are unsupported. Private replacement backups keep their private DACL; the
original Windows owner/group/DACL evidence is saved in the private Receipt and
checked against the restoration destination. Legacy Windows replacement
Receipts without that evidence fail closed rather than guessing lost metadata.
Replacement Undo refuses observable metadata drift from its recovery evidence.
This is not a portable archive/backup contract for timestamps, hard-link
topology, privileged audit policy (such as Windows SACLs), or metadata the OS
does not expose to this process. See [hardening evidence](hardening-evidence.md)
for precise failure-test and metadata boundaries.

Apply, compensation, and Undo open approved roots and the private state root as
directory capabilities before mutation. Filesystem paths are resolved relative
to those retained handles for fingerprinting, copy, rename, replacement, and
link operations. If an opened root and its directory entry no longer identify
the same directory, or if an ancestor symlink would escape the handle, the
operation fails closed without touching the escaped target.

### 4.5 Source updates

SkillRoster records source, revision, and content hash when available. A source-update request submitted through `plan --stdin` first produces a diff and Plan; there is no silent update path. Local modifications stop automatic replacement and require an explicit choice to retain local content, adopt upstream content, or preserve both.

In newly prepared source-update Plans, `local_modification_detected` is `null`
when no trusted source baseline exists; the first observed bytes do not establish
whether a local modification occurred. With a trusted baseline, it is `true`
when the current entrypoint differs and `false` when it matches. An unknown
result still requires an explicit choice. Previously saved Plan summaries remain
immutable.

## 5. CLI contract

The first complete command surface is:

```bash
skillroster [--json]
skillroster scan [--summary] [--json]
skillroster report [--summary | --full | --findings [--category <category>] [--severity <severity>] | --finding <id> [--full]] [--limit <n>] [--offset <n>] [--json]
skillroster find [--hint <text>]... [--limit <n>] [--load] [--variant-skill-id <skill-id>] [--variant-placement-id <placement-id>] [--require-snapshot <scan-id>] [--json] -- <task>
skillroster plan --stdin [--json]
skillroster plan --show <plan-id> [--json]
skillroster apply <plan-id> [--json]
skillroster undo <receipt-id> [--json]
skillroster status [--json]
skillroster source-root confirm --finding <finding-id> --path <absolute-path> [--json]
skillroster source-root inspect [--json]
skillroster source-root revoke <permission-id> [--json]
skillroster setup [--modified-choice retain-local|adopt-current] [--json]
```

The no-subcommand Home is the bounded readiness projection for Agents. It does
not maintain an independent readiness heuristic. Home and `status` use one
continuation selector with this priority: recovery inspection, missing-Snapshot
Scan, invalidated-Snapshot Scan, actionable Ready Plan inspection, a missing
Report for the current Snapshot, then no required continuation. Home exposes
`recovery_required`, `no_snapshot`, `rescan_required`, `plan_ready`,
`report_required`, or `ready`; its JSON action and human argv are the same
generated continuation with the original discovery context preserved. A
current Snapshot proves inventory freshness, not completed analysis: Home and
`status` remain resumable through `report --json` until that Snapshot has a
persisted Report.

Finding lists and full Finding records default to 20 rows per page. Compact
`report --finding <id>` detail defaults to five rows so an Agent receives the
decision and action chain without spending context on a large first page.
Explicit `--limit` values always win on paged Finding lists and detail;
`page` totals and continuation actions remain the authoritative path to
omitted rows. Top-level `report --full` remains an unpaged exhaustive export.

`find --require-snapshot <scan-id>` is an optimistic read boundary used by
typed continuation actions. Find fails closed with `find_snapshot_changed`
when another Scan has become latest; it never silently resolves an ambiguity
or loads exact variant content against a different Snapshot. The returned
read-only retry replaces the stale requirement with the latest observed
Snapshot requirement and preserves the task, hints, limit, and discovery
context so the Agent can restart from those facts without reopening the race.

`source-root confirm` persists one exact local read permission bound to a
current completed escaping-link Finding, its observed canonical directory, and
the directory's stable filesystem identity. Scan freezes active permissions
before discovery; missing, inaccessible, replaced, or retargeted roots fail
closed independently and remain typed audit facts. Confirming read access does
not endorse content, raise Evidence quality, authorize governance or Plan/Apply,
or change Agent/Skill files. `inspect` includes active, revoked, and drifted
records; `revoke` retains the approval and revocation times. No parent, sibling,
descendant, alias, wildcard, or Agent-specific permission is inferred.
Discovery and content consumption recheck the frozen directory identity and
exact entrypoint binding at bounded pre/post checkpoints, discard derived facts
when they observe drift, and preserve the drift in the Snapshot. This protects
against accidental or persistent local replacement and retargeting. It is not
an adversarial sandbox against a same-user process completing an ABA swap
entirely between checkpoints; descriptor/handle-bound traversal is tracked as
separate security hardening. The persisted object epoch is conservative:
platform metadata changes that could indicate object reuse may require an
explicit revoke and reconfirm rather than silently retaining read access.
Revoking a permission used by the latest Snapshot, or observing persistent
identity drift after that Scan, immediately makes the Snapshot
`rescan_required`. Current-Snapshot commands fail closed with the affected
permission IDs and a typed Scan continuation; an old `find --load` may not use
the Snapshot as continuing read authority.

The escaping-link Finding keeps legacy resolution fields for schema
compatibility and adds the canonical `decision_code` plus two explicitly
exclusive `permission_paths`. The durable path runs `source-root confirm` and
continues with a plain Scan; the temporary path skips confirmation persistence
and uses repeatable exact `--source-root` overrides for one Scan. The resolution
reports the complete unique observed-target count, a bounded target list, and a
truncation flag independently of the current placement page. When that list is
truncated, `page_observed_link_targets` and the page's typed confirmation
actions make the remaining exact targets retrievable through ordinary Finding
pagination. The legacy `confirm_trusted_source_roots` value never means that
content trust was assessed.

`setup` detects supported Agents and returns a preview for installing or
upgrading the single bootstrap Skill. Exact official older copies are eligible
for automatic planning. Unknown content is treated as a local modification and
requires an explicit retain/adopt choice before any Plan is created. Links,
non-files, and unreadable targets are blocked. In JSON mode setup never prompts
or mutates; every write remains a normal Plan completed through Apply and
recoverable through Undo. Repeating an unchanged setup preview on the same
Snapshot reuses an equivalent Ready Plan instead of accumulating duplicate
pending Plans. Terminal Plans, newer Snapshots, and changed setup inputs are
never reused. A legacy Ready Plan whose stored summary lacks the current
bounded decision fields is not equivalent: it remains immutable and auditable,
while Setup creates one decision-complete replacement. Both can temporarily
appear as pending; the Agent presents only the newly returned Plan ID.

The affected scope applies the normal Skill identity rules to the projected
package after Apply: declared source plus version/revision takes precedence;
otherwise identity comes from content, excluding `.gitignore`. The content
projection includes preserved unmanaged files and the normalized bytes of
managed files that Setup will write. Content-derived identities remain distinct
when identity-relevant preserved content differs. Package-level before and
after fingerprints bind that projection to Apply, so drift in preserved files
fails closed instead of producing a different post-Apply identity. Package
binding reuses the same depth and byte bounds and excludes `.git`, `target`,
`node_modules`, and `.DS_Store`, matching Scan's package boundary. Placement
counts retain every logical Agent target even when filesystem operations share
one physical root.

Detection is Snapshot-bound and explicit. `existing_skill_root` means the
Agent's fixed Skill root already exists. `included_session_root` means the
current completed Snapshot included that Agent's fixed session root while its
Skill root was genuinely missing. In the latter case Setup may plan only the
fixed missing parent chain, Bootstrap package directories, and four managed
files for that same adapter. A completely empty Home does not establish Agent
presence and remains `no_supported_agent`. Planning is read-only; Apply still
requires confirmation and produces a Receipt whose Undo restores both files
and newly created directories.

`find` preserves the user's original task and accepts repeatable Agent-authored
retrieval hints. Hints let the semantic caller supply a cross-language or
capability paraphrase while the CLI remains a deterministic local lexical
index. Without hints, Find uses one lexical channel. With hints, it ranks the
original task and the task-plus-hints expansion independently, then applies
deterministic reciprocal-rank fusion over a bounded candidate pool. The JSON
`ranking_strategy`, `task_channel_rank`, and `augmented_channel_rank` facts make
that decision inspectable. When the post-fusion protection rule moves the
strongest original-task match, that exact match also carries
`ranking_adjustments: ["protected_original_task_match"]`; unadjusted matches
omit the field. Rank position remains discriminating within these
small candidate pools: a high-ranked Agent hint match outranks weak lexical
overlap that merely appears in both channels, while the strongest original-task
match remains within the default top three when it has protectable evidence.
A complete Skill-name token match is direct hint evidence for both single- and
multi-token names; a partial multi-token name match is not.
Smaller limits intentionally return only the corresponding stable prefix.
Hinted retrieval uses a fixed internal pool of up to 100 capabilities, matching
the public maximum limit. For the same Snapshot, task, and hints, changing
`--limit` only bounds the returned matches: each smaller result is a prefix of
a larger result. A hint can therefore surface English
metadata without letting its raw token count set one global cutoff that
discards a strong native-task result. Each task or hint also remains a separate phrase for
exact name, description, and declared-trigger evidence. The scanner reads
ordinary and folded YAML description scalars.

Ordinary Find and its read-only recovery inspection expose `routing_metadata`
for the exact ranked content observation, bound by the enclosing Snapshot,
Skill ID, and `ranking_content_identity_digest`. The preview includes its
description (at most 512 Unicode characters) and up to five declared triggers
(at most 128 characters each), with explicit description/list truncation and
the full trigger count. Descriptions retain exclusions; metadata from distinct
observations is never combined. Missing metadata remains missing. This is
frozen routing evidence for caller-owned interpretation, not the Skill body,
current filesystem verification, or permission to load. Existing weak-evidence,
ambiguity, trust, and content-verification gates remain unchanged.

Find's `evidence_quality` describes the current lexical relevance inference and
remains `inferred`, with `evidence_quality_scope: routing_relevance`. Observed
history for the exact Skill identity is returned separately in optional
`local_usage_prior`: its Skill ID, observed stages, and evidence Agent names.
Stages and Agents are deduplicated facts, not additive event counts or placement
exposure. History never adds to the relevance score or creates a candidate with
no lexical evidence. No stage or recency ranking policy is inferred. The legacy
outer `match_reasons` may retain `observed_local_usage`; successful load
`ranking_evidence` excludes that marker and carries the independent prior with
its original Skill ID, including when explicit variant selection changes which
package is loaded.

An independent punctuation-delimited task clause that begins with `不要`, the
parallel continuation `也不要`, or the English marker `do not`, `don't`, or `don’t` remains
verbatim in `task` but does not contribute positive candidate or ranking
evidence. The bounded coordinators `but`, `and`, `yet`, `但是`, `不过`, and `但`
may immediately precede one of those markers; the original coordinated clause
remains verbatim in `task_exclusions`. Prefixes such as `do nothing` are not
markers. English markers are case-insensitive and require an end or non-alphanumeric
boundary, so `don'tpanic` and `don’tpanic` remain positive text. A coordinator
without an immediate marker remains positive task
text. `task_exclusions` returns the recognized clauses so the Agent can audit
that deterministic split.
Terms already present in the positive task remain shared object evidence; only
the constraint-specific remainder may exclude a Skill whose positive name,
trigger, or description declares the prohibited capability. Agent-authored
hints remain separate positive evidence and cannot cancel a task exclusion.
Within one bounded exclusion clause, `code`, `file` / `files`, `代码`, and `文件` remain shared task-object
context when the same clause contains another searchable constraint token and
the object follows earlier constraint material;
they cannot by themselves exclude review or analysis Skills. For example,
`do not modify files` excludes a declared modification capability, but the word
`files` alone does not exclude a Skill that inspects files. When an object token is
the clause's only capability token or leads the clause, it remains an effective
exclusion. This filter is applied independently to each exclusion clause so
one constraint cannot weaken another.
`task_exclusion_effects` reports the exact affected candidate count plus a
deterministic preview of at most 10 affected Skill identities, ordered by name
and opaque Skill ID. Each preview item contains only the name, opaque ID, and
the positive name, trigger, and description token counts that caused exclusion;
`items_truncated` states whether more affected candidates were omitted. It does
not expose Skill paths or content. An embedded negative state such as `diagnose
why tests do not pass` is not an independent constraint clause and remains
intact.

Ranking uses
top-level `triggers` and the semicolon-separated string
`metadata.skillroster-routing-triggers` as the same declared retrieval
evidence. The latter must be explicitly quoted and remains valid under the
Agent Skills metadata string-value contract; non-string and nested forms are
ignored. Ranking normalizes conservative ASCII plurals and segments contiguous
Han text into overlapping two-character lexical units after removing a bounded
set of common Chinese stop-units. A query containing Han text expands scoring
only to the already-scanned, locally routable Skill set so natural CJK
paraphrases can reach CJK metadata even when SQLite FTS has no whole-token
candidate; Archived Skills remain excluded. Match reasons expose CJK
description and full-text bigram counts. Ranking also treats explicit use and
English/CJK do-not-use description clauses, including `not for` and `不应触发`,
as positive and exclusion evidence, and removes the low-confidence tail
relative to the strongest result. Same-name Skill
identities occupy one ranked capability result and are
reported as explicit variants rather than silently treated as equivalent.
For an ambiguous result, `variant_finding` binds the complete routable identity
set to a same-Snapshot divergent-content Finding and provides its read-only
drilldown. Without a compatible current Report it returns a typed reason and
the minimum read-only analysis action. A drifted routable variant requires a
new Scan before any Finding can be linked; stale Finding IDs are never returned.
When every unreadable variant is still bound to its Snapshot-observed
unconfirmed external directory, this is a source-trust decision, not drift:
`variant_finding.state` is `source_confirmation_required` and its read-only
action opens the current escaping-link Finding. The variants may require
separate exact source-root decisions; no parent or shared root is inferred.
Exact-load actions remain absent until the relevant source is confirmed. A
missing or retargeted link still requires a new Scan. In a mixed group, readable
variants retain their exact-load actions while unreadable variants do not.
The corresponding divergent-content Finding carries the affected placements
and a bounded `choose_same_name_variant` resolution that keeps each digest,
path, Agent, provider, root, and governability fact together. It offers no Plan
until the caller has compared the variants and chosen canonical content.
Variant details keep provider, governance, and path facts bound to one identity;
the response preserves the total count and marks bounded detail truncation.
Find and Finding detail keep bounded `owned_by_agent`, `mutation_scope`, and
compatibility `governable` facts together. Provider-managed results also include
their plugin identity. Planning remains Finding-specific: `planning.supported`
is never true when a proposed demotion, removal, or operation would touch a
non-governable placement. Read-only placements that remain unchanged may stay in
the wider Finding scope. Typed reasons distinguish provider read-only, durable
read-only, untrusted external, and legacy-unknown authority without asking the
Agent to reconstruct policy.

`find --load` keeps ordinary Find ranking compatible and atomically adds a
verified Top-1 content result. It requires one unambiguous routable identity, a
current non-Archived roster state, an eligible placement, complete package and
entrypoint digests from the latest Snapshot, a regular file contained by an
approved root, valid UTF-8, and at most 128 KiB of raw `SKILL.md` bytes. The
load also requires direct selection evidence whenever the Agent supplied a
retrieval hint; weak fused Top-1 overlap remains visible through ordinary Find
but cannot return complete instructions until the Agent refines the hint. The
load also abstains when a CJK task has no Agent-authored hint and Top-1 has no
direct selection evidence. Direct selection requires an exact declared name or
trigger, complete Skill-name token coverage, an exact description phrase with
at least two matching tokens, or complete normalized token coverage for one
task or hint phrase with at least two tokens. Broad
description-token or CJK bigram overlap may preserve an ordinary lexical
candidate but cannot authorize
complete instruction loading. Ordinary Find still returns the bounded lexical
candidates and its actionable hint warning; the blocked load returns no partial
instructions. Known incomplete package fingerprints are not automatically classified as
Snapshot drift. Scan records a typed `entrypoint_read_error` when an authorized
bounded entrypoint read fails and does not fabricate an empty-file digest.
A repeated permission, encoding, or entrypoint-size failure on the same physical
entrypoint remains an unavailable observation: repair that cause before rescanning.
A missing/retargeted entrypoint or a newly readable entrypoint still requires
fresh evidence; unknown legacy read state is not assumed unchanged. Find exposes
bounded `path_availability` facts including the
fingerprint detail, package byte/depth budgets, and a recovery decision that
requires resolving the limit or read-access cause before rescanning. Missing,
changed, or retargeted entrypoints still require a rescan; no incomplete package
is authorized for exact loading. Same-name groups with incomplete fingerprints
expose an `incomplete_package` inspection state and per-variant availability,
with inventory inspection rather than a misleading missing-layout-Finding path.
A healthy explicitly selected variant remains loadable. A successful rescan after package repair must
search the current Snapshot rather than let historical identities exhaust the
candidate budget. Discovery additionally exposes per-candidate `load_readiness`:
`blocked` identifies missing selection evidence, ambiguous variants or declared
content, unavailable current paths, and known placement read blockers such as
the entrypoint transport-size limit. `verification_required` is advisory
eligibility, not successful loading or semantic correctness. Placement facts
follow the verifier's eligible-placement preference, but cached read observations
never authorize content loading. Callers must still honor the separately
returned `content_choice` decision. Exact verification is not performed by this
diagnostic. Find comparison-load actions are offered only when the original
task and hints pass the same direct-selection evidence gate as loading; weak
candidates retain inspection and Agent-authored refinement instead of an
executable action that is already known to fail. Weak-evidence load refusals offer
a Snapshot-bound read-only
Find inspection command preserving the original task and hints. The Agent must
independently select the intended capability before authoring a faithful refined
hint; a ranked name is never automatically converted into an exact-name bypass.
When selection evidence is missing and same-name ambiguity is present, the load
reports the selection-evidence blocker first, matching the prerequisite for
comparison-load actions. After faithful refinement, unresolved content ambiguity
still requires the existing exact comparison flow. Bootstrap recovery shares
one hint-refinement retry across empty, wrong-domain, and weak-evidence results.
The caller judges the bounded metadata's capability and exclusions independently;
for a non-English or mixed task, that retry may retain the English hint and add
one separate, specific task-language capability hint supported by the original
task and inspected metadata. The original task and all load gates remain intact;
generic matching phrases alone do not establish semantic suitability.
An explicit complete Skill name remains direct evidence, so this
boundary does not prevent deliberate mixed-language selection. The
loaded placement is selected only after identity ranking: an eligible
`default_exposed` placement outranks inventory-only exact copies, with path as
the deterministic tie-breaker; when no exposed placement exists, the same
deterministic eligible fallback keeps source-only Skills loadable. Hidden and
nested exact copies remain inventory facts. If the preferred placement fails
verification, an implicit load may try another frozen copy only when its
complete package fingerprint, routing identity digest, and entrypoint digest
match the preferred copy. Each attempt performs the full current verification;
routing equivalence or a shared declared identity alone is insufficient. Explicit
placement selection never falls back. A successful alternate load does not erase
the stale placement's blocker or rescan requirement. The
success result separates selection, complete content, governance, and
verification facts. Digest or path drift, legacy Snapshot data, ambiguity,
Archived state, untrusted source, unreadable content, escape, or oversize fails
the whole command with typed details and no partial body. The 128 KiB limit is a
SkillRoster Agent-transport limit, not an Agent Skills format restriction.
`task_success` remains `not_evaluated`.

When Top-1 is an ambiguous same-name group, ordinary Find additionally returns
one bounded, read-only exact-load action per displayed variant. The Agent may
retry with `--load --variant-skill-id <skill-id>` only for an identity exposed
inside that current ranked group. The selector cannot name an arbitrary catalog
Skill, bypass ranking, revive Archived content, or weaken snapshot, source,
path, digest, UTF-8, and size checks. A successful result identifies both the
ranked group and the explicitly loaded identity. Its `ranking_evidence` remains
evidence for the ranked capability group and is explicitly scoped as such; it
is not attributed to the selected package identity. It supplies content facts
for the model's comparison; it does not endorse, canonicalize, mutate, or
establish task success.

Same-name Finding resolution and Find groups also expose bounded
`entrypoint_comparison` facts from the frozen Snapshot. They distinguish equal
entrypoint digests, differing entrypoints, and missing digest evidence. Equal
entrypoints with differing complete package fingerprints identify differences
in supporting package files; they do not imply that packages are interchangeable.
Counts cover the complete relevant same-name group even when previews are
truncated. Existing `content_choice` and explicit content-selection requirements
remain authoritative; the caller decides what supporting content to inspect.

For divergent content under one declared identity, Find scores each observed
content separately and keeps the strongest match for that logical ID. FTS may
index the union solely to select candidate IDs; it never combines names,
triggers or description tokens from different observations to justify loading.
The matching routing digest identifies which observation supplied the evidence.

For identities with multiple placements, ordinary Find includes `content_choice`
with bounded routing-content groups and each placement's observed name, path,
digests, authority and current read state. A single-placement identity needs no
choice bundle. Exact-placement comparison actions appear only when divergent
contents require that choice; same-name Skill-ID comparison actions remain for
identities that do not require a placement selector. Different
routing digests require explicit content selection; package differences caused
only by root `.gitignore` do not. `find --load` returns
`declared_identity_content_ambiguous` with no partial body when selection is
required. Its recovery opens the same Snapshot's ordinary Find choices rather
than asking for repeated Scans. A declared-identity Finding exposes the same
read-only comparison choices and full detail when the preview is truncated.

`--variant-placement-id` requires `--load` and selects one exact placement from
the current Top-1 capability group. If a Skill selector is also supplied, both
selectors must agree. The actual name and placement ID identify the loaded
content; its selection evidence is scoped to the ranked capability group.
The selector preserves all source, Snapshot, digest, path, size, encoding and
Archived-state checks and never falls back to a different placement on failure.
A drifted or unreadable copy retains its own blocker even when another copy is
healthy. Exact reading does not select canonical content or authorize governance.

Within one Find, advisory package observations may be reused for the same
physical entrypoint after each placement's authority and path checks. They are
discarded when the command ends. Verified loading always performs its own fresh
package validation; advisory observations cannot authorize returning instructions.

All JSON responses use a versioned envelope:

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "report",
  "run_id": "run_...",
  "result": {},
  "warnings": [],
  "error": null,
  "suggested_actions": []
}
```

A generic human error whose file-change state is unavailable reports that state
as unknown. It must not infer that no files changed merely because a command
failed. Operation-specific verified summaries retain their explicit facts.

IDs for Agents, Scans, reports, Skills, placements, Evidence, Findings, Plans, operations, and Receipts are opaque and stable within the local state store. Names and paths are never write-operation identities. Errors carry a stable code, human-readable message, retryability, and relevant IDs or paths. In JSON mode stdout contains exactly one JSON document and never an interactive prompt, progress bar, ANSI sequence, or log line. Terminal output is a polished human interface; Agents consume JSON rather than scraping styled text. The normative human-output, accessibility, progress, confirmation, and responsive-layout requirements live in [cli-ux-spec.md](cli-ux-spec.md).

In JSON mode, valid help and version display requests return a successful `cli`
envelope with `result.kind` (`help` or `version`) and the display `text`.
They exit successfully without initializing local state. Actual argument parse
failures remain `invalid_cli_arguments` errors.

`scan --summary --json` persists the same complete immutable Snapshot as a full
Scan but returns a bounded Agent continuation view: Snapshot and inventory
counts, exact root totals, at most ten actionable root issues with exact
total/truncation facts, aggregate and per-Agent typed session-coverage limits,
their supported next steps, and mutually exclusive `reliable`,
`sampled_limited`, `missing`, `inaccessible`, `excluded`, or `legacy_unknown`
states; source-root policy counts; and the ordinary Report action. Existing
`scan --json` remains the complete diagnostic projection. Agent-generated Scan
actions and the Bootstrap governance workflow use the Summary view; neither
view changes Agent or Skill files.

To avoid repeating identical typed facts, Summary groups Coverage limitations
and supported next steps by their affected Agent IDs. Every group remains an
explicit Agent-to-fact mapping; callers do not infer one Agent's boundary from
another Agent's state.

Selector-free `report --json` returns a bounded Summary with up to three non-informational Findings;
`--summary` is an explicit alias. The exhaustive diagnostic report requires
top-level `report --full --json`. Each selected Summary Finding has a bounded,
read-only `view_finding` suggested action bound to its stable ID. When the
Snapshot has more Findings than the Summary, the direct drilldowns follow Summary order, with optional `list_findings`
pagination afterwards. Array order is not semantic priority or authorization.
The additive `overview` reports `inventory_empty`, `no_review_items`, or
`review_available`, counts informational/review Findings, and scopes the result
to known and explicit Skill roots. All informational Findings remain in full
and paged views. Unknown usage never establishes that a Skill is unused.

`report --finding ID --json` returns compact paged Evidence items by default.
Each item contains the Evidence ID, subject, path, quality, and decision facts;
it does not repeat complete affected-ID, placement, and Evidence collections.
When the Finding belongs to an older Report and a newer persisted Report covers
the latest completed Snapshot, the same response includes bounded
`current_continuity` facts. Stable placement ID intersection is the only join:
the result exposes complete overlap and missing counts, compact current Finding
references, and current path/status facts for the requested historical page.
Unavailable current facts fail soft, and zero overlap is never a resolution
claim. This comparison does not Scan, inspect new Skill content, Plan, or Apply.
`--full` explicitly requests those complete paged records. For escaping links,
the Finding returns a trust-confirmation resolution and observed link targets;
it must not advertise an automatic Plan before the source is confirmed.
Usage Evidence includes the human-readable `skill_name` beside the opaque
`skill_id`, and serializes `agent` with the same canonical public identifier
used by planning commands. Compact and full detail therefore support usage
summaries without a caller-side Skill lookup or Agent-name rewrite.
The usage Finding also exposes one bounded `usage_overview`: five typed stage
counts with explicit units, the complete aggregate session-coverage boundary,
and up to five recent high-signal named Skills. `Exposed` counts placements;
Matched, Loaded, Applied, and Outcome count events. The ordinary terminal view
renders this structure as separate stage, coverage, and observed-Skill blocks
at 60, 80, and 120 columns without printing session paths.

`plan --stdin --json` returns a bounded decision-complete summary: total change counts, operation groups, affected-scope counts with at most ten Skill IDs, before/after impact, risk, reversibility, and the immutable Plan ID. A Finding-derived Roster Plan also persists `selection_evidence`, including forced, target-Agent, cross-Agent, aggregate positive-signal, and stable-fallback Core counts plus at most five named Core selections per Agent. Every selection includes its reason, evidence scope, and source Agent IDs without session paths or raw content. When fallback or cross-Agent selection dominates any affected Agent, the Plan carries typed `uncertainty` with `review_required: true`; absence of usage evidence remains explicitly non-negative. It does not inline filesystem operations, complete Core selections, or complete large ID collections. `plan --show PLAN_ID --json` is the explicit full-detail path; it reads the stored immutable Plan, returns every named Core selection and reason, and never mutates files. Plans created before complete Core selections were persisted remain readable with their original aggregate evidence. A fail-closed `trusted_canonical_sources_required` error is also bounded: at most ten blocked Skills and ten source roots in the envelope, with `error.details.detail.path` as the independent retrieval path when either set is larger.

## 6. Agent presentation contract

The normative conversation flow, one-confirmation Apply behavior, state-dependent primary action, and bootstrap Skill rules live in [agent-experience-spec.md](agent-experience-spec.md).

When the user requested a concrete change proposal and Evidence is sufficient,
the bootstrap Skill instructs an Agent to prepare a complete read-only Plan,
then present:

1. a one-sentence diagnosis;
2. independent Skill count, placement count, default exposure, and observed-use count;
3. up to three review Findings (none when only informational context remains) plus compact rollups that state the complete scale of every Finding group without loading the exhaustive report;
4. prioritized recommendations with current affected scale; measurable
   before/after impact only when a validated Plan provides it;
5. uncertainties, evidence quality, and safety risks;
6. one primary next action and whether confirmation is required.

The wording and layout may adapt to the host Agent, language, and conversation. The facts, IDs, Evidence, and confirmation boundary may not be omitted. No HTML report or GUI is generated. Users can ask follow-ups such as “show the 27 duplicates” or “why is this a duplicate”; the Agent resolves them against the same report ID with `report --finding` rather than silently rescanning.

Every summary states whether files were changed. An applied summary also includes the Plan ID, changed-path count, verification status, and Receipt ID. A Ready Plan is presented with one conversational Apply action; after the user confirms once, the Agent executes the whole approved scope without per-operation prompts. SkillRoster does not invent an aggregate health score or unsupported token/performance savings.

The human terminal surface uses concise headers, aligned facts, semantic color, TTY-only progress, strong final summaries, and explicit confirmation. It supports `NO_COLOR`, non-TTY/plain output, narrow terminals, and Unicode fallback. It is not a full-screen TUI and never changes the Agent JSON contract.

## 7. Local data and privacy

SkillRoster uses one SQLite database at `~/.skillroster/skillroster.db`, including FTS5. It has no daemon, cache service, cloud backend, account, RBAC, or telemetry.

- Source sessions are read-only and remain in their original locations.
- Derived event summaries are retained for 180 days by default.
- Usage Observations have a stable identity across Scans. An unchanged rescan
  does not create another raw observation. A changed bounded source window is
  recorded as a new observation, not inferred as an additive event delta.
- Usage event windows use timestamps carried by the session record. Session
  file modification times describe coverage only and must not refresh old
  Skill events; when a record has no trustworthy timestamp, event time remains
  unknown. The local observation time may govern retention for that record but
  is never presented as the event time.
- Older usage is reduced to the maximum observed count per source and month.
  These maxima are conservative source-window facts, not cumulative totals.
- Pre-v9 monthly rows cannot be safely reconstructed by source. They are
  retained separately with `derivation=legacy_scan_aggregate` and must not be
  combined with source-month observations.
- Plans and Receipts remain until explicitly purged.
- Overflow source-confirmation details are versioned local artifacts retained until explicitly purged or local state is deleted.
- `status` exposes storage location, retention state, and retained source-confirmation artifact counts and bytes.
- Home is a bounded projection of the same current-state and continuation
  decision used by `status`. It never reports `ready` merely because a completed
  but invalidated Snapshot exists, and it never leaves a missing Snapshot or
  recovery state without the exact read-only continuation.
- `status` remains read-only and available when current inventory facts are
  invalid. It exposes typed `snapshot_state`, the invalidating Receipt ID, and
  the required Scan continuation. Recovery inspection remains higher priority;
  stale-Snapshot refresh remains higher priority than inspecting another Ready
  Plan derived from those stale facts.
- The v11 migration backfills mutation invalidations from legacy second-resolution
  timestamps. When a legacy Undo and the latest Scan completed in the same
  second, their exact order is unknowable; migration fails closed and requires
  one fresh Scan. New v11 mutations persist the exact invalidation relation and
  do not use this timestamp inference.
- `status.pending_plans` contains only actionable Ready Plans for the latest
  Snapshot plus any Applying or Recovery-required Plans. Ready Plans from older
  Snapshots remain inspectable history but are not presented as pending work.
  The total count remains exact while the returned list is capped and reports
  whether additional pending Plans were truncated.
- When `snapshot_state == current` and pending work exists, `status` routes the
  Agent to inspect the first Plan from that deterministic bounded ordering
  before considering a new Scan. Recovery inspection remains higher priority,
  and the action is read-only.
- When `snapshot_state == current`, `status` does not suggest an unconditional
  Scan. A healthy state exposes the Snapshot timestamp and leaves refresh
  judgment to the Agent. Missing or invalidated Snapshots instead expose the
  required Scan continuation described above.
- Users can inspect, export, purge, or delete retained local state and rebuild inventory by scanning again.

Successful local-state deletion returns a typed `scan` suggested action that
retains the invoking executable, Home, state directory, explicit Agent roots,
and temporary source roots. Reusing temporary source roots retains the existing
confirmation requirement. Deletion does not execute the action. Use its literal
`argv` for continuation; the legacy `rebuild_command` string is generic syntax,
not a reconstruction of the caller's context. A new Scan rebuilds inventory;
it does not restore deleted history, evidence exclusions, or durable source
permissions.

Reports may identify structural and provenance risks—unknown source, changed content, executable scripts, declaration mismatch, and escaping links—but must not claim malware detection or runtime safety.

## 8. Scope constraints

The complete first release deliberately contains:

- one Rust binary;
- one SQLite database;
- one bootstrap Skill;
- eight direct Agent adapters;
- core-logic and high-risk mutation tests.

It does not contain HTML reports, a GUI, an interactive or full-screen TUI, MCP, cloud services, accounts, telemetry, a daemon, a plugin SDK, generic Agent adapters, a workflow engine, a second Agent, or built-in model calls. Ponytail is a development experiment only and creates no product dependency or product-specific behavior.

An abstraction requires at least two real consumers. Otherwise, prefer direct code.

## 9. Completion and value gates

The product may progress through internal milestones, but `1.0` is complete only when the full loop passes:

- supported adapter fixtures achieve 100% expected discovery;
- Apply and Undo restore 100% of covered fixture states;
- no unapproved canonical deletion occurs;
- every Finding is traceable to paths and Evidence;
- on inventories over 100 Skills, a reviewed proposal can reduce default exposure by at least 50%;
- On-demand Skill routing reaches at least 95% Top-3 recall on the maintained evaluation set;
- routing does not regress the task-success baseline;
- a normal local analysis completes in at most five minutes on the reference large inventory;
- macOS, Linux, and Windows each pass a real-environment acceptance run;
- small, large, and cross-Agent duplicate scenarios pass the Agent presentation review.

Comparison runs cover unmanaged baselines, careful manual governance, and relevant Skill managers. Ponytail may appear only as a development-complexity comparison.

## 10. Delivery milestones

Milestones are implementation checkpoints, not reduced product promises:

1. **Fact Scan:** adapters, identity, SQLite inventory, stable JSON.
2. **Diagnosis:** seven report categories and evidence drill-down.
3. **Usage Evidence:** local session parsers, evidence levels, retention.
4. **Library and Rosters:** Observed, Managed, Hosted and four exposure layers.
5. **Search and Routing:** FTS5, `find`, and routing evaluation.
6. **Safe Change:** Plan, Apply, journaling, Receipt, Undo, and drift refusal.
7. **Complete Acceptance:** bootstrap Skill, three platforms, eight Agents, value comparisons, and release documentation.
