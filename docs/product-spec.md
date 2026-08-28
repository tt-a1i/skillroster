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

Supported platforms are macOS, Linux, and Windows. WSL is treated as Linux. A platform or Agent is not declared supported until its adapter passes fixtures and a real-environment acceptance run.

SkillRoster scans only known directories for these eight Agents, the active SkillRoster-owned Library root, Skill roots from Codex plugins proven by either explicit local enablement or a valid local remote-plugin install marker, plus paths explicitly provided by the user. It never crawls the entire home directory or treats an arbitrary plugin cache as installed. Explicitly disabled plugins remain excluded. Discovered Codex plugin Skills are Observed, searchable, and provider-managed read-only: they cannot become mutation targets or canonical Library sources. Invalid install markers and ambiguous cached plugin versions fail closed. Every new Snapshot records two orthogonal placement facts: `owned_by_agent` describes only whether the placement path is structurally inside an Agent root, while `mutation_scope` is `mutable`, `provider_read_only`, `durable_read_only`, or `untrusted_external`. Placement-path ownership never endorses linked source content. Compatibility field `governable` is true exactly for `mutable`; missing authority fields in legacy Snapshots remain unknown and cannot authorize mutation. Every Scan reports included, excluded, missing, and inaccessible roots. `--root AGENT=PATH` is an Agent placement root and contributes to exposure; `--source-root PATH` is an approved canonical source with no Agent identity and no default exposure. Source trust is explicit and scoped to the resolved canonical directory; a path and a symlink alias to that same directory have identical trust semantics.

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

The CLI owns deterministic discovery, normalization, hashing, indexing, statistics, validation, mutation, and recovery. It returns bounded, decision-complete facts and typed options so the Agent does not need to rediscover filesystem state or scrape prose. Suggested action argv preserves the effective local state and discovery overrides, keeping follow-up commands on the same Snapshot and explicit source-trust boundary. It validates the Agent's structured choices but does not replace model reasoning with embedded intent classifiers, semantic policy trees, or a second prompt framework. Human-readable terminal output supports audit, diagnosis, and fallback use; versioned JSON is the primary Agent integration contract.

Use this placement test for every capability: local truth, stable identity, reproducibility, safety, and reversible execution belong in the CLI; context-sensitive meaning and open-ended judgment belong in the Agent. The person retains preference and material-change authority. The binary contains no model API or second Agent.

## 4. Capability requirements

### 4.1 Inventory and identity

A read-only Scan discovers Skill roots, `SKILL.md` entry points, source metadata, links, configuration exposure, and supported local session sources. It normalizes paths without following links outside approved roots silently. Entrypoint discovery traverses root and category directories within its depth bound. Once a directory declares `SKILL.md`, arbitrary descendants are package support content rather than more entrypoint search space; only direct child directories that also declare `SKILL.md` continue the nested-Skill chain. Repository and build trees `.git`, `target`, and `node_modules` are excluded from entrypoint discovery. These exclusions do not weaken package fingerprints: support files remain identity- and drift-bearing under the independent fingerprint bounds.

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

Before a large-Roster Finding advertises a Ready-Plan template, SkillRoster derives the proposed Plan without persisting it. If multiple Skill identities would claim one name-derived Library target, planning fails closed with every claimant's exact Skill ID and original name. Library target identity uses a conservative ASCII case-fold so the same Plan is safe on supported case-sensitive and case-insensitive filesystems. A true same-name group links the matching same-Snapshot divergent-content Finding when available; distinct names that normalize to one safe directory use a separate typed reason and never claim that a same-name Finding exists. It never chooses a canonical variant by rank, path, or recency. Full Finding detail may instead offer one exact request that keeps every conflicting identity Core, but only after that request passes the production Recommendation, placement-safety, and Plan derivation paths without a duplicate physical target. Validation recursively closes over later target-claim groups and discloses each target, original name, and Skill ID before confirmation. Compact claimant and protected-ID lists are bounded and cannot expose a template from an incomplete closure. The generic physical-operation conflict check remains the final guard.

The CLI validates targets, ownership, conflicts, and current fingerprints, then stores an immutable Plan. Exact-duplicate Library Plans report comparable before/after physical-source, placement, and default-exposure counts plus explicit deltas, so source consolidation cannot be mistaken for Roster reduction. `apply <plan-id>` executes only the approved scope. Cross-filesystem operations use a journal and compensating steps; the CLI never claims atomicity it cannot provide.

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
weakening the original path's attributes. SkillRoster does not claim portable
preservation of owner, ACLs, or extended attributes.

Apply, compensation, and Undo open approved roots and the private state root as
directory capabilities before mutation. Filesystem paths are resolved relative
to those retained handles for fingerprinting, copy, rename, replacement, and
link operations. If an opened root and its directory entry no longer identify
the same directory, or if an ancestor symlink would escape the handle, the
operation fails closed without touching the escaped target.

### 4.5 Source updates

SkillRoster records source, revision, and content hash when available. A source-update request submitted through `plan --stdin` first produces a diff and Plan; there is no silent update path. Local modifications stop automatic replacement and require an explicit choice to retain local content, adopt upstream content, or preserve both.

## 5. CLI contract

The first complete command surface is:

```bash
skillroster [--json]
skillroster scan [--summary] [--json]
skillroster report [--summary | --full | --findings [--category <category>] [--severity <severity>] | --finding <id> [--full]] [--limit <n>] [--offset <n>] [--json]
skillroster find [--hint <text>]... [--limit <n>] [--load] [--variant-skill-id <skill-id>] [--require-snapshot <scan-id>] [--json] -- <task>
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
Smaller limits intentionally return only the corresponding stable prefix.
Hinted retrieval uses a fixed internal pool of up to 100 capabilities, matching
the public maximum limit. For the same Snapshot, task, and hints, changing
`--limit` only bounds the returned matches: each smaller result is a prefix of
a larger result. A hint can therefore surface English
metadata without letting its raw token count set one global cutoff that
discards a strong native-task result. Each task or hint also remains a separate phrase for
exact name, description, and declared-trigger evidence. The scanner reads
ordinary and folded YAML description scalars. Ranking uses
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
do-not-use description clauses as positive and exclusion evidence, and removes
the low-confidence tail relative to the strongest result. Same-name Skill
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

IDs for Agents, Scans, reports, Skills, placements, Evidence, Findings, Plans, operations, and Receipts are opaque and stable within the local state store. Names and paths are never write-operation identities. Errors carry a stable code, human-readable message, retryability, and relevant IDs or paths. In JSON mode stdout contains exactly one JSON document and never an interactive prompt, progress bar, ANSI sequence, or log line. Terminal output is a polished human interface; Agents consume JSON rather than scraping styled text. The normative human-output, accessibility, progress, confirmation, and responsive-layout requirements live in [cli-ux-spec.md](cli-ux-spec.md).

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

Selector-free `report --json` returns the bounded three-Finding Summary view;
`--summary` is an explicit alias. The exhaustive diagnostic report requires
top-level `report --full --json`. Each selected Summary Finding has a bounded,
read-only `view_finding` suggested action bound to its stable ID. When the
Snapshot has more Findings than the Summary, `list_findings` remains the first
action for pagination and the direct drilldowns follow in Summary order.

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
3. the three most important Findings plus compact rollups that state the complete scale of every Finding group without loading the exhaustive report;
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
