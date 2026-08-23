# Routing details

The root Route gate is authoritative. Read this file for an empty or
wrong-domain result, `variant_count > 1`, `variants_truncated`, or
`report_required`. Keep `TASK` verbatim. A non-English or mixed task receives
exactly one faithful English `--hint` on the first call.
Build the hint from the desired surface, object, operation, and state; never
guess a Skill name.

With a hint, require
`ranking_strategy: task_hint_reciprocal_rank_fusion`. Use `task_channel_rank`
and `augmented_channel_rank` to distinguish native task evidence from
hint-expanded evidence. Retry at most once, only for an empty or wrong-domain
result. Change only the hint; an English task may add one on its retry.

When `variant_count` is above one, keep each path and provider together. Respect
`variants_truncated` and run the returned read-only `variant_finding.argv`.
Materialize the current Report first only when the result says
`report_required`; inspect that exact Finding before choosing content.

Only after ambiguity is resolved, or when `variant_count == 1`, read the
selected result's exact `SKILL.md` path directly. Finding a Skill does not
activate, install, or authorize it, and SkillRoster has no load or activate
step.

Provider-managed results with `governable: false` are valid read-only matches.
Do not move, update, consolidate, or add them to a governance Plan.
