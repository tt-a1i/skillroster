# Routing details

The root Route gate is authoritative. Read this file for a wrong-domain result
or `verified_skill_load_blocked`. Keep `TASK` verbatim. A non-English or mixed
task receives exactly one faithful English `--hint` on the first call.
Build the hint from the desired surface, object, operation, and state; never
guess a Skill name.

With a hint, require
`ranking_strategy: task_hint_reciprocal_rank_fusion`. Use `task_channel_rank`
and `augmented_channel_rank` to distinguish native task evidence from
hint-expanded evidence. Retry at most once, only for an empty or wrong-domain
result. Change only the hint; an English task may add one on its retry.

When the blocker reason is `same_name_variants_ambiguous`, use the ordinary
Find result and its read-only `variant_finding.argv`; keep each path and provider
together. Materialize the current Report only when the result says
`report_required`, then inspect that exact Finding before choosing content.

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
