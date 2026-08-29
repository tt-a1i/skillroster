# Roster recommendation pilot protocol v1

Status: frozen protocol; synthetic dry run complete; no real participant has
been recruited or observed.

## Question and evidence boundary

This qualitative pilot asks whether non-implementer participants can receive a
bounded SkillRoster diagnosis and judge proposed Core and On-demand decisions
without a maintainer composing the CLI workflow for them.

It does not measure population-level preference, token or labor savings,
production performance, model quality, or universal recommendation
superiority. Recommendation agreement, deterministic retrieval, and final task
success remain separate outcomes.

## Participant and authority boundary

- Schedule at least three participants who did not implement the tested
  recommendation behavior. Report every scheduled participant, including an
  abandoned or blocked run.
- Obtain direct consent before messaging, reading a real environment, or
  starting a run. Consent text, names, contact details, and conversation text
  are not copied into the evidence ledger.
- Agree on the supported Agent set and the exact read-only diagnostic scope
  before Scan. The operator records only `authority_verified: true` after that
  check succeeds.
- Real Apply is optional, is not needed for pilot success, and requires a new
  explicit authorization. Refusal to Apply is not a failed run.
- Recruitment, participant messaging, real-home reads, and real Apply are
  external boundaries. Freezing this protocol authorizes none of them.

## Stage classification

Each recommendation records the following stages independently:

1. `setup`: the published package and Bootstrap path are available;
2. `invocation`: the Agent successfully invokes the public SkillRoster path;
3. `diagnosis`: the participant receives a bounded read-only diagnosis;
4. `plan`: a decision-ready Plan or an accurate typed blocker is returned;
5. `deterministic_retrieval`: the relevant On-demand Skill is deterministically
   found and loaded when that check is applicable;
6. `recommendation_decision`: the participant accepts, rejects, is blocked, or
   does not evaluate the recommendation;
7. `final_task`: the participant's task result, if evaluated.

Setup, invocation, diagnosis, Plan, deterministic retrieval, recommendation,
and final-task failures must not be relabelled as one another. A retrieval
failure and a recommendation rejection may coexist; neither explains the
other. Later stages remain `not_evaluated` when an earlier required stage did
not run.

## Bounded ledger

The machine ledger format is
`skillroster-roster-recommendation-pilot`, schema 1. It accepts 3–32 unique
opaque pseudonyms and at most 64 recommendation records per participant.
Participant records contain only:

- `pseudonym`: a short opaque label not derived from a person's name;
- `run_status`: either `observed` or `abandoned_before_observation`;
- `supported_agents`: one or more of the eight supported Agent identifiers;
- `aggregate_inventory`: non-negative `skill_count`, `placement_count`,
  `default_exposure`, and a bounded `session_coverage` state;
- `recommendations`: a bounded recommendation category, outcome, reason
  category, and the seven stage results;
- `safety_outcome`: authority verification and zero/false privacy and mutation
  counters.

An `abandoned_before_observation` record has `null` aggregate inventory and
empty Agent and recommendation arrays. It remains in the participant total but
cannot contribute diagnosis or Plan readiness. This records an early exit
without inventing zero inventory, a recommendation, or authority that was never
observed.

Recommendation categories are `core_general`, `core_agent_specific`,
`on_demand_general`, `on_demand_agent_specific`, `protected_existing`, and
`source_blocked`. They deliberately omit Skill names, IDs, descriptions,
contents, and paths.

Outcomes are `accepted`, `rejected`, `blocked`, and `not_evaluated`. Reasons are
bounded to `accepted_as_proposed`, `personal_preference`,
`insufficient_evidence`, `incorrect_identity`, `unsuitable_source`,
`other_bounded`, and `not_evaluated`. The validator rejects an outcome/reason
mismatch instead of accepting free text.

The ledger contains no raw conversation, prompt, Skill body, secret, absolute
path, participant identity, contact detail, or open-ended explanation. Exact
object-key validation rejects extra fields before aggregation. The summary
contains only counts and never emits pseudonyms.

## Safety gate and stop rules

The safety gate passes only when every observed run records verified authority,
every scheduled run records zero unapproved writes, and none persists raw
conversation, Skill content, secret, or identifying path. An abandoned run may
record unverified authority only when it also records no observation facts.

- Stop and report the run if the safety gate fails. Do not hide it with a rerun.
- Do not infer that missing usage Evidence means a Skill is unused.
- Do not change ranking, add embeddings or model calls, or introduce persistent
  preference policy because one participant disagrees with a recommendation.
- A repeated preference category is only input to a separate product decision
  and specification. The pilot summary always reports every product-change
  authority flag as false.
- At least three participants must receive a bounded diagnosis, and at least
  two must reach a decision-ready Plan or typed blocker before the real pilot
  can satisfy its readiness gates.

## Reproduction

The frozen synthetic input exercises acceptance, rejection, a typed blocker,
an Agent invocation failure, and a deterministic retrieval failure without any
real participant or environment data:

```bash
node scripts/recommendation-pilot.mjs summarize \
  --input tests/fixtures/roster-recommendation-pilot-v1.synthetic.json

node scripts/recommendation-pilot.mjs report \
  --input tests/fixtures/roster-recommendation-pilot-v1.synthetic.json
```

The checked-in [dry-run receipt](../acceptance/roster-recommendation-pilot-v1-dry-run.md)
records the accepted synthetic result. A real run requires the separate
authority and participation boundary tracked by
[#261](https://github.com/tt-a1i/skillroster/issues/261).

The command accepts only a regular, non-symlink ledger file and reads at most
1 MiB through its opened descriptor before JSON parsing.
