import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { assessCoreOrder, assessExactLoad, assessOneCallLoad, assessProtectedScopes, assessRouteOrder, assessSkillSurface, assessTranscriptIntegrity, assessWorkspaceChanges, captureProtectedScopes, classifyPair, deriveArmOutcome, deriveProtocolDecision, evaluateArchitectureSpec, evaluateArchifyReceipts, evaluateOracle, extractVisibleSkills, findWrapperSource, formalResultEligible, groupPairedResults, main, pairInvariant, parseArgs, parseFindAudit, parseFindEnvelope, setupArm, skillRosterFindArgs, skillRosterScanArgs, snapshotWorkspace, validateManifest, verifyArchifyParent } from "./driver.mjs";

const DRIVER = fileURLToPath(new URL("./driver.mjs", import.meta.url));

const digest = async (value) => {
  const { createHash } = await import("node:crypto"); return createHash("sha256").update(value).digest("hex");
};

test("on-demand runtime state and audit stay in the sandbox temp boundary", () => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), "codex-runtime-boundary-"));
  const repo = fileURLToPath(new URL("../../../", import.meta.url));
  const paths = setupArm(root, {
    expected_skill: "event-manifest",
    workspace_files: { "handoff.psv": "fixture\n" },
  }, "on_demand", {
    skillsRoot: join(repo, "tests/fixtures/codex-protocol-skills"),
    bootstrap: join(repo, "skill/skillroster/SKILL.md"),
  });
  try {
    assert.equal(paths.state, join(paths.temp, "state"));
    assert.equal(paths.runtimeAudit, join(paths.temp, "find-audit.jsonl"));
    assert.equal(paths.audit, join(root, "find-audit.jsonl"));
    assert.notEqual(paths.audit, join(paths.temp, "find-audit.jsonl"));
  } finally {
    rmSync(paths.temp, { recursive: true, force: true });
    rmSync(root, { recursive: true, force: true });
  }
});

test("default is a non-executing plan and execution requires explicit auth", () => {
  assert.equal(parseArgs([]).execute, false);
  assert.equal(parseArgs([]).reasoningEffort, "medium");
  assert.equal(parseArgs(["--reasoning-effort", "high"]).reasoningEffort, "high");
  assert.throws(() => parseArgs(["--reasoning-effort", "ultra"]), /reasoning-effort/u);
  assert.throws(() => parseArgs(["--execute"]), /explicit --auth-source/u);
  assert.equal(parseArgs(["--execute", "--auth-source", "/tmp/auth.json"]).execute, true);
  assert.equal(parseArgs(["--reevaluate-root", "/tmp/existing-runs"]).execute, false);
  assert.throws(() => parseArgs(["--execute", "--auth-source", "/tmp/auth.json", "--reevaluate-root", "/tmp/runs"]), /mutually exclusive/u);
  assert.throws(() => main(["--runs-dir", fileURLToPath(new URL("../../../tests/transcripts", import.meta.url))]), /outside the repository/u);
});

test("summary output preserves stdout JSON in a private external file", { skip: process.platform === "win32" }, () => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), "codex-summary-output-")); const runs = join(root, "runs"); const output = join(root, "summary.json");
  const result = spawnSync(process.execPath, [DRIVER, "--runs-dir", runs, "--model", "gpt-5.6-luna", "--summary-output", output], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr); assert.equal(existsSync(output), true);
  assert.deepEqual(JSON.parse(readFileSync(output, "utf8")), JSON.parse(result.stdout));
  assert.equal(statSync(output).mode & 0o777, 0o600);
});

test("summary output refuses existing, run-root, repository, and linked-parent targets", { skip: process.platform === "win32" }, () => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), "codex-summary-boundary-")); const runs = join(root, "runs");
  const invoke = (output) => spawnSync(process.execPath, [DRIVER, "--runs-dir", runs, "--summary-output", output], { encoding: "utf8" });
  const existing = join(root, "existing.json"); writeFileSync(existing, "keep");
  const existingResult = invoke(existing); assert.notEqual(existingResult.status, 0); assert.equal(readFileSync(existing, "utf8"), "keep");
  const runOutput = join(runs, "summary.json"); const runResult = invoke(runOutput); assert.notEqual(runResult.status, 0); assert.equal(existsSync(runOutput), false);
  const repositoryOutput = join(fileURLToPath(new URL("../../../", import.meta.url)), `.forbidden-summary-${process.pid}.json`);
  const repositoryResult = invoke(repositoryOutput); assert.notEqual(repositoryResult.status, 0); assert.equal(existsSync(repositoryOutput), false);
  const realParent = join(root, "real-parent"); const linkedParent = join(root, "linked-parent"); mkdirSync(realParent); symlinkSync(realParent, linkedParent, "dir");
  const linkedOutput = join(linkedParent, "summary.json"); const linkedResult = invoke(linkedOutput); assert.notEqual(linkedResult.status, 0); assert.equal(existsSync(linkedOutput), false);
  const nestedParent = join(realParent, "nested"); mkdirSync(nestedParent);
  const nestedLinkedOutput = join(linkedParent, "nested", "summary.json"); const nestedLinkedResult = invoke(nestedLinkedOutput); assert.notEqual(nestedLinkedResult.status, 0); assert.equal(existsSync(nestedLinkedOutput), false);
});

test("summary persistence failure keeps the stdout JSON contract and fails the command", { skip: process.platform === "win32" || process.getuid?.() === 0 }, () => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), "codex-summary-failure-")); const runs = join(root, "runs"); const outputParent = join(root, "read-only"); mkdirSync(outputParent); chmodSync(outputParent, 0o500);
  try {
    const result = spawnSync(process.execPath, [DRIVER, "--runs-dir", runs, "--model", "gpt-5.6-luna", "--summary-output", join(outputParent, "summary.json")], { encoding: "utf8" });
    assert.notEqual(result.status, 0); assert.equal(JSON.parse(result.stdout).status, "planned"); assert.match(result.stderr, /summary output persistence failed/u);
  } finally { chmodSync(outputParent, 0o700); }
});

test("runs directory cannot hide a repository target behind a symlink", { skip: process.platform === "win32" }, () => {
  const root = mkdtempSync(join(tmpdir(), "codex-runs-link-")); const alias = join(root, "runs"); const repo = new URL("../../../", import.meta.url).pathname;
  symlinkSync(repo, alias, "dir"); assert.throws(() => main(["--runs-dir", alias]), /resolves inside the repository/u);
});

test("offline reevaluation writes only outside the immutable source run tree", () => {
  const parent = mkdtempSync(join(tmpdir(), "codex-reevaluate-")); const source = join(parent, "runs"); mkdirSync(source); const fixture = JSON.parse(readFileSync(new URL("../../fixtures/codex-cold-routing-transfer.json", import.meta.url)));
  assert.throws(() => main(["--reevaluate-root", source, "--reevaluate-output", join(source, "summary.json")]), /outside the source run root/u);
  for (const task of fixture.tasks) for (const arm of ["core", "on_demand"]) { const root = join(source, `${task.id}-${arm}-fixture`); mkdirSync(join(root, "workspace"), { recursive: true }); for (const [path, value] of Object.entries(task.workspace_files)) writeFileSync(join(root, "workspace", path), value); writeFileSync(join(root, "codex-events.jsonl"), ""); }
  const before = snapshotWorkspace(source); const output = join(parent, "post-hoc.json"); assert.equal(main(["--reevaluate-root", source, "--reevaluate-output", output]), 0); assert.deepEqual(snapshotWorkspace(source), before); const summary = JSON.parse(readFileSync(output)); assert.equal(summary.raw_runs_modified, false); assert.equal(summary.formal_gate_eligible, false); assert.equal(summary.source_tree_sha256_before, summary.source_tree_sha256_after); assert.ok(summary.results.every((result) => result.formal_evidence_accepted === null && result.post_hoc_only === true));
});

test("manifest supports bounded one-or-more-task Codex protocol suites", () => {
  const manifest = { schema_version: 1, harness: "codex-transfer", tasks: [
    { id: "a", family: "family-a", expected_skill: "one", prompt: "p", hint: "h", workspace_files: { "in.txt": "x" }, allowed_changed_paths: ["out.md"], oracle: { path: "out.md" } },
    { id: "b", family: "family-b", expected_skill: "two", prompt: "p", hint: "h", workspace_files: {}, allowed_changed_paths: ["out.md"], oracle: { path: "out.md" } },
  ] };
  assert.equal(validateManifest(manifest), manifest);
  assert.throws(() => validateManifest({ ...manifest, harness: "pi" }), /unsupported/u);
  assert.equal(validateManifest({ ...manifest, tasks: [manifest.tasks[0]], trials_per_arm: 3 }).tasks.length, 1);
  assert.throws(() => validateManifest({ ...manifest, tasks: [], trials_per_arm: 3 }), /unsupported/u);
  assert.throws(() => validateManifest({ ...manifest, trials_per_arm: 0 }), /trials_per_arm/u);
  assert.throws(() => validateManifest({ ...manifest, formal_protocol_gate: "yes" }), /formal_protocol_gate/u);
  assert.throws(() => validateManifest({ ...manifest, tasks: [{ ...manifest.tasks[0], family: "family-b" }, manifest.tasks[1]] }), /family must be unique/u);
  assert.throws(() => validateManifest({ ...manifest, tasks: [{ ...manifest.tasks[0], family: "Family A" }, manifest.tasks[1]] }), /family must be unique/u);
  for (const prompt of ["use SkillRoster", "run capability search", "please Find it", "做能力检索", "load one"]) {
    assert.throws(() => validateManifest({ ...manifest, tasks: [{ ...manifest.tasks[0], prompt }] }), /must not disclose/u);
  }
});

test("multi-family protocol aggregation counts every task pair and emits one gate per family", () => {
  const tasks = [{ id: "a", family: "rewrite" }, { id: "b", family: "extract" }, { id: "c", family: "artifact" }];
  const results = tasks.flatMap((task) => [
    { family: task.family, task: task.id, trial: 1, arm: "core", pair_invariant: `${task.id}-pair`, outcome: { accepted: true, harness_valid: true, task: "succeeded", load: "loaded", safety: "passed" } },
    { family: task.family, task: task.id, trial: 1, arm: "on_demand", pair_invariant: `${task.id}-pair`, outcome: { accepted: true, harness_valid: true, task: "succeeded", load: "loaded", safety: "passed", retrieval: "retrieved", contract_violation: false } },
  ]);
  const pairs = groupPairedResults(tasks, results, 1);
  assert.deepEqual(pairs.map((pair) => [pair.family, pair.gate]), [["rewrite", "passed"], ["extract", "passed"], ["artifact", "passed"]]);
  const decision = deriveProtocolDecision(results, 1, tasks, pairs);
  assert.equal(decision.expected_runs, 6);
  assert.equal(decision.core_accepted, 3);
  assert.equal(decision.on_demand_accepted, 3);
  assert.equal(decision.overall_gate, true);
  assert.deepEqual(decision.family_gates.map((gate) => gate.family), ["rewrite", "extract", "artifact"]);
  assert.ok(decision.family_gates.every((gate) => gate.gate === "passed"));
});

test("a failed family gate cannot be hidden by passing families", () => {
  const tasks = [{ id: "rewrite", family: "rewrite" }, { id: "extract", family: "extract" }];
  const result = (task, family, arm, accepted) => ({ family, task, trial: 1, arm, pair_invariant: `${task}-pair`, outcome: { accepted, harness_valid: true, task: accepted ? "succeeded" : "failed", load: accepted ? "loaded" : "load_wrong", safety: "passed", retrieval: accepted ? "retrieved" : "retrieval_wrong", contract_violation: !accepted } });
  const results = [result("rewrite", "rewrite", "core", true), result("rewrite", "rewrite", "on_demand", true), result("extract", "extract", "core", true), result("extract", "extract", "on_demand", false)];
  const pairs = groupPairedResults(tasks, results, 1);
  const decision = deriveProtocolDecision(results, 1, tasks, pairs);
  assert.equal(decision.overall_gate, false);
  assert.equal(decision.family_gates.find((gate) => gate.family === "rewrite").gate, "passed");
  assert.equal(decision.family_gates.find((gate) => gate.family === "extract").gate, "failed");
});

test("formal eligibility requires a complete successful harness record, not a task pass", () => {
  const eligible = { pair_invariant: "frozen", codex_exit_code: 0, surface: { passed: true }, transcript: { passed: true }, workspace: { passed: true }, protected_scopes: { passed: true }, outcome: { harness_valid: true, accepted: false } };
  assert.equal(formalResultEligible(eligible), true);
  for (const mutation of [
    { codex_exit_code: 124 }, { transcript: { passed: false } }, { surface: { passed: false } },
    { workspace: { passed: false } }, { protected_scopes: { passed: false } }, { outcome: { harness_valid: false } },
  ]) assert.equal(formalResultEligible({ ...eligible, ...mutation }), false);
});

test("pair invariant is frozen from the suite snapshot and complete task input", () => {
  const task = { id: "transfer-a", prompt: "fixed", workspace_files: { "input.txt": "one" } };
  const frozen = pairInvariant("suite-snapshot", task);
  assert.equal(pairInvariant("suite-snapshot", task), frozen);
  assert.notEqual(pairInvariant("other-snapshot", task), frozen);
  assert.notEqual(pairInvariant("suite-snapshot", { ...task, prompt: "changed" }), frozen);
  assert.notEqual(pairInvariant("suite-snapshot", task, 2), frozen);
});

test("protocol decision applies the frozen stop conditions", () => {
  const result = (arm, accepted, overrides = {}) => ({ arm, outcome: { accepted, load: "loaded", contract_violation: false, ...overrides } });
  assert.equal(deriveProtocolDecision([result("core", true), result("core", false), result("core", true)], 3).decision, "fix_control_task_or_oracle");
  const core = Array.from({ length: 3 }, () => result("core", true));
  const contractFailures = [result("on_demand", false, { load: "load_wrong" }), result("on_demand", false, { contract_violation: true }), result("on_demand", true)];
  assert.equal(deriveProtocolDecision([...core, ...contractFailures], 3).decision, "fix_bootstrap_or_cli_contract");
  assert.equal(deriveProtocolDecision([...core, ...Array.from({ length: 3 }, () => result("on_demand", true))], 3).decision, "retain_current_design");
});

test("prompt-input preflight permits only fixed Codex system skills plus the arm skill", () => {
  const promptInput = (names) => JSON.stringify([{ type: "message", role: "developer", content: [{ type: "input_text", text: `<skills_instructions>\n### Available skills\n${names.map((name) => `- ${name}: description (file: /tmp/${name}/SKILL.md)`).join("\n")}\n</skills_instructions>` }, { type: "input_text", text: "## Examples\n- Pipes: |\n- Todoist: unrelated" }] }]);
  const system = ["imagegen", "openai-docs", "plugin-creator", "skill-creator", "skill-installer"];
  assert.deepEqual(extractVisibleSkills(promptInput([...system, "humanizer-zh"])), [...system, "humanizer-zh"].sort());
  assert.equal(assessSkillSurface(extractVisibleSkills(promptInput([...system, "humanizer-zh"])), "core", "humanizer-zh").passed, true);
  assert.equal(assessSkillSurface(extractVisibleSkills(promptInput([...system, "skillroster"])), "on_demand", "humanizer-zh").passed, true);
  assert.deepEqual(extractVisibleSkills(promptInput([...system, "vendor:skill"])).includes("vendor:skill"), true);
  const polluted = assessSkillSurface(extractVisibleSkills(promptInput([...system, "skillroster", "other"])), "on_demand", "humanizer-zh");
  assert.equal(polluted.passed, false); assert.deepEqual(polluted.unexpected, ["other"]);
});

test("SkillRoster seam uses the real envelope and global-option ordering", () => {
  const envelope = JSON.stringify({ schema_version: 1, ok: true, command: "find", result: { ranking_strategy: "task_hint_reciprocal_rank_fusion", matches: [{ name: "humanizer-zh", skill_id: "skill_hash", paths: ["/tmp/source/humanizer-zh/SKILL.md"], roster_state: "on_demand", agents: [] }] } });
  const parsed = parseFindEnvelope(envelope); assert.equal(parsed.skill, "humanizer-zh"); assert.equal(parsed.path, "/tmp/source/humanizer-zh/SKILL.md"); assert.equal(parsed.roster_state, "on_demand");
  assert.throws(() => parseFindEnvelope(JSON.stringify({ schema_version: 1, ok: true, command: "find", result: { ranking_strategy: "single_lexical_channel", matches: [] } })), /reciprocal_rank_fusion/u);
  const paths = { home: "/tmp/home", state: "/tmp/state" }; const task = { prompt: "完整任务", hint: "agent hint" };
  assert.deepEqual(skillRosterScanArgs(paths, "/tmp/source"), ["--home", "/tmp/home", "--state-dir", "/tmp/state", "--json", "scan", "--source-root", "/tmp/source"]);
  assert.deepEqual(skillRosterFindArgs(paths, task), ["--home", "/tmp/home", "--state-dir", "/tmp/state", "--json", "find", "完整任务", "--hint", "agent hint"]);
  assert.deepEqual(skillRosterFindArgs(paths, { ...task, one_call_load: true }), ["--home", "/tmp/home", "--state-dir", "/tmp/state", "--json", "find", "完整任务", "--hint", "agent hint", "--load", "--limit", "1"]);
});

test("Find audit preserves exact argument mismatch and retry classification", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-find-audit-")); const target = join(root, "source", "humanizer-zh", "SKILL.md");
  mkdirSync(join(root, "source", "humanizer-zh"), { recursive: true }); writeFileSync(target, "skill");
  const expected = { task: "完整任务", skill: "humanizer-zh", path: target };
  const good = { kind: "find_call", argv_shape_valid: true, envelope_valid: true, task_sha256: await digest(expected.task), hint_count: 1, hint_nonempty: true, hint_sha256: [await digest("hint")], exit_code: 0, top1_skill: expected.skill, top1_path_sha256: await digest(realpathSync(target)) };
  const clean = parseFindAudit(`${JSON.stringify(good)}\n`, expected);
  assert.equal(clean.first_call_task_complete, true); assert.equal(clean.returned_path_exact, true); assert.equal(clean.retry_classification, "single_attempt"); assert.equal(clean.contract_violation, false);
  const bad = { ...good, task_sha256: await digest(""), hint_count: 0, hint_nonempty: false, hint_sha256: [], exit_code: 2, top1_skill: null, top1_path_sha256: null };
  const recovered = parseFindAudit(`${JSON.stringify(bad)}\n${JSON.stringify(good)}\n`, expected);
  assert.equal(recovered.top1_correct, true); assert.equal(recovered.retry_classification, "recovered_after_argument_mismatch"); assert.equal(recovered.contract_violation, true);
  const invalidEnvelope = parseFindAudit(`${JSON.stringify({ ...good, envelope_valid: false })}\n`, expected);
  assert.equal(invalidEnvelope.top1_correct, false); assert.equal(invalidEnvelope.contract_violation, true);
});

test("Find audit canonicalizes symlinked path spellings", { skip: process.platform === "win32" }, async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-find-path-")); const real = join(root, "real"); const alias = join(root, "alias");
  mkdirSync(real); symlinkSync(real, alias, "dir"); const target = join(real, "SKILL.md"); writeFileSync(target, "skill");
  const expected = { task: "任务", skill: "sample", path: join(alias, "SKILL.md") };
  const call = { kind: "find_call", argv_shape_valid: true, envelope_valid: true, task_sha256: await digest(expected.task), hint_count: 1, hint_nonempty: true, hint_sha256: [await digest("hint")], exit_code: 0, top1_skill: expected.skill, top1_path_sha256: await digest(realpathSync(target)) };
  assert.equal(parseFindAudit(`${JSON.stringify(call)}\n`, expected).returned_path_exact, true);
});

test("wrapper source is allowlisted and records hashes rather than raw task or hint", () => {
  const source = findWrapperSource();
  assert.match(source, /permits only/u); assert.match(source, /task_sha256/u); assert.match(source, /hint_sha256/u);
  assert.doesNotMatch(source, /task_raw|hint_raw/u);
  const root = mkdtempSync(join(tmpdir(), "codex-find-wrapper-")); const path = join(root, "skillroster.mjs"); writeFileSync(path, source);
  const checked = spawnSync(process.execPath, ["--check", path], { encoding: "utf8" });
  assert.equal(checked.status, 0, checked.stderr);
  writeFileSync(path, findWrapperSource(true));
  const oneCallChecked = spawnSync(process.execPath, ["--check", path], { encoding: "utf8" });
  assert.equal(oneCallChecked.status, 0, oneCallChecked.stderr);
});

test("post-hoc workspace audit treats every sidecar outside exact allowlist as a safety failure", () => {
  const before = new Map([["input.txt", "a"]]);
  const after = new Map([["input.txt", "a"], ["outputs/result.md", "b"], ["outputs/visual-check.json", "c"]]);
  const assessed = assessWorkspaceChanges(before, after, ["input.txt"], ["outputs/result.md"]);
  assert.equal(assessed.passed, false); assert.deepEqual(assessed.unexpected_changes, ["outputs/visual-check.json"]);
});

test("workspace snapshot never follows a symlink even when its path is allowlisted", { skip: process.platform === "win32" }, () => {
  const root = mkdtempSync(join(tmpdir(), "codex-workspace-link-")); const outside = mkdtempSync(join(tmpdir(), "codex-outside-"));
  writeFileSync(join(outside, "secret.md"), "must not be read"); mkdirSync(join(root, "outputs")); symlinkSync(join(outside, "secret.md"), join(root, "outputs", "result.md"));
  const after = snapshotWorkspace(root); assert.equal(after.get("outputs/result.md"), "special:symlink");
  const assessed = assessWorkspaceChanges(new Map(), after, [], ["outputs/result.md"]);
  assert.equal(assessed.passed, false); assert.deepEqual(assessed.special_entries, [{ path: "outputs/result.md", kind: "symlink" }]);
});

test("oracle rejects a symlinked output and a symlinked ancestor", { skip: process.platform === "win32" }, () => {
  const root = mkdtempSync(join(tmpdir(), "codex-oracle-link-")); const outside = mkdtempSync(join(tmpdir(), "codex-oracle-outside-")); writeFileSync(join(outside, "out.md"), "hello 42");
  symlinkSync(join(outside, "out.md"), join(root, "direct.md"));
  assert.equal(evaluateOracle(root, { path: "direct.md", required_substrings: ["42"] }).passed, false);
  symlinkSync(outside, join(root, "outputs"), "dir");
  assert.equal(evaluateOracle(root, { path: "outputs/out.md", required_substrings: ["42"] }).passed, false);
});

test("protected target, exposed Skill, and auth scopes fail on content or identity drift", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-protected-scopes-")); const targetPackage = join(root, "target"); const exposedPackage = join(root, "exposed"); const authCopy = join(root, "auth.json");
  mkdirSync(targetPackage); mkdirSync(exposedPackage); writeFileSync(join(targetPackage, "SKILL.md"), "target"); writeFileSync(join(exposedPackage, "SKILL.md"), "bootstrap"); writeFileSync(authCopy, "auth");
  const paths = { targetPackage, exposedPackage, authCopy }; const before = captureProtectedScopes(paths);
  writeFileSync(join(targetPackage, "SKILL.md"), "tampered");
  const targetDrift = assessProtectedScopes(before, captureProtectedScopes(paths)); assert.equal(targetDrift.passed, false); assert.deepEqual(targetDrift.changed_scopes, ["target_package"]);
  writeFileSync(join(targetPackage, "SKILL.md"), "target"); const restored = captureProtectedScopes(paths); writeFileSync(join(exposedPackage, "SKILL.md"), "tampered-bootstrap");
  assert.deepEqual(assessProtectedScopes(restored, captureProtectedScopes(paths)).changed_scopes, ["exposed_package"]);
  writeFileSync(join(exposedPackage, "SKILL.md"), "bootstrap"); const authBaseline = captureProtectedScopes(paths); writeFileSync(authCopy, "changed-auth");
  assert.deepEqual(assessProtectedScopes(authBaseline, captureProtectedScopes(paths)).changed_scopes, ["auth_copy"]);
  const outcome = deriveArmOutcome({ arm: "core", surface: { passed: true }, retrieval: {}, load: { passed: true }, oracle: { passed: true }, workspace: { passed: true }, coreOrder: { passed: true }, transcript: { passed: true }, protectedScopes: { passed: false } });
  assert.equal(outcome.safety, "failed"); assert.equal(outcome.accepted, false);
});

test("earliest baseline catches preflight-stage workspace and protected-scope mutation", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-earliest-baseline-")); const workspace = join(root, "workspace"); const targetPackage = join(root, "target"); const exposedPackage = join(root, "exposed"); const authCopy = join(root, "auth.json");
  mkdirSync(workspace); mkdirSync(targetPackage); mkdirSync(exposedPackage); writeFileSync(join(workspace, "input.md"), "original"); writeFileSync(join(targetPackage, "SKILL.md"), "target"); writeFileSync(join(exposedPackage, "SKILL.md"), "bootstrap"); writeFileSync(authCopy, "auth");
  const initialWorkspace = snapshotWorkspace(workspace); const paths = { targetPackage, exposedPackage, authCopy }; const initialProtected = captureProtectedScopes(paths);
  writeFileSync(join(workspace, "preflight-sidecar.md"), "unexpected"); writeFileSync(join(exposedPackage, "SKILL.md"), "preflight-tamper");
  const workspaceResult = assessWorkspaceChanges(initialWorkspace, snapshotWorkspace(workspace), ["input.md"], []); const protectedResult = assessProtectedScopes(initialProtected, captureProtectedScopes(paths));
  assert.equal(workspaceResult.passed, false); assert.equal(protectedResult.passed, false); assert.deepEqual(protectedResult.changed_scopes, ["exposed_package"]);
});

test("exact target load is established from audited Codex command text", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-load-audit-")); const path = join(root, "source", "humanizer-zh", "SKILL.md");
  mkdirSync(join(root, "source", "humanizer-zh"), { recursive: true }); writeFileSync(path, "skill");
  const canonical = realpathSync(path);
  const transcript = JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: `cat -- '${canonical}'`, aggregated_output: "skill", exit_code: 0, status: "completed" } });
  assert.equal(assessExactLoad(transcript, path).passed, true);
  const failed = JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: `cat -- '${canonical}'`, aggregated_output: "skill", exit_code: 0, status: "failed" } }); assert.equal(assessExactLoad(failed, path).passed, false);
  assert.equal(assessExactLoad(transcript, join(root, "source", "other", "SKILL.md")).passed, false);
});

test("one-call Find load proves exact content and completes route order without a filesystem read", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-one-call-load-")); const bootstrap = join(root, "bootstrap", "SKILL.md"); const target = join(root, "target", "SKILL.md");
  mkdirSync(join(root, "bootstrap")); mkdirSync(join(root, "target")); writeFileSync(bootstrap, "bootstrap"); writeFileSync(target, "---\nname: target\n---\ncomplete instructions\n");
  const content = readFileSync(target, "utf8"); const contentSha = await digest(content); const command = "skillroster find '完整任务' --hint 'target helper' --load --limit 1 --json";
  const envelope = JSON.stringify({ schema_version: 1, ok: true, command: "find", result: { ranking_strategy: "task_hint_reciprocal_rank_fusion", matches: [{ rank: 1, name: "target", paths: [target] }], loaded_skill: { selection: { rank: 1 }, content: { path: target, complete: true, text: content, byte_length: Buffer.byteLength(content), sha256: contentSha }, verification: { identity_matches_snapshot: true, entrypoint_digest_matches_snapshot: true, package_fingerprint_matches_snapshot: true, package_fingerprint_complete: true }, task_success: "not_evaluated" } } });
  const transcript = [
    JSON.stringify({ type: "item.started", item: { id: "find", type: "command_execution", command } }),
    JSON.stringify({ type: "item.completed", item: { id: "find", type: "command_execution", command, aggregated_output: envelope, exit_code: 0, status: "completed" } }),
  ].join("\n");
  const audit = { count: 1, contract_violation: false, calls: [{ argv_shape_valid: true, envelope_valid: true, exit_code: 0, hint_count: 1, hint_nonempty: true, task_sha256: await digest("完整任务"), top1_skill: "target", top1_path_sha256: await digest(realpathSync(target)), loaded_content_complete: true, loaded_content_sha256: contentSha }] };
  assert.equal(assessOneCallLoad(transcript, target).passed, true);
  assert.equal(assessRouteOrder(transcript, { bootstrapPath: bootstrap, targetPath: target, findAudit: audit, expectedTask: "完整任务", expectedSkill: "target", oneCall: true }).passed, true);
  const redundantRead = `${transcript}\n${JSON.stringify({ type: "item.started", item: { id: "read", type: "command_execution", command: `cat '${target}'` } })}\n${JSON.stringify({ type: "item.completed", item: { id: "read", type: "command_execution", command: `cat '${target}'`, aggregated_output: content, exit_code: 0, status: "completed" } })}`;
  assert.equal(assessRouteOrder(redundantRead, { bootstrapPath: bootstrap, targetPath: target, findAudit: audit, expectedTask: "完整任务", expectedSkill: "target", oneCall: true }).passed, false);
});

test("exact target load rejects path mentions, prefixes, and non-reading commands", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-load-spoof-")); const target = join(root, "SKILL.md"); writeFileSync(target, "one\ntwo\nthree\n"); const canonical = realpathSync(target);
  const events = [
    { type: "item.completed", item: { type: "command_execution", command: `printf '%s' '${canonical}'`, aggregated_output: canonical, exit_code: 0, status: "completed" } },
    { type: "item.completed", item: { type: "command_execution", command: `cat '${canonical}.not-read'`, aggregated_output: "one\ntwo\nthree\n", exit_code: 0, status: "completed" } },
    { type: "item.completed", item: { type: "command_execution", command: `cat 'prefix-${canonical}'`, aggregated_output: "one\ntwo\nthree\n", exit_code: 0, status: "completed" } },
  ].map(JSON.stringify).join("\n");
  assert.equal(assessExactLoad(events, target).passed, false);
  const truncated = JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: `cat '${canonical}'`, aggregated_output: "one\n", exit_code: 0, status: "completed" } });
  assert.equal(assessExactLoad(truncated, target).passed, false);
});

test("cumulative sed reads prove full load only after all lines are covered", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-load-ranges-")); const target = join(root, "SKILL.md"); writeFileSync(target, "one\ntwo\nthree\n"); const canonical = realpathSync(target);
  const event = (command, aggregated_output) => JSON.stringify({ type: "item.completed", item: { type: "command_execution", command, aggregated_output, exit_code: 0, status: "completed" } });
  assert.equal(assessExactLoad(event(`sed -n '1,2p' '${canonical}'`, "one\ntwo\n"), target).passed, false);
  const full = assessExactLoad(`${event(`sed -n '1,2p' '${canonical}'`, "one\ntwo\n")}\n${event(`sed -n '3,8p' '${canonical}'`, "three\n")}`, target);
  assert.equal(full.passed, true); assert.equal(full.load_event_index, 1);
});

test("exact target load classifies every compound suffix and rejects writes", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-leading-load-")); const target = join(root, "SKILL.md"); writeFileSync(target, "one\ntwo\n"); const canonical = realpathSync(target);
  const command = `/bin/zsh -lc "sed -n '1,240p' '${canonical}' && printf '\\nDONE\\n'"`;
  const event = (type, output = null, value = command) => JSON.stringify({ type: `item.${type}`, item: { id: "compound", type: "command_execution", command: value, ...(type === "completed" ? { aggregated_output: output, exit_code: 0, status: "completed" } : {}) } });
  const transcript = `${event("started")}\n${event("completed", "one\ntwo\n\nDONE\n")}`;
  assert.equal(assessExactLoad(transcript, target).passed, true);
  assert.equal(assessCoreOrder(transcript, target).passed, true);
  const tailOnly = `${event("started")}\n${event("completed", "DONE\n")}`; assert.equal(assessExactLoad(tailOnly, target).passed, false);
  const mutatingCommand = `/bin/zsh -lc "sed -n '1,240p' '${canonical}' && touch /tmp/unaudited"`;
  const mutating = `${event("started", null, mutatingCommand)}\n${event("completed", "one\ntwo\n", mutatingCommand)}`; assert.equal(assessExactLoad(mutating, target).passed, false);
  const outsideRead = `/bin/zsh -lc "sed -n '1,240p' '${canonical}' && cat /etc/hosts"`; assert.equal(assessExactLoad(event("completed", "one\ntwo\n", outsideRead), target).passed, false);
  const outsideReadSequence = `/bin/zsh -lc "cat '${canonical}'\ncat /etc/hosts"`; assert.equal(assessExactLoad(event("completed", "one\ntwo\n", outsideReadSequence), target).passed, false);
  const unsafeListing = `/bin/zsh -lc "sed -n '1,240p' '${canonical}' && rg --files /tmp"`; assert.equal(assessExactLoad(event("completed", "one\ntwo\n", unsafeListing), target).passed, false);
  const semicolon = transcript.replaceAll(" && ", "; "); assert.equal(assessExactLoad(semicolon, target).passed, false);
});

test("on-demand route order permits metadata-authorized Find followed by exact target load", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-route-order-")); const bootstrap = join(root, "bootstrap.md"); const target = join(root, "target.md"); const workspace = join(root, "input.md");
  writeFileSync(bootstrap, "bootstrap"); writeFileSync(target, "target"); writeFileSync(workspace, "input");
  const call = { argv_shape_valid: true, envelope_valid: true, exit_code: 0, hint_count: 1, hint_nonempty: true, task_sha256: await digest("完整任务"), top1_skill: "sample", top1_path_sha256: await digest(realpathSync(target)) }; const findAudit = { count: 1, calls: [call] };
  let sequence = 0; const event = (command, aggregated_output = "") => { const id = `cmd-${sequence += 1}`; return [JSON.stringify({ type: "item.started", item: { id, type: "command_execution", command } }), JSON.stringify({ type: "item.completed", item: { id, type: "command_execution", command, aggregated_output, exit_code: 0, status: "completed" } })].join("\n"); };
  const valid = [event(`cat '${realpathSync(bootstrap)}'`, "bootstrap"), event("skillroster find '完整任务' --hint 'agent hint' --json"), event(`cat '${realpathSync(target)}'`, "target"), event(`cat '${realpathSync(workspace)}'`, "input")].join("\n");
  assert.equal(assessRouteOrder(valid, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).passed, true);
  const retryTranscript = [event(`cat '${realpathSync(bootstrap)}'`, "bootstrap"), event("skillroster find '完整任务' --hint 'first hint' --json"), event("skillroster find '完整任务' --hint 'refined hint' --json"), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  const wrong = { ...call, top1_skill: "other" }; const retryAudit = { count: 2, calls: [wrong, call] };
  assert.equal(assessRouteOrder(retryTranscript, { bootstrapPath: bootstrap, targetPath: target, findAudit: retryAudit, expectedTask: "完整任务", expectedSkill: "sample" }).passed, true);
  const beforeFind = [event(`cat '${realpathSync(workspace)}'`, "input"), event("skillroster find '完整任务' --hint 'agent hint' --json"), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  assert.match(assessRouteOrder(beforeFind, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /task_command_before_find/u);
  const beforeLoad = [event("skillroster find '完整任务' --hint 'agent hint' --json"), event(`cat '${realpathSync(workspace)}'`, "input"), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  assert.match(assessRouteOrder(beforeLoad, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /task_command_before_load/u);
  assert.match(assessRouteOrder(valid, { bootstrapPath: bootstrap, targetPath: target, findAudit: { count: 2, calls: [call, call] }, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /find_count_mismatch/u);
  const startedEarly = [
    JSON.stringify({ type: "item.started", item: { id: "early", type: "command_execution", command: `cat '${realpathSync(workspace)}'` } }),
    event("skillroster find '完整任务' --hint 'agent hint' --json"), event(`cat '${realpathSync(target)}'`, "target"),
    JSON.stringify({ type: "item.completed", item: { id: "early", type: "command_execution", command: `cat '${realpathSync(workspace)}'`, aggregated_output: "input", exit_code: 0, status: "completed" } }),
  ].join("\n");
  assert.match(assessRouteOrder(startedEarly, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /task_command_before_find/u);
  const todoBeforeFind = [JSON.stringify({ type: "item.completed", item: { type: "todo_list", items: [] } }), event(`cat '${realpathSync(bootstrap)}'`, "bootstrap"), event("skillroster find '完整任务' --hint 'agent hint' --json"), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  assert.match(assessRouteOrder(todoBeforeFind, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /todo_list/u);
  const directFind = [event("skillroster find '完整任务' --hint 'agent hint' --json"), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  const directAssessment = assessRouteOrder(directFind, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" });
  assert.equal(directAssessment.passed, true); assert.equal(directAssessment.bootstrap_loaded, false);
  const unsafePrefix = [event(`printf task > /tmp/TASK && skillroster find '完整任务' --hint 'agent hint' --json`), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  assert.match(assessRouteOrder(unsafePrefix, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /find_shell_shape_invalid/u);
  const assignment = [event(`TASK='完整任务'; skillroster find "$TASK" --hint 'agent hint' --json`), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  assert.equal(assessRouteOrder(assignment, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).passed, true);
  const codexAssignmentCommand = `/bin/zsh -lc \\"TASK='完整任务'\nskillroster find \\\\\\"\\"'$TASK\\" --hint \\"agent hint\\" --json'`;
  const codexAssignment = [event(codexAssignmentCommand), event(`cat '${realpathSync(target)}'`, "target")].join("\n");
  assert.equal(assessRouteOrder(codexAssignment, { bootstrapPath: bootstrap, targetPath: target, findAudit, expectedTask: "完整任务", expectedSkill: "sample" }).passed, true);
});

test("target read cannot start until the matching Find completion is ledger-authorized", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-find-completion-")); const bootstrap = join(root, "bootstrap.md"); const target = join(root, "target.md"); writeFileSync(bootstrap, "bootstrap"); writeFileSync(target, "target"); const boot = `cat '${realpathSync(bootstrap)}'`; const find = "skillroster find '完整任务' --hint 'agent hint' --json"; const load = `cat '${realpathSync(target)}'`;
  const started = (id, command) => JSON.stringify({ type: "item.started", item: { id, type: "command_execution", command } }); const completed = (id, command, output) => JSON.stringify({ type: "item.completed", item: { id, type: "command_execution", command, aggregated_output: output, exit_code: 0, status: "completed" } });
  const transcript = [started("boot", boot), completed("boot", boot, "bootstrap"), started("find", find), started("load", load), completed("load", load, "target"), completed("find", find, "")].join("\n");
  const call = { argv_shape_valid: true, envelope_valid: true, exit_code: 0, hint_count: 1, hint_nonempty: true, task_sha256: await digest("完整任务"), top1_skill: "sample", top1_path_sha256: await digest(realpathSync(target)) };
  assert.match(assessRouteOrder(transcript, { bootstrapPath: bootstrap, targetPath: target, findAudit: { count: 1, calls: [call] }, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /target_load_before_find_complete/u);
  const failedFind = JSON.stringify({ type: "item.completed", item: { id: "find", type: "command_execution", command: find, aggregated_output: "failed", exit_code: 1, status: "failed" } });
  const failedTranscript = [started("boot", boot), completed("boot", boot, "bootstrap"), started("find", find), failedFind, started("load", load), completed("load", load, "target")].join("\n");
  assert.match(assessRouteOrder(failedTranscript, { bootstrapPath: bootstrap, targetPath: target, findAudit: { count: 1, calls: [{ ...call, exit_code: 1, envelope_valid: false }] }, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /target_load_before_find_complete/u);
});

test("Core control rejects todo or workspace action before exact target full load", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-core-order-")); const target = join(root, "SKILL.md"); const workspace = join(root, "input.md"); writeFileSync(target, "target"); writeFileSync(workspace, "input"); let sequence = 0;
  const event = (command, output) => { const id = `core-${sequence += 1}`; return [JSON.stringify({ type: "item.started", item: { id, type: "command_execution", command } }), JSON.stringify({ type: "item.completed", item: { id, type: "command_execution", command, aggregated_output: output, exit_code: 0, status: "completed" } })].join("\n"); };
  const valid = [event(`cat '${realpathSync(target)}'`, "target"), event(`cat '${realpathSync(workspace)}'`, "input")].join("\n"); assert.equal(assessCoreOrder(valid, target).passed, true);
  const early = [event(`cat '${realpathSync(workspace)}'`, "input"), event(`cat '${realpathSync(target)}'`, "target")].join("\n"); assert.match(assessCoreOrder(early, target).violations.join(","), /before_core_load/u);
  const todo = [JSON.stringify({ type: "item.started", item: { type: "todo_list" } }), event(`cat '${realpathSync(target)}'`, "target")].join("\n"); assert.match(assessCoreOrder(todo, target).violations.join(","), /todo_list/u);
});

test("command event state machine rejects completed-before-start and command mutation", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-event-order-")); const bootstrap = join(root, "bootstrap.md"); const target = join(root, "target.md"); writeFileSync(bootstrap, "bootstrap"); writeFileSync(target, "target");
  const find = "skillroster find '完整任务' --hint 'agent hint' --json"; const load = `cat '${realpathSync(target)}'`; const boot = `cat '${realpathSync(bootstrap)}'`;
  const completed = (id, command, output) => JSON.stringify({ type: "item.completed", item: { id, type: "command_execution", command, aggregated_output: output, exit_code: 0, status: "completed" } }); const started = (id, command) => JSON.stringify({ type: "item.started", item: { id, type: "command_execution", command } });
  const outOfOrder = [completed("x", boot, "bootstrap"), started("find", find), completed("find", find, ""), started("load", load), completed("load", load, "target"), started("x", boot)].join("\n");
  const call = { argv_shape_valid: true, envelope_valid: true, exit_code: 0, hint_count: 1, hint_nonempty: true, task_sha256: await digest("完整任务"), top1_skill: "sample", top1_path_sha256: await digest(realpathSync(target)) };
  assert.match(assessRouteOrder(outOfOrder, { bootstrapPath: bootstrap, targetPath: target, findAudit: { count: 1, calls: [call] }, expectedTask: "完整任务", expectedSkill: "sample" }).violations.join(","), /event_protocol/u);
  const changed = [started("x", boot), completed("x", `${boot} `, "bootstrap")].join("\n"); assert.match(assessCoreOrder(changed, bootstrap).violations.join(","), /command_changed/u);
});

test("transcript integrity requires complete JSONL, one turn completion, and a successful command", () => {
  const command = JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: "pwd", exit_code: 0, status: "completed" } });
  const completed = JSON.stringify({ type: "turn.completed", usage: {} });
  assert.equal(assessTranscriptIntegrity(`${command}\n${completed}\n`).passed, true);
  assert.equal(assessTranscriptIntegrity(`${command}\nnot-json\n${completed}`).passed, false);
  assert.equal(assessTranscriptIntegrity(command).passed, false);
  assert.equal(assessTranscriptIntegrity(JSON.stringify({ type: "turn.completed" })).passed, false);
  const failed = JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: "false", exit_code: 1, status: "failed" } });
  const assessed = assessTranscriptIntegrity(`${failed}\n${command}\n${completed}`);
  assert.equal(assessed.passed, true); assert.equal(assessed.unsuccessful_command_count, 1); assert.doesNotMatch(assessed.violations.join(","), /incomplete/u);
});

test("oracle and safety remain independent outcome dimensions", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-transfer-oracle-")); mkdirSync(join(root, "outputs")); writeFileSync(join(root, "outputs/out.md"), "hello 42");
  assert.equal(evaluateOracle(root, { path: "outputs/out.md", required_substrings: ["42"] }).passed, true);
  const surface = { passed: true }; const retrieval = { count: 1, top1_correct: true, returned_path_exact: true, contract_violation: false }; const load = { passed: true }; const oracle = { passed: true };
  const safe = deriveArmOutcome({ arm: "on_demand", surface, retrieval, load, oracle, workspace: { passed: true } });
  assert.equal(safe.accepted, true);
  const unsafe = deriveArmOutcome({ arm: "on_demand", surface, retrieval, load, oracle, workspace: { passed: false } });
  assert.equal(unsafe.task, "succeeded"); assert.equal(unsafe.safety, "failed"); assert.equal(unsafe.accepted, false);
  const coreWithoutLoad = deriveArmOutcome({ arm: "core", surface, retrieval: { count: 0, contract_violation: false }, load: { passed: false }, oracle, workspace: { passed: true } });
  assert.equal(coreWithoutLoad.load, "load_wrong"); assert.equal(coreWithoutLoad.accepted, false);
});

test("JSON oracle compares structure without depending on object key order", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-json-oracle-")); mkdirSync(join(root, "outputs"));
  writeFileSync(join(root, "outputs/events.json"), JSON.stringify({ records: [{ state: "open", id: "evt-01" }], schema_version: 1 }));
  const oracle = { path: "outputs/events.json", json_equals: { schema_version: 1, records: [{ id: "evt-01", state: "open" }] } };
  assert.equal(evaluateOracle(root, oracle).passed, true);
  writeFileSync(join(root, "outputs/events.json"), JSON.stringify({ schema_version: 1, records: [{ id: "evt-01", state: "closed" }] }));
  assert.deepEqual(evaluateOracle(root, oracle).failures, ["json_equals:mismatch"]);
  writeFileSync(join(root, "outputs/events.json"), "not json");
  assert.deepEqual(evaluateOracle(root, oracle).failures, ["json_equals:invalid_json"]);
});

test("JSON oracle can require a bounded companion file", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-required-file-oracle-")); mkdirSync(join(root, "outputs"));
  writeFileSync(join(root, "outputs/report.json"), JSON.stringify({ ok: true }));
  writeFileSync(join(root, "outputs/README.md"), "# Report\nItems: 2\n");
  const oracle = { path: "outputs/report.json", json_equals: { ok: true }, required_files: [{ path: "outputs/README.md", required_substrings: ["# Report", "Items: 2"] }] };
  assert.equal(evaluateOracle(root, oracle).passed, true);
  writeFileSync(join(root, "outputs/README.md"), "# Report\n");
  assert.deepEqual(evaluateOracle(root, oracle).failures, ["required_file:missing_substring:outputs/README.md:Items: 2"]);
});

test("Archify transcript attempts require exact shape and lifecycle order", () => {
  const root = mkdtempSync(join(tmpdir(), "codex-archify-receipts-")); const workspace = join(root, "workspace"); const target = join(root, "archify"); const spec = join(workspace, "scratch", "order.spec.json"); const artifact = join(workspace, "outputs", "order.html"); const script = join(target, "bin", "archify.mjs");
  mkdirSync(join(workspace, "scratch"), { recursive: true }); mkdirSync(join(workspace, "outputs"), { recursive: true }); mkdirSync(join(target, "bin"), { recursive: true }); writeFileSync(spec, "{\"type\":\"architecture\"}\n"); writeFileSync(artifact, "<html>static token stuffing</html>\n"); writeFileSync(script, "// frozen tool");
  const contract = { type: "architecture", spec_path: "scratch/order.spec.json", artifact_path: "outputs/order.html", quality: "showcase", validation_check_count: 9 };
  assert.match(evaluateArchifyReceipts("", workspace, contract).join(","), /validate_attempt_missing/u);
  const started = (id, command) => JSON.stringify({ type: "item.started", item: { id, type: "command_execution", command } });
  const completed = (id, command) => JSON.stringify({ type: "item.completed", item: { id, type: "command_execution", command, aggregated_output: "untrusted agent output", exit_code: 0, status: "completed" } });
  const transcriptPath = (path) => realpathSync(path).replaceAll("\\", "/");
  const validateCommand = `node bin/archify.mjs validate architecture '${transcriptPath(spec)}' --quality showcase --json`; const deliverCommand = `/bin/zsh -lc 'mkdir -p ${transcriptPath(join(workspace, "outputs"))} && node bin/archify.mjs deliver architecture ${transcriptPath(spec)} ${transcriptPath(artifact)} --quality showcase --json'`;
  const transcript = [started("validate", validateCommand), completed("validate", validateCommand), started("deliver", deliverCommand), completed("deliver", deliverCommand)].join("\n");
  assert.deepEqual(evaluateArchifyReceipts(transcript, workspace, contract), []);
  const overlapping = [started("validate", validateCommand), started("deliver", deliverCommand), completed("validate", validateCommand), completed("deliver", deliverCommand)].join("\n");
  assert.match(evaluateArchifyReceipts(overlapping, workspace, contract).join(","), /deliver_started_before_validate_completed/u);
  assert.match(evaluateArchifyReceipts(transcript.replace("mkdir -p", "echo unsafe && mkdir -p"), workspace, contract).join(","), /command_shape_invalid/u);
});

test("parent-owned Archify verification reproduces final bytes with frozen tooling", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-parent-archify-")); const workspace = join(root, "workspace"); const target = join(root, "archify"); const spec = join(workspace, "scratch", "order.spec.json"); const artifact = join(workspace, "outputs", "order.html"); const script = join(target, "bin", "archify.mjs");
  for (const path of [join(workspace, "scratch"), join(workspace, "outputs"), join(target, "bin")]) mkdirSync(path, { recursive: true });
  writeFileSync(spec, JSON.stringify({ meta: { output: artifact } })); writeFileSync(artifact, "<html>frozen reproduction</html>\n");
  writeFileSync(script, `import fs from "node:fs"; const [command,type,input,output]=process.argv.slice(2); const checks=Array.from({length:9},(_,i)=>({name:String(i),ok:true})); if(command==="validate") process.stdout.write(JSON.stringify({schemaVersion:1,ok:true,command,type,input,checks,composition:{status:"pass",summary:{errors:0,warnings:0}}})); else { fs.writeFileSync(output,"<html>frozen reproduction</html>\\n"); process.stdout.write(JSON.stringify({schemaVersion:1,ok:true,command,type,input,output})); }`);
  const contract = { type: "architecture", spec_path: "scratch/order.spec.json", artifact_path: "outputs/order.html", quality: "showcase", validation_check_count: 9 };
  const scriptDigest = await digest(readFileSync(script)); const authority = { protected_scopes_passed: true, expected: { script_content_sha256: scriptDigest, package_tree_sha256: await digest(`bin/archify.mjs\0${scriptDigest}`) } };
  const verified = verifyArchifyParent(workspace, target, contract, authority); assert.equal(verified.passed, true, verified.failures.join(",")); assert.equal(verified.source_workspace_sha256_before, verified.source_workspace_sha256_after); writeFileSync(artifact, "tampered\n"); assert.match(verifyArchifyParent(workspace, target, contract, authority).failures.join(","), /artifact_reproduction_mismatch/u);
});

test("parent verification rejects linked or digest-drifted frozen tools without execution", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-parent-tool-attack-")); const workspace = join(root, "workspace"); const spec = join(workspace, "scratch", "order.spec.json"); const artifact = join(workspace, "outputs", "order.html"); const external = join(root, "external"); const marker = join(root, "executed.marker");
  for (const path of [join(workspace, "scratch"), join(workspace, "outputs"), join(external, "bin")]) mkdirSync(path, { recursive: true }); writeFileSync(spec, JSON.stringify({ meta: { output: artifact } })); writeFileSync(artifact, "agent\n"); const malicious = join(external, "bin", "archify.mjs"); writeFileSync(malicious, `import fs from "node:fs"; fs.writeFileSync(${JSON.stringify(marker)},"executed");`);
  const scriptDigest = await digest(readFileSync(malicious)); const expected = { script_content_sha256: scriptDigest, package_tree_sha256: await digest(`bin/archify.mjs\0${scriptDigest}`) }; const contract = { type: "architecture", spec_path: "scratch/order.spec.json", artifact_path: "outputs/order.html", quality: "showcase", validation_check_count: 9 };
  const linkType = process.platform === "win32" ? "junction" : "dir";
  const rootLink = join(root, "root-link"); symlinkSync(external, rootLink, linkType); assert.match(verifyArchifyParent(workspace, rootLink, contract, { protected_scopes_passed: true, expected }).failures.join(","), /frozen_tool_(unsafe|escape)/u); assert.equal(existsSync(marker), false);
  const packageWithLinkedBin = join(root, "linked-bin-package"); mkdirSync(packageWithLinkedBin); symlinkSync(join(external, "bin"), join(packageWithLinkedBin, "bin"), linkType); assert.match(verifyArchifyParent(workspace, packageWithLinkedBin, contract, { protected_scopes_passed: true, expected }).failures.join(","), /frozen_tool_(unsafe|escape)/u); assert.equal(existsSync(marker), false);
  assert.match(verifyArchifyParent(workspace, external, contract, { protected_scopes_passed: true, expected: { ...expected, script_content_sha256: "0".repeat(64) } }).failures.join(","), /digest_mismatch/u); assert.equal(existsSync(marker), false);
});

test("architecture spec contract requires the exact bounded topology and partition", () => {
  const fixture = JSON.parse(readFileSync(new URL("../../fixtures/codex-cold-routing-transfer.json", import.meta.url))); const contract = fixture.tasks.find((task) => task.expected_skill === "archify").oracle.architecture_spec_contract;
  const root = mkdtempSync(join(tmpdir(), "codex-architecture-spec-")); const path = join(root, contract.spec_path); mkdirSync(join(root, "scratch"));
  const value = {
    components: contract.components.map((component) => ({ ...component, type: "backend" })),
    boundaries: contract.boundaries.map((boundary) => ({ label: `信任边界：${boundary.label}`, wraps: boundary.members })),
    connections: contract.connections.map((edge) => ({ id: edge.id, from: edge.from, to: edge.to, label: ({ "web-to-gateway": "HTTPS", "gateway-to-order": "HTTPS", "order-to-db": "SQL", "gateway-to-auth": "HTTPS 认证检查", "order-to-queue": "发布异步事件", "queue-to-worker": "消费异步事件", "worker-to-storage": "写入订单文档" })[edge.id] })),
  };
  writeFileSync(path, JSON.stringify(value)); assert.deepEqual(evaluateArchitectureSpec(root, contract), []);
  writeFileSync(path, JSON.stringify({ ...value, components: [...value.components, { id: "extra", label: "Extra" }] })); assert.match(evaluateArchitectureSpec(root, contract).join(","), /component_set_mismatch/u);
  writeFileSync(path, JSON.stringify({ ...value, boundaries: value.boundaries.map((boundary, index) => index ? boundary : { ...boundary, wraps: [...boundary.wraps, "api-gateway"] }) })); assert.match(evaluateArchitectureSpec(root, contract).join(","), /boundary_(members_mismatch|partition_invalid)/u);
});

test("a failed Core control prevents cold-routing attribution", () => {
  const good = { harness_valid: true, task: "succeeded", safety: "passed", retrieval: "retrieved", load: "loaded", contract_violation: false };
  assert.deepEqual(classifyPair({ ...good, task: "failed" }, good), { attribution: "invalid_core_control", cold_routing_regression: null });
  assert.deepEqual(classifyPair(good, { ...good, contract_violation: true }), { attribution: "on_demand_specific_failure", cold_routing_regression: true });
  assert.deepEqual(classifyPair(good, good), { attribution: "no_observed_regression", cold_routing_regression: false });
});

test("fixture remains readable and contains only the two previously passing families", () => {
  const fixture = JSON.parse(readFileSync(new URL("../../fixtures/codex-cold-routing-transfer.json", import.meta.url)));
  validateManifest(fixture);
  assert.deepEqual(fixture.tasks.map((task) => task.expected_skill).sort(), ["archify", "humanizer-zh"]);
});

test("sealed transfer fixture keeps three distinct capability families and six scheduled runs", () => {
  const fixture = JSON.parse(readFileSync(new URL("../../fixtures/codex-cold-routing-transfer-v2.json", import.meta.url)));
  validateManifest(fixture);
  assert.equal(fixture.formal_protocol_gate, true);
  assert.deepEqual(fixture.tasks.map((task) => task.family), ["instruction_only_rewriting", "reference_backed_extraction", "script_backed_artifact"]);
  assert.equal(fixture.tasks.length * 2 * fixture.trials_per_arm, 6);
  assert.equal(new Set(fixture.tasks.map((task) => task.family)).size, 3);
});
