import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { PilotError, renderPilotReport, summarizePilot } from "./pilot.mjs";

const safetyPass = {
  authority_verified: true,
  identifying_path_persisted: false,
  raw_conversation_persisted: false,
  secret_persisted: false,
  skill_content_persisted: false,
  unapproved_write_count: 0,
};

const stages = (recommendationDecision) => ({
  diagnosis: "passed",
  deterministic_retrieval: "passed",
  final_task: "not_evaluated",
  invocation: "passed",
  plan: recommendationDecision === "blocked" ? "blocked" : "passed",
  recommendation_decision: recommendationDecision,
  setup: "passed",
});

const participant = (pseudonym, outcome, reasonCategory) => ({
  aggregate_inventory: {
    default_exposure: 36,
    placement_count: 120,
    session_coverage: "sampled_limited",
    skill_count: 64,
  },
  pseudonym,
  recommendations: [{
    outcome,
    reason_category: reasonCategory,
    recommendation_category: "on_demand_general",
    stage_results: stages(outcome),
  }],
  run_status: "observed",
  safety_outcome: safetyPass,
  supported_agents: ["codex"],
});

const ledger = {
  format: "skillroster-roster-recommendation-pilot",
  participants: [
    participant("synthetic-a", "accepted", "accepted_as_proposed"),
    participant("synthetic-b", "rejected", "personal_preference"),
    participant("synthetic-c", "blocked", "insufficient_evidence"),
  ],
  schema_version: 1,
  synthetic: true,
};

const script = fileURLToPath(new URL("../../../scripts/recommendation-pilot.mjs", import.meta.url));
const syntheticFixture = fileURLToPath(new URL("../../fixtures/roster-recommendation-pilot-v1.synthetic.json", import.meta.url));
const syntheticSummary = fileURLToPath(new URL("../../../docs/acceptance/artifacts/roster-recommendation-pilot-v1.synthetic-summary.json", import.meta.url));

test("synthetic pilot keeps recommendation, stage, and safety outcomes separate", () => {
  const summary = summarizePilot(ledger);

  assert.deepEqual(summary.participants, {
    abandoned_before_observation: 0,
    observed: 3,
    reported: 3,
    required: 3,
  });
  assert.deepEqual(summary.recommendation_outcomes, {
    accepted: 1,
    blocked: 1,
    not_evaluated: 0,
    rejected: 1,
  });
  assert.deepEqual(summary.reason_categories, {
    accepted_as_proposed: 1,
    insufficient_evidence: 1,
    personal_preference: 1,
  });
  assert.deepEqual(summary.stage_results.recommendation_decision, {
    accepted: 1,
    blocked: 1,
    not_evaluated: 0,
    rejected: 1,
  });
  assert.deepEqual(summary.safety, {
    authority_unverified_observed_count: 0,
    identifying_path_persisted_count: 0,
    passed: true,
    raw_conversation_persisted_count: 0,
    secret_persisted_count: 0,
    skill_content_persisted_count: 0,
    unapproved_write_count: 0,
  });
  assert.deepEqual(summary.product_change_authority, {
    embedding: false,
    model: false,
    policy: false,
    ranking: false,
    reason: "synthetic_evidence_cannot_authorize_product_change",
  });
  assert.equal(JSON.stringify(summary).includes("synthetic-a"), false);
});

test("pilot ledger rejects raw conversation fields before aggregation", () => {
  const unsafe = structuredClone(ledger);
  unsafe.participants[0].raw_conversation = "private participant message";

  assert.throws(
    () => summarizePilot(unsafe),
    (error) => error instanceof PilotError && error.code === "invalid_ledger",
  );
});

test("pilot ledger rejects identifying paths disguised as recommendation categories", () => {
  const unsafe = structuredClone(ledger);
  unsafe.participants[0].recommendations[0].recommendation_category = "/Users/alice/private-skill";

  assert.throws(
    () => summarizePilot(unsafe),
    (error) => error instanceof PilotError && error.code === "invalid_ledger",
  );
});

test("pilot ledger keeps retrieval failure independent from recommendation rejection", () => {
  const independent = structuredClone(ledger);
  const recommendation = independent.participants[1].recommendations[0];
  recommendation.stage_results.deterministic_retrieval = "failed";

  const summary = summarizePilot(independent);
  assert.equal(summary.stage_results.deterministic_retrieval.failed, 1);
  assert.equal(summary.stage_results.recommendation_decision.rejected, 1);
});

test("pilot stage ledger cannot skip an earlier failed setup", () => {
  const confused = structuredClone(ledger);
  const recommendation = confused.participants[0].recommendations[0];
  recommendation.stage_results.setup = "failed";

  assert.throws(
    () => summarizePilot(confused),
    (error) => error instanceof PilotError && error.code === "invalid_ledger",
  );
});

test("pilot outcome cannot use a reason category from another decision state", () => {
  const confused = structuredClone(ledger);
  confused.participants[1].recommendations[0].reason_category = "accepted_as_proposed";

  assert.throws(
    () => summarizePilot(confused),
    (error) => error instanceof PilotError && error.code === "invalid_ledger",
  );
});

test("pilot ledger records diagnosis and Plan readiness as separate stages", () => {
  const withReadiness = structuredClone(ledger);
  for (const participantEntry of withReadiness.participants) {
    for (const recommendation of participantEntry.recommendations) {
      recommendation.stage_results.diagnosis = "passed";
      recommendation.stage_results.plan = recommendation.outcome === "blocked" ? "blocked" : "passed";
    }
  }

  const summary = summarizePilot(withReadiness);
  assert.equal(summary.stage_results.diagnosis.passed, 3);
  assert.equal(summary.stage_results.plan.passed, 2);
  assert.equal(summary.stage_results.plan.blocked, 1);
});

test("pilot ledger keeps the qualitative participant set bounded", () => {
  const oversized = structuredClone(ledger);
  oversized.participants = Array.from({ length: 33 }, (_, index) => ({
    ...structuredClone(ledger.participants[0]),
    pseudonym: `synthetic-${String(index).padStart(2, "0")}`,
  }));

  assert.throws(
    () => summarizePilot(oversized),
    (error) => error instanceof PilotError && error.code === "invalid_ledger",
  );
});

test("pilot ledger reports a participant who left before observation without invented facts", () => {
  const withAbandonedRun = structuredClone(ledger);
  withAbandonedRun.participants.push({
    aggregate_inventory: null,
    pseudonym: "synthetic-d",
    recommendations: [],
    run_status: "abandoned_before_observation",
    safety_outcome: {
      ...safetyPass,
      authority_verified: false,
    },
    supported_agents: [],
  });

  const summary = summarizePilot(withAbandonedRun);
  assert.deepEqual(summary.participants, {
    abandoned_before_observation: 1,
    observed: 3,
    reported: 4,
    required: 3,
  });
  assert.deepEqual(summary.participant_readiness, {
    decision_ready: 3,
    diagnosed: 3,
    required_decision_ready: 2,
  });
  assert.equal(summary.safety.authority_unverified_observed_count, 0);
  assert.equal(summary.safety.passed, true);
  assert.match(renderPilotReport(summary), /Observed participants: 3; abandoned before observation: 1/u);
});

test("observed pilot runs cannot omit inventory or recommendations", () => {
  const incomplete = structuredClone(ledger);
  incomplete.participants[0].aggregate_inventory = null;
  incomplete.participants[0].recommendations = [];

  assert.throws(
    () => summarizePilot(incomplete),
    (error) => error instanceof PilotError && error.code === "invalid_ledger",
  );
});

test("synthetic pilot report states the evidence and product-change boundary", () => {
  const report = renderPilotReport(summarizePilot(ledger));

  assert.match(report, /Synthetic dry run/u);
  assert.match(report, /Participants reported: 3\/3/u);
  assert.match(report, /Decision-ready participants: 3\/2 required/u);
  assert.match(report, /Accepted: 1; rejected: 1; blocked: 1; not evaluated: 0/u);
  assert.match(report, /Safety gate: passed/u);
  assert.match(report, /does not authorize ranking, embedding, model, or policy changes/u);
  assert.equal(report.includes("synthetic-a"), false);
});

test("pilot CLI validates one ledger and emits bounded JSON or Markdown", () => {
  const root = mkdtempSync(join(resolve(tmpdir()), "skillroster-pilot-"));
  const input = join(root, "synthetic.json");
  writeFileSync(input, JSON.stringify(ledger));

  const summary = spawnSync(process.execPath, [script, "summarize", "--input", input], { encoding: "utf8" });
  assert.equal(summary.status, 0, summary.stderr);
  assert.equal(JSON.parse(summary.stdout).participants.reported, 3);
  assert.equal(summary.stdout.includes("synthetic-a"), false);

  const report = spawnSync(process.execPath, [script, "report", "--input", input], { encoding: "utf8" });
  assert.equal(report.status, 0, report.stderr);
  assert.match(report.stdout, /Synthetic dry run/u);
  assert.equal(report.stdout.includes("synthetic-a"), false);

  rmSync(root, { recursive: true, force: true });
});

test("pilot CLI refuses an oversized ledger before JSON parsing", () => {
  const root = mkdtempSync(join(resolve(tmpdir()), "skillroster-pilot-large-"));
  const input = join(root, "oversized.json");
  writeFileSync(input, `{"padding":"${"x".repeat(1024 * 1024)}"}`);

  const result = spawnSync(process.execPath, [script, "summarize", "--input", input], { encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /^input_too_large:/u);

  rmSync(root, { recursive: true, force: true });
});

test("frozen synthetic dry run keeps invocation, retrieval, disagreement, and blocker facts independent", () => {
  const fixture = JSON.parse(readFileSync(syntheticFixture, "utf8"));
  const summary = summarizePilot(fixture);

  assert.deepEqual(summary.recommendation_outcomes, {
    accepted: 1,
    blocked: 1,
    not_evaluated: 2,
    rejected: 1,
  });
  assert.equal(summary.stage_results.invocation.failed, 1);
  assert.equal(summary.stage_results.deterministic_retrieval.failed, 1);
  assert.equal(summary.stage_results.recommendation_decision.rejected, 1);
  assert.equal(summary.stage_results.final_task.passed, 2);
  assert.deepEqual(summary.participant_readiness, {
    decision_ready: 3,
    diagnosed: 3,
    required_decision_ready: 2,
  });
  assert.equal(summary.safety.passed, true);
  assert.equal(summary.product_change_authority.ranking, false);
  assert.equal(summary.product_change_authority.policy, false);
});

test("checked-in synthetic summary is reproduced exactly from the frozen fixture", () => {
  const fixture = JSON.parse(readFileSync(syntheticFixture, "utf8"));
  const recorded = JSON.parse(readFileSync(syntheticSummary, "utf8"));

  assert.deepEqual(recorded, summarizePilot(fixture));
});
