# Coordinated task-exclusion dogfood v1

## Problem and value

Public SkillRoster v1.8.39 correctly excluded a prohibited capability when an
independent clause began directly with `do not`, `不要`, or `也不要`. Real Agent
wording often introduces the same clause with a coordinator. Against one fresh
read-only Snapshot of 263 Skills and 1,037 placements, this task returned an
empty `task_exclusions` list and left `simplify-code` in the first five matches:

```text
Review this code for correctness and security, but do not simplify or refactor it
```

Replacing only `, but do not` with `; do not` produced the expected exclusion
and removed the simplify candidate. The parallel Chinese comparison showed the
same split between `但是不要` and a direct `不要` clause. This is a routing-safety
bug: the person explicitly prohibited a capability, but the deterministic Find
contract presented it as positive evidence.

## Diagnosis

The feedback loop used the anonymous public macOS arm64 v1.8.39 archive, the
real local Skill Inventory, and a fresh temporary state directory. Scan and all
Find calls reported `files_changed=false`; no Agent or Skill files were
modified.

Ranked hypotheses were tested one variable at a time:

1. The clause parser required an exclusion marker at byte zero and did not
   recognize a leading coordinator. Confirmed: removing only the coordinator
   made the failure disappear.
2. The real `simplify-code` name differed from the fixture's
   `simplify-codebase`. Rejected: both were excluded after the punctuation-only
   differential.
3. Observed usage evidence overwhelmed the exclusion penalty. Rejected: the
   failing result contained no exclusion effect at all.
4. The published binary had drifted from the source behavior. Rejected: the
   public tag and current parser shared the same boundary.

## Fix and regression boundary

[Issue #344](https://github.com/tt-a1i/skillroster/issues/344) bounds the fix.
The parser now recognizes only six explicit coordinator prefixes immediately
before an existing exclusion marker: `but`, `and`, `yet`, `但是`, `不过`, and
`但`. It keeps the person's original coordinated clause in JSON evidence, then
removes the coordinator and marker only for capability-token comparison.

The public CLI regression exercises English and Chinese coordinated clauses
through Scan and Find, proves that `code-review` stays Top-1 in the controlled
fixture, proves that `simplify-codebase` is absent, and keeps
`files_changed=false`. Unit coverage separately retains the negative-state
control `diagnose why tests do not pass`, so this bounded grammar does not turn
embedded prose into an exclusion.

The original 263-Skill replay now returns the coordinated clauses in
`task_exclusions` and removes the observed simplify candidates in both
languages. This remains lexical routing evidence, not general pronoun
resolution or semantic-negation understanding.

## Retrospective

The v1.8.39 fixture proved a direct negative clause but its release note used a
more natural coordinated sentence that the fixture did not execute. The
preventive change is to bind every public routing example to the exact public
CLI regression string, including punctuation and coordinators. Future rounds
should continue replaying realistic Agent phrasing against the full Inventory,
not infer broad behavior from a nearby synthetic sentence.
