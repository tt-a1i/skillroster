# Routing details

The root Route gate is authoritative. Read this file for a wrong-domain result
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
rank/score order through that adjustment. Retry at most once, only for an empty
or wrong-domain result. Change only the hint; an English task may add one on its
retry.

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

For `no_routable_match`, retry once with a refined capability hint. For drift,
legacy Snapshot, unreadable, oversized, escaping, or untrusted-source reasons,
follow `error.details.next_action`; never bypass the check or recover partial
instructions. A successful load returns complete instructions but does not
activate, install, authorize, endorse, or establish task success.

Read `owned_by_agent` as placement-path structure only, never as ownership or
endorsement of linked source content. Read `mutation_scopes` before governance:
only `mutable` may participate in a mutating Plan. `provider_read_only`,
`durable_read_only`, and `untrusted_external` remain valid routing results but
must not be moved, updated, consolidated, or added to a governance Plan. Missing
scope facts mean a legacy Snapshot is unknown; rescan instead of inferring
authority. `governable` remains a compatibility projection of `mutable`.
