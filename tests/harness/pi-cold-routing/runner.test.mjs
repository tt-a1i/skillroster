import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, truncateSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import test from "node:test";

import { acceptanceBoundary, aggregateExitCode, aggregateSuite, applyPairInvalidation, assertNoBoundPathDrift, assessCommandReceipt, assessTranscriptCompletion, assessWorkspaceChanges, buildArmSchedule, classifyExecutionFailure, classifyPiTermination, cleanupPrivateConfig, commandUsage, copyPiConfig, deriveOutcomes, effectiveTaskTimeout, evaluateOracle, evaluateTopologyContract, freezeSuite, gateEventIntegrity, gateEventsBinding, gitBoundTreeDigest, invalidateOnCoreFailure, modulePathFromUrl, oracleEvidenceRecord, pairInvariantDigest, parseArgs, parseGateEvents, piProcessFacts, planWorkspaceInputs, promptContainsForbiddenTerm, sealPayloadDigest, snapshotPiConfig, summarizePolicyDenials, validateManifest, validateTimeoutOverride, verifyFirstSealBlob, verifySealContract, writeWorkspaceInputs } from "./runner.mjs";

const taskIds = ["a", "b", "c", "d"];
const digest = (value) => createHash("sha256").update(value).digest("hex");
const trainingGate = { core_task_success_minimum: 4, on_demand_task_success_minimum: 3, on_demand_load_success_minimum: 3 };
const aggregate = (results, complete = true, gate = trainingGate) => aggregateSuite(results, taskIds, complete, gate);
const result = (task, arm, overrides = {}) => ({
  task, arm, evaluation_status: "evaluated", execution_outcome: "task_succeeded",
  protocol_outcome: arm === "core" ? "core_control" : "retrieval_loaded", safety_outcome: "passed", accepted: true, ...overrides,
});

test("task mismatch is a protocol failure even when the task succeeds without loading the selected Skill", () => {
  const outcomes = deriveOutcomes("on_demand", [{ kind: "retrieval_failed", failure_type: "task_mismatch" }], true, 0, []);
  assert.equal(outcomes.execution_outcome, "task_succeeded");
  assert.equal(outcomes.protocol_outcome, "retrieval_wrong");
  assert.equal(outcomes.deepest_stage, "retrieval_wrong");
  assert.equal(outcomes.accepted, false);
  assert.equal(outcomes.task_succeeded_without_loaded_skill, true);
  const core = deriveOutcomes("core", [], true, 0, []);
  assert.equal(core.task_succeeded_without_loaded_skill, false);
  const coreWithExtraneousFindFailure = deriveOutcomes("core", [{ kind: "retrieval_failed", failure_type: "task_mismatch", contract_violation: true }], true, 0, []);
  assert.equal(coreWithExtraneousFindFailure.contract_violation, false); assert.equal(coreWithExtraneousFindFailure.accepted, true);
});

test("deepest stage uses only the Issue 129 exclusive vocabulary", () => {
  const allowed = new Set(["no_retrieval_call", "retrieval_wrong", "load_wrong", "task_execution_failed", "task_succeeded"]);
  const outcomes = [
    deriveOutcomes("on_demand", [], false, 0, []),
    deriveOutcomes("on_demand", [{ kind: "retrieval_failed", failure_type: "wrong_result" }], false, 0, []),
    deriveOutcomes("on_demand", [{ kind: "retrieval_succeeded", task_mismatch: false }], false, 0, []),
    deriveOutcomes("on_demand", [{ kind: "retrieval_succeeded", task_mismatch: false }, { kind: "target_skill_loaded" }], true, 0, []),
    deriveOutcomes("core", [], false, 0, []),
  ];
  assert(outcomes.every((outcome) => allowed.has(outcome.deepest_stage)));
  assert.deepEqual(outcomes.map((outcome) => outcome.deepest_stage), ["no_retrieval_call", "retrieval_wrong", "load_wrong", "task_succeeded", "task_execution_failed"]);
});

test("on-demand pre-route workspace reads and commands are contract violations, not safety failures", () => {
  const events = [
    { kind: "file_tool_blocked", failure_type: "route_order", classification: "protocol_denial", contract_violation: true },
    { kind: "command_blocked", failure_type: "route_order", classification: "protocol_denial", contract_violation: true },
  ];
  const outcome = deriveOutcomes("on_demand", events, true, 0, []);
  assert.equal(outcome.contract_violation, true); assert.equal(outcome.safety_outcome, "passed"); assert.equal(outcome.accepted, false);
});

test("retry uses deepest stage but an earlier task mismatch remains a contract violation", () => {
  const recovered = deriveOutcomes("on_demand", [{ kind: "retrieval_failed", failure_type: "wrong_result" }, { kind: "retrieval_succeeded", task_mismatch: false }, { kind: "target_skill_loaded" }], true, 0, []);
  assert.equal(recovered.protocol_outcome, "retrieval_loaded"); assert.equal(recovered.deepest_stage, "task_succeeded"); assert.equal(recovered.contract_violation, false); assert.equal(recovered.accepted, true);
  const mismatch = deriveOutcomes("on_demand", [{ kind: "retrieval_failed", failure_type: "task_mismatch" }, { kind: "retrieval_succeeded", task_mismatch: false }, { kind: "target_skill_loaded" }], true, 0, []);
  assert.equal(mismatch.protocol_outcome, "retrieval_loaded"); assert.equal(mismatch.contract_violation, true); assert.equal(mismatch.accepted, false);
  const tooManyAttempts = deriveOutcomes("on_demand", [{ kind: "retrieval_failed", failure_type: "wrong_result" }, { kind: "retrieval_succeeded", task_mismatch: false }, { kind: "retrieval_succeeded", task_mismatch: false }, { kind: "target_skill_loaded" }], true, 0, []);
  assert.equal(tooManyAttempts.retrieval_attempt_count, 3); assert.equal(tooManyAttempts.contract_violation, true); assert.equal(tooManyAttempts.accepted, false);
});

test("workspace assessment separates input mutation and extra output", () => {
  const before = new Map([["input.txt", "a"]]);
  const after = new Map([["input.txt", "b"], ["allowed.md", "x"], ["extra.md", "y"]]);
  assert.deepEqual(assessWorkspaceChanges(before, after, ["input.txt"], ["allowed.md"]), {
    changed: ["allowed.md", "extra.md", "input.txt"], input_mutations: ["input.txt"], unexpected_changes: ["extra.md", "input.txt"], special_outputs: [],
  });
  after.set("allowed.md", "special:symlink");
  assert.deepEqual(assessWorkspaceChanges(before, after, ["input.txt"], ["allowed.md"]).special_outputs, ["allowed.md"]);

  const exact = assessWorkspaceChanges(new Map(), new Map([["scratch/order-architecture.spec.json", "ok"], ["scratch/extra.json", "no"]]), [], ["scratch/order-architecture.spec.json", "outputs/order-architecture.html"]);
  assert.deepEqual(exact.unexpected_changes, ["scratch/extra.json"]);
});

test("contained denials remain auditable and nonfatal while actual extra files fail post-tree", () => {
  const event = { schema_version: 1, kind: "file_tool_blocked", classification: "policy_denial", failure_type: "output_path_denied", contained: true };
  assert.deepEqual(summarizePolicyDenials([event]), { policy_outcome: "denied", contained_denial_count: 1, contained_denials: [event], policy_denials: [event] });
  assert.deepEqual(summarizePolicyDenials([]), { policy_outcome: "clean", contained_denial_count: 0, contained_denials: [], policy_denials: [] });
  const outcome = deriveOutcomes("core", [event], true, 0, []);
  assert.equal(outcome.execution_outcome, "task_succeeded"); assert.equal(outcome.safety_outcome, "passed"); assert.equal(outcome.accepted, true);
  const changes = assessWorkspaceChanges(new Map(), new Map([["outputs/unexpected.md", "bytes"]]), [], ["outputs/expected.md"]);
  assert.deepEqual(changes.unexpected_changes, ["outputs/unexpected.md"]);
  assert.equal(deriveOutcomes("core", [event], true, 0, changes.unexpected_changes.map((path) => `unauthorized_change:${path}`)).accepted, false);
});

test("training permits one on-demand failure but stops on typed thresholds", () => {
  const threeOfFour = taskIds.flatMap((task, index) => [result(task, "core"), result(task, "on_demand", index === 3 ? { execution_outcome: "task_failed", accepted: false } : {})]);
  assert.equal(aggregate(threeOfFour).status, "passed");
  const coreFailed = threeOfFour.map((entry) => entry.task === "a" && entry.arm === "core" ? { ...entry, execution_outcome: "task_failed", accepted: false } : entry);
  assert.match(aggregate(coreFailed).stop_reason, /^core_failed:/u);
  const loadWrong = [result("a", "core"), result("a", "on_demand", { protocol_outcome: "load_wrong", accepted: false })];
  assert.equal(aggregate(loadWrong).stop_reason, "load_wrong:a");
  const unsafe = [result("a", "core", { safety_outcome: "failed", accepted: false })];
  assert.match(aggregate(unsafe).stop_reason, /^safety_failed:/u);
  assert.equal(aggregate(loadWrong, false).stop_reason, "load_wrong:a");

  const oneNoRetrieval = [result("a", "core"), result("a", "on_demand", { protocol_outcome: "no_retrieval_call", accepted: false })];
  assert.equal(aggregate(oneNoRetrieval).stop_reason, "suite_incomplete");
  const twoNoRetrieval = [...oneNoRetrieval, result("b", "core"), result("b", "on_demand", { protocol_outcome: "no_retrieval_call", accepted: false })];
  assert.equal(aggregate(twoNoRetrieval).stop_reason, "no_retrieval_call_threshold");
  const twoWrong = [result("a", "on_demand", { protocol_outcome: "retrieval_wrong", accepted: false }), result("b", "on_demand", { protocol_outcome: "retrieval_wrong", accepted: false })];
  assert.equal(aggregate(twoWrong).stop_reason, "retrieval_wrong_threshold");
  const duplicateWrongTask = [result("a", "on_demand", { protocol_outcome: "retrieval_wrong", accepted: false }), result("a", "on_demand", { contract_violation: true, accepted: false })];
  assert.equal(aggregate(duplicateWrongTask).stop_reason, "suite_incomplete");
  const twoContractTasks = [result("a", "on_demand", { contract_violation: true, accepted: false }), result("b", "on_demand", { contract_violation: true, accepted: false })];
  assert.equal(aggregate(twoContractTasks).stop_reason, "on_demand_load_threshold");
  const twoPostLoadFailures = [result("a", "on_demand", { execution_outcome: "task_failed", accepted: false }), result("b", "on_demand", { execution_outcome: "task_failed", accepted: false })];
  assert.equal(aggregate(twoPostLoadFailures).stop_reason, "on_demand_task_success_threshold");

  const crossSetFalseGreen = taskIds.flatMap((task, index) => [
    result(task, "core"),
    index < 2 ? result(task, "on_demand") : index === 2
      ? result(task, "on_demand", { protocol_outcome: "retrieval_wrong", accepted: false })
      : result(task, "on_demand", { execution_outcome: "task_failed", accepted: false }),
  ]);
  assert.equal(aggregate(crossSetFalseGreen).status, "failed");
});

test("training gate failure exits 2 while partial and passing runs exit 0", () => {
  assert.equal(aggregateExitCode({ status: "failed" }), 2);
  assert.equal(aggregateExitCode({ status: "passed" }), 0);
  assert.equal(aggregateExitCode({ status: "in_progress" }), 0);
  assert.equal(aggregateExitCode({ status: "not_evaluated" }), 0);
  assert.equal(aggregateExitCode({ status: "not_evaluated" }, [result("a", "on_demand", { accepted: false })]), 2);
  assert.equal(aggregateExitCode({ status: "passed" }, [result("d", "on_demand", { accepted: false })]), 0);
});

test("Pi timeout, signal, and nonzero exit remain typed execution results", () => {
  const timeout = { error: { code: "ETIMEDOUT" }, signal: "SIGTERM", status: null };
  assert.deepEqual(classifyPiTermination(timeout), { kind: "timeout", signal: "SIGTERM", exit_code: null });
  assert.deepEqual(piProcessFacts({ ...timeout, termination: classifyPiTermination(timeout) }), { pi_exit_code: null, pi_termination: { kind: "timeout", signal: "SIGTERM", exit_code: null } });
  assert.equal(deriveOutcomes("core", [], false, null, []).execution_outcome, "execution_failed");
  assert.deepEqual(classifyPiTermination({ signal: "SIGKILL", status: null }), { kind: "signal", signal: "SIGKILL", exit_code: null });
  assert.deepEqual(classifyPiTermination({ signal: null, status: 7 }), { kind: "exit_nonzero", signal: null, exit_code: 7 });
  assert.deepEqual(classifyPiTermination({ signal: null, status: 0 }), { kind: "completed", signal: null, exit_code: 0 });
  const wrappedTimeout = { kind: "timeout", signal: null, exit_code: null };
  assert.equal(classifyExecutionFailure(143, wrappedTimeout, { status: "failed", failure_type: "assistant_completion_missing" }), "wall_timeout");
  const timedOutOutcome = deriveOutcomes("core", [], false, 143, [], { status: "failed", failure_type: "assistant_completion_missing" }, wrappedTimeout);
  assert.equal(timedOutOutcome.execution_failure_type, "wall_timeout"); assert.equal(timedOutOutcome.execution_outcome, "execution_failed"); assert.equal(timedOutOutcome.deepest_stage, "task_execution_failed");
  assert.equal(classifyExecutionFailure(143, { kind: "signal", signal: "SIGTERM", exit_code: null }, { status: "completed" }), "pi_signal_termination");
});

test("provider transport errors with Pi exit zero are typed execution failures, not oracle task failures", () => {
  const transcript = [
    { type: "message_end", message: { role: "assistant", stopReason: "error", errorMessage: "WebSocket fetch failed" } },
    { type: "agent_settled" },
  ].map(JSON.stringify).join("\n");
  const assessed = assessTranscriptCompletion(transcript);
  assert.equal(assessed.failure_type, "provider_transport_failure");
  const outcome = deriveOutcomes("core", [], false, 0, [], assessed);
  assert.equal(outcome.execution_outcome, "execution_failed"); assert.equal(outcome.execution_failure_type, "provider_transport_failure"); assert.equal(outcome.accepted, false);
  const oracle = oracleEvidenceRecord({ passed: false, failures: ["missing:output"] }, outcome.execution_outcome);
  assert.equal(oracle.evaluation_status, "not_evaluated"); assert.equal(oracle.passed, null); assert.equal(oracle.observed.failures.length, 1);
  const outcomeAggregate = aggregate([result("a", "core", { execution_outcome: "execution_failed", execution_failure_type: "provider_transport_failure", accepted: false })]);
  assert.equal(outcomeAggregate.stop_reason, "execution_failed:a:core:provider_transport_failure");
  const normal = assessTranscriptCompletion(JSON.stringify({ type: "message_end", message: { role: "assistant", stopReason: "stop", content: [] } }));
  assert.equal(deriveOutcomes("core", [], false, 0, [], normal).execution_outcome, "task_failed");
  assert.equal(assessTranscriptCompletion("not-json").failure_type, "transcript_invalid");
});

test("command help projects fixed structured argument shapes", () => {
  const usage = commandUsage({ name: "validate", arguments: [{ kind: "enum", values: ["architecture", "workflow"] }, { kind: "read_path" }, { kind: "literal", value: "--json" }] });
  assert.equal(usage, "harness_command name=validate args=[<architecture|workflow>, <READ_PATH>, --json]");
});

test("pilot acceptance boundary exposes visual review as explicitly not evaluated", () => {
  const boundary = acceptanceBoundary({ required_successful_commands: ["validate", "deliver"] });
  assert.deepEqual(boundary, { required_successful_commands: ["validate", "deliver"], visual_review: "not_evaluated" });
});

test("a Core failure invalidates an earlier randomized on-demand result", () => {
  const onDemand = result("a", "on_demand");
  const core = result("a", "core", { execution_outcome: "task_failed", accepted: false });
  const results = invalidateOnCoreFailure([onDemand, core], core);
  assert.equal(results[0].evaluation_status, "not_evaluated");
  assert.equal(results[0].invalidation_reason, "core_control_failed");
  const invalidatedAggregate = aggregate(results);
  assert.deepEqual(invalidatedAggregate.invalidated_tasks, ["a"]);
});

test("pair invariant excludes arm and remains bound to frozen facts", () => {
  const base = { manifest_sha256: "m", bootstrap_sha256: "b", cli_sha256: "c", gate_sha256: "g", runner_sha256: "r" };
  const task = { id: "a", prompt: "same" }; const frozen = { packageDigest: "p", workspaceDigest: "w" };
  assert.deepEqual(pairInvariantDigest(base, "suite", task, frozen), pairInvariantDigest(base, "suite", task, frozen));
});

test("suite snapshot stays stable after live package inputs change", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-freeze-"));
  const skillsRoot = join(root, "skills"); const skill = join(skillsRoot, "sample");
  mkdirSync(skill, { recursive: true }); writeFileSync(join(skill, "SKILL.md"), "original");
  const bootstrap = join(root, "bootstrap.md"); const cli = join(root, "skillroster");
  writeFileSync(bootstrap, "bootstrap"); writeFileSync(cli, "binary");
  const task = { id: "task", expected_skill: "sample", prompt: "prompt", workspace_files: { "input.txt": "input" }, post_load_permissions: {}, oracle: {} };
  const manifest = { schema_version: 1, harness: "pi", suite_id: "test-fixture-freeze", model: "test/model", aggregate_gate: { core_task_success_minimum: 1, on_demand_task_success_minimum: 1, on_demand_load_success_minimum: 1 }, common: { tools: ["read"], forbidden_prompt_terms: [] }, tasks: [task] };
  const bytes = Buffer.from(JSON.stringify(manifest));
  const frozen = freezeSuite({ runsDir: join(root, "runs"), skillsRoot, bootstrap, cli, cliSourceRevision: "a".repeat(40), armSchedule: buildArmSchedule([task], "both", true, "0".repeat(32)), piIdentity: { executable_sha256: "pi", version_sha256: "version" }, piConfigSnapshot: { modelMappingDigest: "models" } }, manifest, bytes, [task]);
  const before = pairInvariantDigest(frozen.base, frozen.suiteSnapshotSha, task, frozen.tasks.get("task"));
  writeFileSync(join(skill, "SKILL.md"), "changed live source");
  const after = pairInvariantDigest(frozen.base, frozen.suiteSnapshotSha, task, frozen.tasks.get("task"));
  assert.deepEqual(after, before);
});

test("manifest rejects path traversal and unsafe Skill identity", () => {
  const manifest = { schema_version: 1, harness: "pi", suite_id: "test-fixture-paths", aggregate_gate: { core_task_success_minimum: 1, on_demand_task_success_minimum: 1, on_demand_load_success_minimum: 1 }, common: { tools: ["read", "write", "bash"], forbidden_prompt_terms: [] }, tasks: [{ id: "task", expected_skill: "skill", prompt: "p", workspace_files: { "../escape": "x" }, allowed_changed_paths: [], oracle: {} }] };
  assert.throws(() => validateManifest(manifest), /unsafe/u);
  manifest.tasks[0].workspace_files = {}; manifest.tasks[0].expected_skill = "../skill";
  assert.throws(() => validateManifest(manifest), /expected_skill/u);
  manifest.tasks[0].expected_skill = "skill"; manifest.tasks[0].contained_write_roots = ["../outputs"];
  assert.throws(() => validateManifest(manifest), /contained write root/u);
});

test("only the session task declares a safe contained output root", () => {
  const manifest = validateManifest(JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-training.json", "utf8")));
  assert.equal(manifest.suite_id, "cold-routing-training-v10");
  assert.deepEqual(manifest.tasks.find((task) => task.id === "train-session-mining-001").contained_write_roots, ["outputs"]);
  assert(manifest.tasks.filter((task) => task.id !== "train-session-mining-001").every((task) => task.contained_write_roots === undefined));
});

test("sealed holdout manifest validates, stays domain-distinct, and freezes stricter gates", () => {
  const training = validateManifest(JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-training.json", "utf8")));
  const holdout = validateManifest(JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-holdout.json", "utf8")));
  assert.equal(holdout.suite_id, "cold-routing-holdout-v2"); assert.equal(holdout.model, "seal/gpt-5.6-sol"); assert.equal(holdout.tasks.length, 4);
  const seal = JSON.parse(readFileSync(join("tests/fixtures", holdout.seal_contract), "utf8"));
  assert.equal(seal.seal_state, "frozen_before_first_run"); assert.equal(seal.facts.suite_id, holdout.suite_id);
  assert.match(seal.source_revision, /^[a-f0-9]{40}$/u); assert.match(seal.seal_sha256, /^[a-f0-9]{64}$/u);
  assert.deepEqual(holdout.aggregate_gate, { core_task_success_minimum: 4, on_demand_task_success_minimum: 4, on_demand_load_success_minimum: 3 });
  assert.deepEqual(training.aggregate_gate, trainingGate);
  assert.deepEqual(new Set(holdout.tasks.map((task) => task.family)), new Set(training.tasks.map((task) => task.family)));
  assert.deepEqual(new Set(holdout.tasks.map((task) => task.expected_skill)), new Set(training.tasks.map((task) => task.expected_skill)));
  for (const task of holdout.tasks) assert.equal(promptContainsForbiddenTerm(task.prompt, [...holdout.common.forbidden_prompt_terms, task.expected_skill]), null);
  assert(holdout.tasks.every((task) => !training.tasks.some((candidate) => digest(candidate.prompt) === digest(task.prompt))));
  assert(holdout.tasks.every((task) => !training.tasks.some((candidate) => JSON.stringify(Object.keys(candidate.workspace_files)) === JSON.stringify(Object.keys(task.workspace_files)))));
  assert(holdout.tasks.every((task) => !training.tasks.some((candidate) => JSON.stringify(candidate.allowed_changed_paths) === JSON.stringify(task.allowed_changed_paths))));
  assert.notDeepEqual(holdout.tasks.find((task) => task.family === "local_session_content_mining").contained_write_roots, training.tasks.find((task) => task.family === "local_session_content_mining").contained_write_roots);
  const lowered = structuredClone(holdout); lowered.aggregate_gate.on_demand_task_success_minimum = 3;
  assert.throws(() => validateManifest(lowered), /frozen suite policy/u);
  const renamedSeal = structuredClone(holdout); renamedSeal.seal_contract = "replacement.seal.json";
  assert.throws(() => validateManifest(renamedSeal), /frozen suite policy/u);
  const reseeded = structuredClone(holdout); reseeded.arm_schedule_seed = "0".repeat(32);
  assert.throws(() => validateManifest(reseeded), /frozen suite policy/u);
  const trainingWithSeal = structuredClone(training); trainingWithSeal.seal_contract = "training.seal.json";
  assert.throws(() => validateManifest(trainingWithSeal), /frozen suite policy/u);
  const inconsistentEdges = structuredClone(holdout); inconsistentEdges.tasks[0].oracle.topology_contract.connection_count += 1;
  assert.throws(() => validateManifest(inconsistentEdges), /connection_count is inconsistent/u);
  const reordered = structuredClone(holdout); reordered.aggregate_gate = { on_demand_load_success_minimum: 3, core_task_success_minimum: 4, on_demand_task_success_minimum: 4 };
  assert.doesNotThrow(() => validateManifest(reordered));
  const leaked = structuredClone(holdout); leaked.tasks[0].prompt += ` ${leaked.tasks[0].expected_skill}`;
  assert.throws(() => validateManifest(leaked), /prompt identity/u);
  const retired = structuredClone(holdout); retired.suite_id = "cold-routing-holdout-v1";
  assert.throws(() => validateManifest(retired), /unknown suite_id/u);
});

test("aggregate gates are manifest-driven for complete training and holdout selections", () => {
  const holdoutGate = { core_task_success_minimum: 4, on_demand_task_success_minimum: 4, on_demand_load_success_minimum: 3 };
  const threeLoadedPlusUnaided = taskIds.flatMap((task, index) => [
    result(task, "core"),
    index < 3 ? result(task, "on_demand") : result(task, "on_demand", { protocol_outcome: "no_retrieval_call", accepted: false }),
  ]);
  assert.equal(aggregate(threeLoadedPlusUnaided, true, holdoutGate).status, "passed");
  assert.equal(aggregate(threeLoadedPlusUnaided, true, trainingGate).status, "passed");
  const holdoutTaskFailure = threeLoadedPlusUnaided.map((entry) => entry.task === "d" && entry.arm === "on_demand" ? { ...entry, execution_outcome: "task_failed" } : entry);
  assert.equal(aggregate(holdoutTaskFailure, true, holdoutGate).stop_reason, "on_demand_task_success_threshold");
  const onlyTwoLoads = threeLoadedPlusUnaided.map((entry) => entry.task === "c" && entry.arm === "on_demand" ? { ...entry, protocol_outcome: "retrieval_wrong", accepted: false } : entry);
  assert.equal(aggregate(onlyTwoLoads, true, holdoutGate).stop_reason, "on_demand_load_threshold");
  assert.equal(aggregate(threeLoadedPlusUnaided.slice(0, 6), false, holdoutGate).status, "not_evaluated");
  const invalid = JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-holdout.json", "utf8")); invalid.aggregate_gate.on_demand_task_success_minimum = 5;
  assert.throws(() => validateManifest(invalid), /aggregate_gate/u);
});

test("HTML oracle positive control is a structured offline artifact, not token padding", () => {
  const manifest = JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-holdout.json", "utf8")); const task = manifest.tasks.find((candidate) => candidate.family === "interactive_architecture_diagram"); const root = mkdtempSync(join(tmpdir(), "skillroster-html-structure-")); mkdirSync(join(root, "deliverables"));
  const semanticLabels = task.oracle.required_substrings.filter((value) => !value.startsWith("<") && value !== "data-theme");
  const nodes = Array.from({ length: 120 }, (_, index) => `<g class="node" data-node="n${index}"><rect x="${(index % 12) * 90}" y="${Math.floor(index / 12) * 55}" width="80" height="40"/><text>${semanticLabels[index % semanticLabels.length]} ${index}</text></g>`).join("");
  const edges = Array.from({ length: 119 }, (_, index) => `<path class="edge" data-from="n${index}" data-to="n${index + 1}" d="M0 ${index} L100 ${index + 1}"/>`).join("");
  const html = `<!doctype html><html data-theme="light"><head><meta charset="utf-8"><style>body{font-family:system-ui}.node{cursor:pointer}.edge{stroke:#567;fill:none}.active{stroke:#f60}</style></head><body><button id="btn-theme">Theme</button><input id="component-locator"><button id="path-highlight">Trace</button><svg viewBox="0 0 1200 700">${nodes}${edges}</svg><script>document.querySelector('#btn-theme').onclick=()=>document.documentElement.dataset.theme=document.documentElement.dataset.theme==='light'?'dark':'light';document.querySelector('#component-locator').oninput=e=>document.querySelectorAll('.node').forEach(n=>n.hidden=!n.textContent.includes(e.target.value));document.querySelector('#path-highlight').onclick=()=>document.querySelectorAll('.edge').forEach(e=>e.classList.toggle('active'));</script></body></html>`;
  writeFileSync(join(root, task.oracle.path), html);
  assert.equal(evaluateOracle(task.oracle, root, new Set(["validate", "deliver"]), [task.oracle.path]).passed, true);
  const requiredFact = semanticLabels[0]; writeFileSync(join(root, task.oracle.path), html.replaceAll(requiredFact, "removed-fact"));
  assert.equal(evaluateOracle(task.oracle, root, new Set(["validate", "deliver"]), [task.oracle.path]).passed, false);
});

test("architecture topology contract rejects label stuffing and wrong graph shape", () => {
  const manifest = JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-holdout.json", "utf8")); const contract = manifest.tasks.find((task) => task.family === "interactive_architecture_diagram").oracle.topology_contract;
  const facts = {
    component_count: 8, boundary_count: 3, has_directed_cycle: false,
    components: ["Operations Desk", "Relay Hub", "Identity Authority", "Field Node East", "Field Node West", "Telemetry Vault", "Rule Engine", "Pager Channel"],
    boundaries: [{ label: "操作终端区", wraps: ["Operations Desk"] }, { label: "中央控制区", wraps: ["Relay Hub", "Identity Authority"] }, { label: "远端与遥测区", wraps: ["Field Node East", "Field Node West", "Telemetry Vault", "Rule Engine", "Pager Channel"] }],
    connections: [
      { from: "Operations Desk", to: "Relay Hub", label: "HTTPS" }, { from: "Relay Hub", to: "Identity Authority", label: "gRPC sync" },
      { from: "Relay Hub", to: "Field Node East", label: "mTLS" }, { from: "Relay Hub", to: "Field Node West", label: "mTLS" },
      { from: "Field Node East", to: "Telemetry Vault", label: "MQTT" }, { from: "Field Node West", to: "Telemetry Vault", label: "MQTT" }, { from: "Telemetry Vault", to: "Rule Engine", label: "异步事件" }, { from: "Rule Engine", to: "Pager Channel", label: "通知" },
    ],
  };
  assert.deepEqual(evaluateTopologyContract(facts, contract), []);
  const linear = { ...facts, connections: facts.connections.filter((edge) => edge.from !== "Relay Hub" || edge.to === "Identity Authority") };
  assert(evaluateTopologyContract(linear, contract).some((failure) => failure.includes("hub_spoke")));
  assert(evaluateTopologyContract({ ...facts, has_directed_cycle: true }, contract).includes("topology:directed_cycle"));
  const overlapping = { ...facts, boundaries: [...facts.boundaries, { wraps: ["Relay Hub"] }], boundary_count: 3 };
  assert(evaluateTopologyContract(overlapping, contract).includes("topology:boundary_partition"));
  const wrongZone = { ...facts, boundaries: facts.boundaries.map((boundary, index) => index === 1 ? { ...boundary, label: "错误区域" } : boundary) };
  assert(evaluateTopologyContract(wrongZone, contract).some((failure) => failure.includes("topology:boundary:中央控制区")));
  const hallucinated = { ...facts, connections: [...facts.connections, { from: "Pager Channel", to: "Operations Desk", label: "invented" }] };
  assert(evaluateTopologyContract(hallucinated, contract).some((failure) => failure.includes("topology:unlisted_edge")));
  const parallel = { ...facts, connections: [...facts.connections, { from: "Relay Hub", to: "Field Node East", label: "invented parallel edge" }] };
  const parallelFailures = evaluateTopologyContract(parallel, contract); assert(parallelFailures.includes("topology:connection_count")); assert(!parallelFailures.some((failure) => failure.includes("topology:unlisted_edge")));
  const stuffed = { ...facts, connections: [{ from: "Operations Desk", to: "Pager Channel", label: "Relay Hub Identity Authority mTLS MQTT async" }] };
  assert(evaluateTopologyContract(stuffed, contract).length > 3);
});

test("glossary oracle checks every deprecated term without Markdown formatting dependence", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-deprecated-")); const path = "terms.txt";
  const oracle = { type: "markdown_glossary", path, required_substrings: ["正式概念"], deprecated_terms: ["旧词甲", "旧词乙"] };
  writeFileSync(join(root, path), "正式概念：定义。旧词甲属于旧称，已经停用。\n请不要使用旧词乙。");
  assert.equal(evaluateOracle(oracle, root, new Set(), [path]).passed, true);
  writeFileSync(join(root, path), `正式概念：定义。旧词甲属于旧称，已经停用。\n旧词乙仍在文中。${"普通说明".repeat(20)}`);
  const missing = evaluateOracle(oracle, root, new Set(), [path]); assert.equal(missing.passed, false); assert(missing.failures.includes(`${path}:deprecated_relation:旧词乙`));
});

test("task timeout is frozen per task with a 300000 default", () => {
  assert.equal(effectiveTaskTimeout({ timeout_ms: 600000 }, 300000), 600000);
  assert.equal(effectiveTaskTimeout({}, 300000), 300000);
  const task = { id: "task", expected_skill: "skill", prompt: "p", timeout_ms: 600000, workspace_files: {}, allowed_changed_paths: [], oracle: {} };
  const manifest = { schema_version: 1, harness: "pi", suite_id: "test-fixture-timeout", aggregate_gate: { core_task_success_minimum: 1, on_demand_task_success_minimum: 1, on_demand_load_success_minimum: 1 }, common: { tools: ["read"], forbidden_prompt_terms: [] }, tasks: [task] };
  assert.doesNotThrow(() => validateManifest(manifest));
  task.timeout_ms = 999; assert.throws(() => validateManifest(manifest), /timeout_ms/u);
  task.timeout_ms = 900001; assert.throws(() => validateManifest(manifest), /timeout_ms/u);
});

test("workspace inputs are fully planned before destination mutation", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-workspace-plan-")); const absent = join(root, "absent");
  const task = (workspaceFiles) => ({ id: "workspace-task", workspace_files: workspaceFiles });
  assert.throws(() => writeWorkspaceInputs(task({ "a": "file", "a/b.txt": "child" }), absent), /prefix conflict/u);
  assert.equal(existsSync(absent), false);

  const empty = join(root, "empty"); mkdirSync(empty);
  assert.throws(() => writeWorkspaceInputs(task({ "same/path.txt": "one", "same\\path.txt": "two" }), empty), /collide/u);
  assert.deepEqual(readdirSync(empty), []);
  assert.throws(() => planWorkspaceInputs(task({ [`${"d/".repeat(32)}file.txt`]: "deep" })), /depth/u);

  const tooMany = Object.fromEntries(Array.from({ length: 10_001 }, (_, index) => [`files/${index}.txt`, "x"]));
  assert.throws(() => planWorkspaceInputs(task(tooMany)), /file count/u);
  const tooManyMaterializedEntries = Object.fromEntries(Array.from({ length: 5_001 }, (_, index) => [`directory-${index}/file.txt`, "x"]));
  assert.throws(() => planWorkspaceInputs(task(tooManyMaterializedEntries)), /materialized entry count/u);
  const oversized = "x".repeat(64 * 1024 * 1024 + 1);
  assert.throws(() => planWorkspaceInputs(task({ "large.txt": oversized })), /workspace file exceeds/u);
  const fortyMiB = "x".repeat(40 * 1024 * 1024);
  assert.throws(() => planWorkspaceInputs(task(Object.fromEntries(Array.from({ length: 7 }, (_, index) => [`${index}.txt`, fortyMiB])))), /workspace total exceeds/u);
});

test("official formal suites reject timeout overrides while bounded diagnostics are ineligible", () => {
  const formal = parseArgs(["--timeout-ms", "600000"]);
  assert.equal(formal.timeoutOverridden, true);
  assert.throws(() => validateTimeoutOverride(formal, { suite_id: "cold-routing-training-v10" }), /formal suites forbid/u);
  const diagnostic = parseArgs(["--diagnostic", "--timeout-ms", "900000"]);
  assert.deepEqual(validateTimeoutOverride(diagnostic, { suite_id: "cold-routing-holdout-v2" }), { formal_eligible: false });
  assert.throws(() => parseArgs(["--diagnostic", "--timeout-ms", "900001"]), /between 1000 and 900000/u);
  const frozen = parseArgs([]);
  assert.equal(frozen.timeoutOverridden, false);
  assert.deepEqual(validateTimeoutOverride(frozen, { suite_id: "cold-routing-holdout-v2" }), { formal_eligible: true });
});

test("module file URLs decode spaces and reserved path characters portably", () => {
  const path = join(tmpdir(), "skillroster harness # percent %.mjs");
  assert.equal(modulePathFromUrl(pathToFileURL(path)), path);
});

test("oracle and gate-ledger full reads fail closed at the shared single-file cap", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-full-read-cap-"));
  const output = join(root, "output.html"); writeFileSync(output, ""); truncateSync(output, 65 * 1024 * 1024);
  assert.throws(() => evaluateOracle({ type: "html", path: "output.html" }, root, new Set(), ["output.html"]), /bounded I\/O policy/u);
  const events = join(root, "events.jsonl"); writeFileSync(events, ""); truncateSync(events, 65 * 1024 * 1024);
  assert.throws(() => gateEventsBinding(events), /bounded I\/O policy/u);
});

test("redaction oracle checks only public seeds for public facts and all changed files for leaks", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-oracle-")); mkdirSync(join(root, "outputs"));
  writeFileSync(join(root, "outputs/public.md"), "Windows CRLF");
  writeFileSync(join(root, "outputs/private.md"), "客户 不公开 <REDACTED_SECRET>");
  writeFileSync(join(root, "outputs/report.md"), "路径 邮箱 URL 凭据");
  const oracle = {
    type: "redaction_bundle", required_paths: ["outputs/public.md", "outputs/private.md", "outputs/report.md"],
    private_path: "outputs/private.md", report_path: "outputs/report.md",
    required_nonempty_paths: ["outputs/public.md"], public_seed_paths: ["outputs/public.md"], public_required_regex: ["Windows", "CRLF"],
    private_required_substrings: ["客户", "不公开"], report_required_substrings: ["路径", "邮箱", "URL", "凭据"], forbidden_across_outputs: ["<REDACTED_SECRET>"],
  };
  const evaluated = evaluateOracle(oracle, root, new Set(), ["outputs/public.md", "outputs/private.md", "outputs/report.md"]);
  assert.equal(evaluated.passed, false); assert(evaluated.failures.some((failure) => failure.includes("changed_outputs")));
});

test("session oracle accepts equivalent privacy wording and rejects missing privacy categories", () => {
  const manifest = JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-training.json", "utf8"));
  const task = manifest.tasks.find((item) => item.id === "train-session-mining-001"); const oracle = task.oracle;
  const root = mkdtempSync(join(tmpdir(), "skillroster-session-equivalent-")); mkdirSync(join(root, "outputs"));
  const writeBundle = (privateText, reportText) => {
    writeFileSync(join(root, "outputs/x_reply_seeds.md"), "Windows notes");
    writeFileSync(join(root, "outputs/x_thread_seeds.md"), "CRLF handling");
    writeFileSync(join(root, "outputs/private_do_not_post.md"), privateText);
    writeFileSync(join(root, "outputs/redaction_report.md"), reportText);
  };
  writeBundle("客户材料禁止发布", "路径、联系信息、URL、凭据均已处理");
  assert.equal(evaluateOracle(oracle, root, new Set(), task.allowed_changed_paths).passed, true);
  writeBundle("客户材料仅供内部参考", "路径、联系信息、URL、凭据均已处理");
  const missingPrivate = evaluateOracle(oracle, root, new Set(), task.allowed_changed_paths);
  assert.equal(missingPrivate.passed, false); assert(missingPrivate.failures.some((failure) => failure.includes("不公开|不可公开|不得公开|禁止发布")));
  writeBundle("客户材料不得公开", "路径、URL、凭据均已处理");
  const missingContact = evaluateOracle(oracle, root, new Set(), task.allowed_changed_paths);
  assert.equal(missingContact.passed, false); assert(missingContact.failures.some((failure) => failure.includes("邮箱|邮件|联系方式|联系信息")));
  const wrongName = assessWorkspaceChanges(new Map(), new Map([["outputs/private.md", "x"]]), [], task.allowed_changed_paths);
  assert.deepEqual(wrongName.unexpected_changes, ["outputs/private.md"]);
});

test("required nonempty outputs reject whitespace-only content", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-nonempty-")); writeFileSync(join(root, "output.md"), " \n\t");
  const result = evaluateOracle({ type: "text", path: "output.md", required_nonempty_paths: ["output.md"] }, root, new Set(), ["output.md"]);
  assert.equal(result.passed, false); assert(result.failures.includes("empty:output.md"));
});

test("domain oracle accepts plain formatting and requires a retirement relation for every old term", () => {
  const manifest = JSON.parse(readFileSync("tests/fixtures/pi-cold-routing-training.json", "utf8"));
  const oracle = manifest.tasks.find((task) => task.id === "train-domain-language-001").oracle;
  const root = mkdtempSync(join(tmpdir(), "skillroster-domain-oracle-"));
  const facts = ["空间：客户组织的数据隔离边界。租户是旧称，账户不再使用。", "成员：获得访问权限的人。不要使用用户。", "邀请：尚未被接受的加入空间请求。待激活成员应避免使用。"].join("\n");
  writeFileSync(join(root, "CONTEXT.md"), facts);
  assert.equal(evaluateOracle(oracle, root, new Set(), ["CONTEXT.md"]).passed, true);
  writeFileSync(join(root, "CONTEXT.md"), facts.replace("加入空间请求", "申请加入空间"));
  const rejected = evaluateOracle(oracle, root, new Set(), ["CONTEXT.md"]);
  assert.equal(rejected.passed, false); assert(rejected.failures.some((failure) => failure.includes("加入.{0,4}请求")));
  writeFileSync(join(root, "CONTEXT.md"), facts.replace("不要使用用户", "用户也表示成员"));
  const missingRelation = evaluateOracle(oracle, root, new Set(), ["CONTEXT.md"]);
  assert.equal(missingRelation.passed, false); assert(missingRelation.failures.includes("CONTEXT.md:deprecated_relation:用户"));
});

test("command receipt binds validation, delivery, canonical artifact path, and final bytes", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-command-receipt-")); mkdirSync(join(root, "outputs"));
  const artifact = join(root, "outputs/final.html"); writeFileSync(artifact, "<!doctype html><svg></svg>");
  const artifactSha = digest(readFileSync(artifact)); const canonicalPathSha = digest(realpathSync(artifact)); const validation = digest("validation");
  const events = [
    { kind: "command", name: "validate", exit_code: 0, source_sha256: "source", receipt_chain_sha256: validation },
    { kind: "command", name: "deliver", exit_code: 0, source_sha256: "source", validation_receipt_sha256: validation, artifact_path_sha256: canonicalPathSha, artifact_sha256: artifactSha, receipt_chain_sha256: digest("delivery") },
  ];
  const oracle = { path: "outputs/final.html", required_successful_commands: ["validate", "deliver"] };
  assert.equal(assessCommandReceipt(events, oracle, root).status, "passed");
  writeFileSync(artifact, "overwritten after delivery");
  assert(assessCommandReceipt(events, oracle, root).failures.includes("command_receipt:artifact_digest_drift"));
  assert(assessCommandReceipt(events.slice(1), oracle, root).failures.includes("command_receipt:validation_chain_missing"));
  const topologyOracle = { ...oracle, topology_contract: { component_count: 1, boundary_count: 1, forbid_directed_cycle: true, require_all_components_in_boundaries: true, require_partitioned_boundaries: true, forbid_unlisted_edges: true, required_boundaries: [], required_edges: [], required_paths: [] } };
  const topologyReceipt = assessCommandReceipt(events, topologyOracle, root); assert.equal(topologyReceipt.status, "failed"); assert(topologyReceipt.topology_failures.includes("topology:validated_graph_missing"));
});

test("sealed schedule is deterministic and bound to both arms", () => {
  const tasks = [{ id: "a" }, { id: "b" }]; const seed = "1".repeat(32);
  const first = buildArmSchedule(tasks, "both", true, seed); const second = buildArmSchedule(tasks, "both", true, seed);
  assert.deepEqual(first, second);
  for (const order of Object.values(first.order)) assert.deepEqual(new Set(order), new Set(["core", "on_demand"]));
});

test("seal verification is exact, excludes private config facts, and rejects repository drift", () => {
  const facts = { suite_id: "cold-routing-holdout-v2", manifest_sha256: "a".repeat(64), public_model_profile_sha256: "b".repeat(64) };
  const sourceRevision = "c".repeat(40); const contract = { schema_version: 1, suite_id: facts.suite_id, seal_state: "frozen_before_first_run", source_revision: sourceRevision, facts, seal_sha256: sealPayloadDigest(sourceRevision, facts) };
  assert.equal(verifySealContract(contract, facts), true);
  const reorderedFacts = { public_model_profile_sha256: facts.public_model_profile_sha256, manifest_sha256: facts.manifest_sha256, suite_id: facts.suite_id };
  assert.equal(verifySealContract({ ...contract, facts: reorderedFacts }, facts), true);
  assert.throws(() => verifySealContract({ ...contract, facts: { ...facts, auth_sha256: "secret-derived" } }, facts), /digest|match/u);
  assert.equal(Object.keys(facts).some((key) => /auth|private|pi_config/iu.test(key)), false);
  assert.equal(assertNoBoundPathDrift(""), true);
  for (const status of [" M tests/harness/pi-cold-routing/runner.mjs\n", "M  tests/fixtures/pi-cold-routing-holdout.json\n", "?? tests/new.json\n"]) assert.throws(() => assertNoBoundPathDrift(status), /drift/u);
});

test("seal provenance binds a real source tree and the immutable first-add blob", () => {
  const repo = mkdtempSync(join(tmpdir(), "skillroster-seal-git-")); const runGit = (...args) => {
    const result = spawnSync("git", args, { cwd: repo, encoding: "utf8" }); assert.equal(result.status, 0, result.stderr); return result.stdout.trim();
  };
  runGit("init", "-q"); runGit("config", "user.email", "seal@example.invalid"); runGit("config", "user.name", "Seal Test");
  writeFileSync(join(repo, "bound.txt"), "implementation\n"); runGit("add", "bound.txt"); runGit("commit", "-qm", "implementation"); const sourceRevision = runGit("rev-parse", "HEAD");
  const boundPaths = [join(repo, "bound.txt")]; const facts = { suite_id: "cold-routing-holdout-v2", git_bound_tree_sha256: gitBoundTreeDigest(sourceRevision, boundPaths, repo) };
  const contractPath = join(repo, "seal.json"); const contract = { schema_version: 1, suite_id: facts.suite_id, seal_state: "frozen_before_first_run", source_revision: sourceRevision, facts, seal_sha256: sealPayloadDigest(sourceRevision, facts) };
  writeFileSync(contractPath, `${JSON.stringify(contract, null, 2)}\n`); runGit("add", "seal.json"); runGit("commit", "-qm", "first immutable seal");
  assert.equal(verifySealContract(contract, facts, { repo, contractPath, boundPaths }), true);
  assert.equal(verifyFirstSealBlob(contractPath, sourceRevision, repo).blob_sha256, digest(readFileSync(contractPath)));
  const fakeFacts = { ...facts, target_packages_sha256: { forged: "f".repeat(64) } }; const reSignedFacts = { ...contract, facts: fakeFacts, seal_sha256: sealPayloadDigest(sourceRevision, fakeFacts) };
  writeFileSync(contractPath, `${JSON.stringify(reSignedFacts, null, 2)}\n`);
  assert.throws(() => verifySealContract(reSignedFacts, fakeFacts, { repo, contractPath, boundPaths }), /first-add/u);
  writeFileSync(contractPath, `${JSON.stringify(contract, null, 2)}\n`);
  const wrongRevision = runGit("rev-parse", "HEAD"); const resigned = { ...contract, source_revision: wrongRevision, seal_sha256: sealPayloadDigest(wrongRevision, facts) };
  writeFileSync(contractPath, `${JSON.stringify(resigned, null, 2)}\n`);
  assert.throws(() => verifySealContract(resigned, facts, { repo, contractPath, boundPaths }), /first-add|added after|source tree/u);
  writeFileSync(contractPath, `${JSON.stringify(contract, null, 2)}\n`); writeFileSync(join(repo, "bound.txt"), "drift\n"); runGit("add", "bound.txt"); runGit("commit", "-qm", "bound tree drift");
  assert.throws(() => verifySealContract(contract, facts, { repo, contractPath, boundPaths }), /source tree/u);
  assert.throws(() => gitBoundTreeDigest("f".repeat(40), boundPaths, repo), /does not exist/u);
});

test("pair invalidation is order-independent", () => {
  const core = result("a", "core", { execution_outcome: "task_failed", accepted: false }); const od = result("a", "on_demand");
  for (const ordered of [[core, { ...od }], [{ ...od }, core]]) {
    applyPairInvalidation(ordered); const candidate = ordered.find((entry) => entry.arm === "on_demand");
    assert.equal(candidate.evaluation_status, "not_evaluated"); assert.equal(candidate.invalidation_reason, "core_control_failed");
  }
});

test("official partial runs require explicit diagnostic mode", () => {
  const partial = { status: "not_evaluated" }; const accepted = [result("a", "core")];
  assert.equal(aggregateExitCode(partial, accepted, { official: true, diagnostic: false, complete: false }), 2);
  assert.equal(aggregateExitCode(partial, accepted, { official: true, diagnostic: true, complete: false }), 0);
});

test("Pi config is copied from one in-memory suite snapshot without persisting its fingerprint", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-config-")); const source = join(root, "source"); const destination = join(root, "arm"); mkdirSync(source);
  writeFileSync(join(source, "auth.json"), "first-secret"); writeFileSync(join(source, "models.json"), '{"model":"one"}');
  const snapshot = snapshotPiConfig(source, "seal/model-one"); writeFileSync(join(source, "auth.json"), "rotated-secret");
  const rotated = snapshotPiConfig(source, "seal/model-one"); copyPiConfig(snapshot, destination);
  assert.equal(readFileSync(join(destination, "auth.json"), "utf8"), "first-secret");
  assert.equal(typeof snapshot.modelMappingDigest, "string");
  assert.equal(rotated.modelMappingDigest, snapshot.modelMappingDigest);
  assert.notEqual(rotated.privateFingerprint, snapshot.privateFingerprint);
});

test("forbidden prompt terms are case-insensitive", () => {
  assert.equal(promptContainsForbiddenTerm("Use SKILLROSTER quietly", ["SkillRoster"]), "SkillRoster");
  assert.equal(promptContainsForbiddenTerm("ordinary task", ["SkillRoster"]), null);
});

test("gate event evidence binds full bytes or an explicit missing sentinel", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-gate-binding-")); const path = join(root, "events.jsonl");
  const missing = gateEventsBinding(path); assert.equal(missing.source, "missing_sentinel"); assert.equal(missing.sha256.length, 64);
  writeFileSync(path, '{"kind":"command"}\n{"kind":"command_failed"}\n');
  const present = gateEventsBinding(path); assert.equal(present.source, "file"); assert.equal(present.sha256.length, 64); assert.notEqual(present.sha256, missing.sha256);
  truncateSync(path, 8 * 1024 * 1024 + 1);
  assert.throws(() => gateEventsBinding(path), /single file/u);
  assert.throws(() => parseGateEvents(path), /single file/u);
});

test("gate event integrity fails closed for missing, empty, duplicate, schema, arm, and malformed evidence", () => {
  const ready = { schema_version: 1, kind: "gate_ready", arm: "core" };
  assert.deepEqual(gateEventIntegrity([ready], [], "core", "file"), []);
  assert(gateEventIntegrity([], [], "core", "missing").some((item) => item.includes("gate_events_missing")));
  assert(gateEventIntegrity([], [], "core", "file").some((item) => item.includes("gate_events_empty")));
  assert(gateEventIntegrity([ready, ready], [], "core", "file").some((item) => item.includes("gate_ready_duplicate")));
  assert(gateEventIntegrity([{ ...ready, schema_version: 2 }], [], "core", "file").some((item) => item.includes("wrong_schema")));
  assert(gateEventIntegrity([{ ...ready, arm: "on_demand" }], [], "core", "file").some((item) => item.includes("wrong_arm")));
  assert(gateEventIntegrity([], ["gate_event_parse_error:1"], "core", "file").some((item) => item.includes("parse_error")));
  const coreOutcome = deriveOutcomes("core", [], true, 0, gateEventIntegrity([], [], "core", "missing"));
  assert.equal(coreOutcome.safety_outcome, "failed"); assert.equal(coreOutcome.accepted, false);
});

test("private config cleanup is scoped to the run root", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-cleanup-"));
  const config = join(root, "pi-config");
  mkdirSync(config); writeFileSync(join(config, "auth.json"), "secret");
  cleanupPrivateConfig(config, root);
  assert.equal(existsSync(config), false);
  assert.throws(() => cleanupPrivateConfig(root, root), /unsafe/u);
});
