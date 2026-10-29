# Routing details

The root search workflow is authoritative. Read this file for a wrong-domain result
or `verified_skill_load_blocked`. Keep `TASK` verbatim. A non-English or mixed
task receives exactly one faithful English `--hint` on the first call.
Build the hint from the desired surface, object, operation, and state; never
guess a Skill name.

With a hint, require
`ranking_strategy: task_hint_reciprocal_rank_fusion`. Use `task_channel_rank`
and `augmented_channel_rank` to distinguish native task evidence from
hint-expanded evidence. A match with
`ranking_adjustments: ["protected_original_task_match"]` was deliberately moved
by the documented original-task protection rule after fusion; interpret its
rank/score order through that adjustment. Empty results, wrong-domain results,
and the selection-evidence blockers below share one hint-refinement retry.
Snapshot refresh and unchanged read-only inspections do not reset that budget.

## Refine selection evidence

For a weak match, execute the returned read-only `inspect_ranked_skill_evidence`
action. Judge whether the candidate's `routing_metadata` supports the requested
operation and object, including the description's exclusions. Keep the metadata
bound to its Skill ID and `ranking_content_identity_digest`. Truncated or missing
metadata may leave that judgment unresolved; it is not evidence of suitability.

Use the single refinement to express the intended capability concisely; preserve
the complete `TASK` and its constraints. When refining from an inspection,
retain its executable, effective context flags, discovery overrides, and
`--require-snapshot`; change the hint arguments and restore `--load --limit 1`.
On `find_snapshot_changed`, follow the returned read-only rerun action and judge
the new evidence without resetting the refinement budget.
An English task may add a capability
hint if it had none. For a non-English or mixed task whose inspected metadata
supports that capability in the task's language, retain the faithful English
hint and optionally add one separate `--hint` in the original task's language.
Both are literal argv values. For description-based evidence, a reliable way
to express the independently chosen capability is to select a short, specific
operation/object phrase from the inspected description and reuse its words
unchanged. Normalized lexical matching does not equate synonyms; adding even a
faithful synonym can leave a complete hint phrase unsupported. The complete
TASK retains its other constraints. Generic editing or boilerplate phrases are
insufficient to establish the requested specialized capability. A
ranked name alone does not justify an exact-name hint. If the candidate does not support
the task, use the wrong-domain fallback instead of manufacturing direct evidence.
After this retry, follow remaining typed blockers or the root fallback; no
further hint refinement is available. Passing the lexical gate still requires
independent semantic judgment and complete content verification.

## Resolve content ambiguity

When the blocker reason is `same_name_variants_ambiguous`, execute its returned
read-only `inspect_same_name_variants` action. Do not reconstruct or rerun Find:
the action preserves the original task and hint and binds the recovery to the
same Snapshot. Keep each path and provider together. Materialize the current
Report only when the returned result says `report_required`, then inspect that
exact Finding. Use only the returned `load_exact_variant_for_comparison`
actions to load the exact identities under comparison. If the recovery fails
with `find_snapshot_changed`, execute only its returned read-only
`rerun_find_on_latest_snapshot` action, then use the new result's Snapshot-bound
exact variant actions. Require the requested and loaded Skill IDs to match and
retain each identity's content, path, provider, and governance facts together.
Compare the complete entrypoint instructions semantically and treat any choice
as model-owned. Identical entrypoint digests do not resolve a divergent package
fingerprint; report the remaining package ambiguity instead of choosing a
canonical identity. Exact loading does not canonicalize content, modify a
Roster, or authorize a later Plan.

When the blocker reason is `declared_identity_content_ambiguous`, execute its
returned read-only `inspect_declared_identity_content` action. This ordinary Find
preserves the original `TASK`, hints, and Snapshot. Inspect `content_choice.groups`
by `content_identity_digest`, retaining each placement's `placement_id`, `path`,
`name`, digest, package fingerprint, mutation scope, and read state. For truncated
choices, use the returned Finding reference and `report --finding --full` to read
the complete choices. Use the returned `load_exact_content_for_comparison` actions
with `--variant-placement-id` to read the exact observed content. Verify that
`loaded_skill.selection.placement_id` and `loaded_skill.selection.name` match
the requested placement,
and require complete content and all identity/digest checks. Its
`loaded_skill.selection.ranking_evidence_scope: ranked_capability_group`
describes the ranked group's
relevance; it does not rank the individual placement's content. Compare the
complete instructions while keeping placement facts together. The comparison
selects no canonical content and authorizes no Plan. If the Snapshot changed,
follow the returned read-only `rerun_find_on_latest_snapshot` action and use the
new result's exact content actions.

For `no_routable_match`, retry once with a refined capability hint. For
`cjk_hint_required_for_weak_match`, keep `TASK` unchanged, author the one
faithful English capability hint required by the root search workflow, and retry
once. For `hint_direct_selection_evidence_required`, use the selection-evidence
procedure above before any same-name or declared-content comparison. These
branches consume the same single refinement budget, including when the next
result has a different blocker. For drift, legacy Snapshot, unreadable,
oversized, escaping, or untrusted-source reasons, follow
`error.details.next_action`; never bypass the check or recover partial
instructions. A successful load returns complete
instructions but does not activate, install, authorize, endorse, or establish
task success.

Read `owned_by_agent` as placement-path structure only, never as ownership or
endorsement of linked source content. Read `mutation_scopes` before governance:
only `mutable` may participate in a mutating Plan. `provider_read_only`,
`durable_read_only`, `shared_read_only`, and `untrusted_external` remain valid
routing results but must not be moved, updated, consolidated, or added to a
governance Plan. Missing
scope facts mean a legacy Snapshot is unknown; rescan instead of inferring
authority. `governable` remains a compatibility projection of `mutable`.
