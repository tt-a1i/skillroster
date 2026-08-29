const PARTICIPANT_REQUIREMENT = 3;
const PARTICIPANT_LIMIT = 32;
const RECOMMENDATION_LIMIT = 64;
const RECOMMENDATION_CATEGORIES = new Set([
  "core_agent_specific",
  "core_general",
  "on_demand_agent_specific",
  "on_demand_general",
  "protected_existing",
  "source_blocked",
]);
const SUPPORTED_AGENTS = new Set([
  "claude-code",
  "codex",
  "cursor",
  "gemini-cli",
  "github-copilot",
  "hermes",
  "opencode",
  "pi",
]);
const OUTCOMES = new Set(["accepted", "blocked", "not_evaluated", "rejected"]);
const REASONS_BY_OUTCOME = {
  accepted: new Set(["accepted_as_proposed"]),
  blocked: new Set(["incorrect_identity", "insufficient_evidence", "other_bounded", "unsuitable_source"]),
  not_evaluated: new Set(["not_evaluated"]),
  rejected: new Set(["incorrect_identity", "insufficient_evidence", "other_bounded", "personal_preference", "unsuitable_source"]),
};
const REASON_CATEGORIES = new Set(Object.values(REASONS_BY_OUTCOME).flatMap((reasons) => [...reasons]));
const STAGE_STATES = new Set(["blocked", "failed", "not_evaluated", "passed"]);
const STAGE_DEFINITIONS = {
  setup: { prerequisites: [], states: STAGE_STATES },
  invocation: { prerequisites: [["setup", ["passed"]]], states: STAGE_STATES },
  diagnosis: { prerequisites: [["invocation", ["passed"]]], states: STAGE_STATES },
  plan: { prerequisites: [["diagnosis", ["passed"]]], states: STAGE_STATES },
  deterministic_retrieval: { prerequisites: [["invocation", ["passed"]]], states: STAGE_STATES },
  recommendation_decision: {
    prerequisites: [["diagnosis", ["passed"]], ["plan", ["blocked", "passed"]]],
    states: OUTCOMES,
  },
  final_task: { prerequisites: [["invocation", ["passed"]]], states: STAGE_STATES },
};
const COVERAGE_STATES = new Set(["complete", "missing", "sampled_limited", "unavailable"]);
const RUN_STATUSES = new Set(["abandoned_before_observation", "observed"]);

export class PilotError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "PilotError";
    this.code = code;
  }
}

const fail = (message) => { throw new PilotError("invalid_ledger", message); };
const exactKeys = (value, expected, label) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${label} must be an object`);
  const actual = Object.keys(value).sort();
  if (actual.join("\0") !== [...expected].sort().join("\0")) fail(`${label} has an invalid shape`);
};
const nonNegativeInteger = (value, label) => {
  if (!Number.isSafeInteger(value) || value < 0) fail(`${label} must be a non-negative integer`);
};
const boundedEnum = (value, vocabulary, label) => {
  if (!vocabulary.has(value)) fail(`${label} is outside the bounded vocabulary`);
};

const validateLedger = (ledger) => {
  exactKeys(ledger, ["format", "participants", "schema_version", "synthetic"], "ledger");
  if (ledger.format !== "skillroster-roster-recommendation-pilot" || ledger.schema_version !== 1) {
    fail("ledger has an unsupported format");
  }
  if (typeof ledger.synthetic !== "boolean") fail("ledger synthetic marker must be boolean");
  if (
    !Array.isArray(ledger.participants)
    || ledger.participants.length < PARTICIPANT_REQUIREMENT
    || ledger.participants.length > PARTICIPANT_LIMIT
  ) {
    fail(`ledger requires ${PARTICIPANT_REQUIREMENT} to ${PARTICIPANT_LIMIT} participants`);
  }
  const pseudonyms = new Set();
  for (const participant of ledger.participants) {
    exactKeys(
      participant,
      ["aggregate_inventory", "pseudonym", "recommendations", "run_status", "safety_outcome", "supported_agents"],
      "participant",
    );
    if (typeof participant.pseudonym !== "string" || !/^[a-z0-9][a-z0-9-]{2,31}$/u.test(participant.pseudonym)) {
      fail("participant pseudonym must be a bounded opaque label");
    }
    if (pseudonyms.has(participant.pseudonym)) fail("participant pseudonyms must be unique");
    pseudonyms.add(participant.pseudonym);
    boundedEnum(participant.run_status, RUN_STATUSES, "participant run status");
    exactKeys(
      participant.safety_outcome,
      [
        "authority_verified",
        "identifying_path_persisted",
        "raw_conversation_persisted",
        "secret_persisted",
        "skill_content_persisted",
        "unapproved_write_count",
      ],
      "safety outcome",
    );
    for (const field of [
      "authority_verified",
      "identifying_path_persisted",
      "raw_conversation_persisted",
      "secret_persisted",
      "skill_content_persisted",
    ]) {
      if (typeof participant.safety_outcome[field] !== "boolean") fail(`safety ${field} must be boolean`);
    }
    nonNegativeInteger(participant.safety_outcome.unapproved_write_count, "unapproved write count");
    if (participant.run_status === "abandoned_before_observation") {
      if (
        participant.aggregate_inventory !== null
        || !Array.isArray(participant.supported_agents)
        || participant.supported_agents.length !== 0
        || !Array.isArray(participant.recommendations)
        || participant.recommendations.length !== 0
      ) {
        fail("an unobserved participant cannot contain invented observation facts");
      }
      continue;
    }
    if (!Array.isArray(participant.supported_agents) || participant.supported_agents.length === 0) {
      fail("participant supported agents must be a non-empty array");
    }
    if (new Set(participant.supported_agents).size !== participant.supported_agents.length) {
      fail("participant supported agents must be unique");
    }
    for (const agent of participant.supported_agents) boundedEnum(agent, SUPPORTED_AGENTS, "supported Agent");
    exactKeys(
      participant.aggregate_inventory,
      ["default_exposure", "placement_count", "session_coverage", "skill_count"],
      "aggregate inventory",
    );
    nonNegativeInteger(participant.aggregate_inventory.skill_count, "Skill count");
    nonNegativeInteger(participant.aggregate_inventory.placement_count, "Placement count");
    nonNegativeInteger(participant.aggregate_inventory.default_exposure, "default exposure");
    boundedEnum(participant.aggregate_inventory.session_coverage, COVERAGE_STATES, "session coverage");
    if (
      !Array.isArray(participant.recommendations)
      || participant.recommendations.length === 0
      || participant.recommendations.length > RECOMMENDATION_LIMIT
    ) {
      fail("participant recommendations must be a bounded non-empty array");
    }
    for (const recommendation of participant.recommendations) {
      exactKeys(
        recommendation,
        ["outcome", "reason_category", "recommendation_category", "stage_results"],
        "recommendation",
      );
      if (!RECOMMENDATION_CATEGORIES.has(recommendation.recommendation_category)) {
        fail("recommendation category is outside the bounded vocabulary");
      }
      boundedEnum(recommendation.outcome, OUTCOMES, "recommendation outcome");
      boundedEnum(recommendation.reason_category, REASON_CATEGORIES, "reason category");
      exactKeys(
        recommendation.stage_results,
        Object.keys(STAGE_DEFINITIONS),
        "stage results",
      );
      for (const [stage, definition] of Object.entries(STAGE_DEFINITIONS)) {
        boundedEnum(recommendation.stage_results[stage], definition.states, `${stage} result`);
        if (
          recommendation.stage_results[stage] !== "not_evaluated"
          && definition.prerequisites.some(([prerequisite, allowed]) => (
            !allowed.includes(recommendation.stage_results[prerequisite])
          ))
        ) {
          fail(`${stage} must remain unevaluated until its prerequisites pass or block as specified`);
        }
      }
      if (recommendation.outcome !== recommendation.stage_results.recommendation_decision) {
        fail("recommendation outcome must match its stage result");
      }
      if (
        recommendation.stage_results.plan === "blocked"
        && recommendation.stage_results.recommendation_decision !== "blocked"
      ) {
        fail("a blocked Plan requires a blocked recommendation outcome");
      }
      if (!REASONS_BY_OUTCOME[recommendation.outcome].has(recommendation.reason_category)) {
        fail("recommendation reason category does not match its outcome");
      }
    }
  }
  return ledger;
};

const countValues = (values, vocabulary) => Object.fromEntries(
  vocabulary.map((value) => [value, values.filter((entry) => entry === value).length]),
);

export const summarizePilot = (ledger) => {
  validateLedger(ledger);
  const recommendations = ledger.participants.flatMap((participant) => participant.recommendations);
  const safetyOutcomes = ledger.participants.map((participant) => participant.safety_outcome);
  const reasonCategories = recommendations.map((recommendation) => recommendation.reason_category);
  const observedParticipants = ledger.participants.filter((participant) => participant.run_status === "observed");

  return {
    format: "skillroster-roster-recommendation-pilot-summary",
    participants: {
      abandoned_before_observation: ledger.participants.length - observedParticipants.length,
      observed: observedParticipants.length,
      reported: ledger.participants.length,
      required: PARTICIPANT_REQUIREMENT,
    },
    participant_readiness: {
      decision_ready: ledger.participants.filter((participant) => participant.recommendations.some(
        (recommendation) => recommendation.stage_results.diagnosis === "passed"
          && ["blocked", "passed"].includes(recommendation.stage_results.plan),
      )).length,
      diagnosed: ledger.participants.filter((participant) => participant.recommendations.some(
        (recommendation) => recommendation.stage_results.diagnosis === "passed",
      )).length,
      required_decision_ready: 2,
    },
    product_change_authority: {
      embedding: false,
      model: false,
      policy: false,
      ranking: false,
      reason: ledger.synthetic
        ? "synthetic_evidence_cannot_authorize_product_change"
        : "pilot_evidence_requires_separate_product_decision",
    },
    reason_categories: Object.fromEntries(
      [...new Set(reasonCategories)].sort().map((reason) => [
        reason,
        reasonCategories.filter((entry) => entry === reason).length,
      ]),
    ),
    recommendation_outcomes: countValues(
      recommendations.map((recommendation) => recommendation.outcome),
      ["accepted", "blocked", "not_evaluated", "rejected"],
    ),
    safety: (() => {
      const result = {
        authority_unverified_observed_count: observedParticipants.filter(
          (participant) => !participant.safety_outcome.authority_verified,
        ).length,
        identifying_path_persisted_count: safetyOutcomes.filter((outcome) => outcome.identifying_path_persisted).length,
        raw_conversation_persisted_count: safetyOutcomes.filter((outcome) => outcome.raw_conversation_persisted).length,
        secret_persisted_count: safetyOutcomes.filter((outcome) => outcome.secret_persisted).length,
        skill_content_persisted_count: safetyOutcomes.filter((outcome) => outcome.skill_content_persisted).length,
        unapproved_write_count: safetyOutcomes.reduce((sum, outcome) => sum + outcome.unapproved_write_count, 0),
      };
      return {
        ...result,
        passed: Object.values(result).every((value) => value === 0),
      };
    })(),
    schema_version: 1,
    stage_results: Object.fromEntries(Object.entries(STAGE_DEFINITIONS).map(([stage, definition]) => [
      stage,
      countValues(
        recommendations.map((recommendation) => recommendation.stage_results[stage]),
        [...definition.states],
      ),
    ])),
    synthetic: ledger.synthetic,
  };
};

export const renderPilotReport = (summary) => {
  const outcomes = summary.recommendation_outcomes;
  return [
    "# Roster recommendation pilot",
    "",
    summary.synthetic ? "Status: Synthetic dry run" : "Status: Real pilot evidence",
    `Participants reported: ${summary.participants.reported}/${summary.participants.required}`,
    `Observed participants: ${summary.participants.observed}; abandoned before observation: ${summary.participants.abandoned_before_observation}`,
    `Decision-ready participants: ${summary.participant_readiness.decision_ready}/${summary.participant_readiness.required_decision_ready} required`,
    `Accepted: ${outcomes.accepted}; rejected: ${outcomes.rejected}; blocked: ${outcomes.blocked}; not evaluated: ${outcomes.not_evaluated}`,
    `Safety gate: ${summary.safety.passed ? "passed" : "failed"}`,
    "",
    "This evidence does not authorize ranking, embedding, model, or policy changes.",
    "A separate product decision must interpret completed real-pilot evidence.",
    "",
  ].join("\n");
};
