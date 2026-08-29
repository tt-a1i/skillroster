# Roster recommendation pilot v1 synthetic dry run

## Result

The frozen protocol's synthetic dry run passes. It uses three invented
pseudonyms, aggregate fixture facts, five bounded recommendation records, and no
real participant, Agent session, Skill body, secret, or identifying path.

- participants reported: 3/3;
- observed participants: 3; abandoned before observation: 0;
- participants with a bounded diagnosis: 3;
- participants reaching a decision-ready Plan or typed blocker: 3/2 required;
- recommendation outcomes: 1 accepted, 1 rejected, 1 blocked, 2 not evaluated;
- deliberately separate failures: 1 Agent invocation failure and 1
  deterministic retrieval failure;
- final-task results: 2 passed, 3 not evaluated;
- safety gate: passed, with zero unapproved writes and all persisted-private-data
  counters at zero;
- ranking, embedding, model, and policy changes authorized: none.

The machine summary is
[`roster-recommendation-pilot-v1.synthetic-summary.json`](artifacts/roster-recommendation-pilot-v1.synthetic-summary.json).
It is reproduced from
[`roster-recommendation-pilot-v1.synthetic.json`](../../tests/fixtures/roster-recommendation-pilot-v1.synthetic.json)
through the public research-harness command documented in the
[frozen protocol](../research/roster-recommendation-pilot-v1.md).

## Privacy and classification checks

The validator rejects extra object fields, free-form recommendation identities,
path-shaped categories, invalid Agent names, outcome/reason mismatches, duplicate
pseudonyms or labels outside the bounded grammar, and
stage sequences that claim later work after setup or invocation did not pass.
The CLI also rejects symlink, non-regular, and larger-than-1-MiB inputs before
unbounded JSON parsing. A scheduled participant who exits before observation
can be retained with typed status and empty observation facts instead of being
omitted or assigned invented zero values.
Aggregation removes pseudonyms and reports setup, invocation, diagnosis, Plan,
deterministic retrieval, recommendation decision, and final task independently.

This run proves only that the protocol, validator, aggregation, redaction, and
reporting path behave on synthetic input. It does not prove that an independent
participant can use SkillRoster successfully or agrees with its recommendations.
That external evidence remains owned by
[#261](https://github.com/tt-a1i/skillroster/issues/261) and requires direct
participant consent before any recruitment, messaging, or real-environment read.
