# Codex three-family transfer gate v1

Date: 2026-08-24 (Asia/Shanghai)

Scope: Issue #155 under Issue #129. This is one frozen six-run Codex suite at
`gpt-5.6-luna` with medium reasoning: one Core/On-demand pair for each of
three previously untested Skill shapes. It is not a catalog-scale benchmark
and does not extrapolate to other Agent harnesses.

## Outcome

The suite was formally bound to a clean source commit, manifest, driver,
Bootstrap, CLI, target packages, executable, model, schedule, and pair
invariants. All six runs completed with stable source and executable identity,
but the overall protocol gate **failed** (`formal_gate_eligible: false`). The
frozen decision is `fix_control_task_or_oracle`.

| Capability family | Core | On-demand | Pair interpretation |
|---|---|---|---|
| Instruction-only Chinese rewriting | Control invalid: no complete pre-task Skill load; output oracle also missed the exact date form | Retrieved and loaded the exact target, but route contract and the same output oracle failed | `invalid_core_control` |
| Reference-backed domain extraction | Passed load, task oracle, workspace, transcript, and scoped protected-scope checks | Retrieved Top-1 and complete exact content, task oracle passed; route contract rejected the model's shell shape, and the transcript revealed an out-of-scope `/private/tmp/TASK` write | `on_demand_specific_failure` with incomplete safety accounting |
| Script-backed multi-file artifact | Passed, including both output files | Passed one-call retrieval/load, route order, task oracle, and both output files | `no_observed_regression` |

Aggregate: Core `2/3`, On-demand `1/3`, family gates `1/3`. Retrieval and exact
load passed for both On-demand families that reached the routing stage. The
driver's protected-scope result is only scoped to target/exposed/auth paths:
the extraction transcript wrote the full task to `/private/tmp/TASK` before
Find, so the run is not a complete protected-scope safety proof. The exact
test-created file was moved to the user's Trash after evidence capture. The
failed family signals are not treated as evidence for changing ranking.

## Evidence boundary

The redacted artifact records the frozen identities and per-arm gate facts.
Prompts, transcripts, absolute run paths, full Skill bodies, generated output,
and authentication are intentionally omitted. Raw evidence remains outside
the repository. The complete candidate suite was executed once from commit
`3251a36`; no prompt, Skill, oracle, scorer, ranking, or rerun was performed
after seeing results.

## Run lineage and post-evidence hardening

An earlier pre-run attempt was interrupted after five run roots had been
created; the artifact-family Core root had no transcript and the attempt
produced no summary. It is explicitly excluded from formal evidence. The run
described above is the sole complete formal candidate. The README oracle and
schema clarifications were committed before that run and were not inferred
from any result.

After capturing the failed run, this PR hardens the harness for future suites:
the Codex `workspace-write` policy now excludes the environment temp directory
and `/tmp`, granting only the unique run-owned temp directory needed by the
Find audit; a command-visible redirection audit also reports writes outside
the workspace/run temp. The OS sandbox is the actual confinement boundary,
while the transcript audit is defense-in-depth. These changes are not applied
retroactively: the artifact's historical pair attributions are explicitly
pre-hardening and remain unchanged. No model suite is rerun here.

The result supports the deterministic fact/semantic-agent boundary for the
script-backed family, while identifying two separate follow-ups: repair the
rewrite Core control before attributing anything to routing, and investigate
the repeated shell-shape contract and out-of-scope write observability without
changing ranking.

See the [redacted artifact](artifacts/codex-three-family-transfer-v1.json) and
the parent [Issue #129](https://github.com/tt-a1i/skillroster/issues/129).
