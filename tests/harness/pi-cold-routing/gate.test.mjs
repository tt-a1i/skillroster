import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import registerGate, { appendGateLedgerLine, architectureGraphFacts, boundedCommandDiagnostics, canonicalPathInRoots, canonicalPolicyPathInRun, classifyContainedWriteDenial, commandArgumentFailureClassification, commandFailureDetail, containsUnquotedShellSyntax, deniedFileAttemptClassification, findParseFailureClassification, isExactInjectedBootstrapPath, isRouteOrderViolation, isSafePreRouteWriteDenial, parseFindCommand, processFailureType, retrievalFailureType, retrievalStageAfter, runAllowlistedProcess, validateCommandArguments, validatedArchitectureEvidence, violatesHintContract } from "./gate.ts";

test("read gate rejects traversal and symlink escapes", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-gate-"));
  const allowed = join(root, "allowed");
  const outside = join(root, "outside");
  mkdirSync(allowed);
  mkdirSync(outside);
  writeFileSync(join(outside, "secret"), "sealed");
  symlinkSync(outside, join(allowed, "escape"));

  assert.throws(() => canonicalPathInRoots(join(allowed, "..", "outside", "secret"), [allowed], "read"), /escapes/);
  assert.throws(() => canonicalPathInRoots(join(allowed, "escape", "secret"), [allowed], "read"), /escapes/);
});

test("write gate rejects symlink aliases and accepts a new descendant", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-gate-"));
  const allowed = join(root, "allowed");
  const outside = join(root, "outside");
  mkdirSync(allowed);
  mkdirSync(outside);
  symlinkSync(join(outside, "result"), join(allowed, "alias"));

  assert.throws(() => canonicalPathInRoots(join(allowed, "alias"), [allowed], "write"), /symbolic link|escapes/);
  assert.equal(canonicalPathInRoots(join(allowed, "new", "result.json"), [allowed], "write"), join(realpathSync(allowed), "new", "result.json"));
});

test("contained new-output denial is nonfatal only after canonical safety proof", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-contained-write-"));
  const workspace = join(root, "workspace"); const outputs = join(workspace, "outputs"); const protectedRoot = join(root, "artifacts");
  mkdirSync(outputs, { recursive: true }); mkdirSync(protectedRoot);
  const input = join(workspace, "session-extract.jsonl"); const allowed = join(outputs, "expected.md"); const sibling = join(outputs, "wrong-name.md");
  writeFileSync(input, "immutable"); writeFileSync(allowed, "existing output");

  const siblingCanonical = canonicalPathInRoots(sibling, [workspace], "write");
  assert.deepEqual(classifyContainedWriteDenial("write", sibling, siblingCanonical, [outputs], [input], [protectedRoot]), { classification: "policy_denial", contained: true });
  assert.deepEqual(classifyContainedWriteDenial("edit", sibling, siblingCanonical, [outputs], [input], [protectedRoot]), { classification: "safety_violation", contained: false });
  assert.deepEqual(classifyContainedWriteDenial("write", allowed, canonicalPathInRoots(allowed, [workspace], "write"), [outputs], [input], [protectedRoot]), { classification: "safety_violation", contained: false });
  assert.deepEqual(classifyContainedWriteDenial("write", input, canonicalPathInRoots(input, [workspace], "write"), [outputs], [input], [protectedRoot]), { classification: "safety_violation", contained: false });

  const protectedOutput = join(protectedRoot, "wrong.md");
  assert.deepEqual(classifyContainedWriteDenial("write", protectedOutput, canonicalPathInRoots(protectedOutput, [root], "write"), [outputs], [input], [protectedRoot]), { classification: "safety_violation", contained: false });
  const outside = join(root, "other", "wrong.md");
  assert.deepEqual(classifyContainedWriteDenial("write", outside, canonicalPathInRoots(outside, [root], "write"), [outputs], [input], [protectedRoot]), { classification: "safety_violation", contained: false });

  const external = mkdtempSync(join(tmpdir(), "skillroster-contained-external-"));
  symlinkSync(external, join(outputs, "escape"));
  assert.throws(() => canonicalPathInRoots(join(outputs, "escape", "wrong.md"), [workspace], "write"), /escapes/u);
  assert.equal(isSafePreRouteWriteDenial(sibling, [workspace], [allowed], [outputs], [input], [protectedRoot]), true);
  assert.equal(isSafePreRouteWriteDenial(join(outputs, "expected-new.md"), [workspace], [join(outputs, "expected-new.md")], [], [input], [protectedRoot]), true);
  assert.equal(isSafePreRouteWriteDenial(input, [workspace], [input], [outputs], [input], [protectedRoot]), false);
  assert.equal(isSafePreRouteWriteDenial(join(external, "escape.md"), [workspace], [], [outputs], [input], [protectedRoot]), false);
});

test("live file hook records a typed contained denial while exact output remains allowed", async () => {
  const suite = mkdtempSync(join(tmpdir(), "skillroster-contained-hook-")); const run = join(suite, "run"); const workspace = join(run, "workspace");
  const outputs = join(workspace, "outputs"); const artifacts = join(run, "artifacts");
  for (const path of [outputs, artifacts, join(run, "home"), join(run, "state"), join(run, "command-home"), join(run, "tmp")]) mkdirSync(path, { recursive: true });
  const bootstrap = join(suite, "bootstrap.md"); const cli = join(suite, "skillroster"); const input = join(workspace, "input.txt");
  const allowed = join(outputs, "expected.md"); const sibling = join(outputs, "other.md"); const ledger = join(artifacts, "events.jsonl"); const policyPath = join(run, "policy.json");
  writeFileSync(bootstrap, "bootstrap"); writeFileSync(cli, "binary"); writeFileSync(input, "immutable");
  writeFileSync(policyPath, JSON.stringify({
    schema_version: 1, run_root: run, suite_root: suite, bootstrap_path: bootstrap, cwd: workspace, ledger_events_path: ledger, arm: "core",
    cli: { executable: cli, home: join(run, "home"), state_dir: join(run, "state") }, expected: { skill_name: "sample", roster_state: "core", task_sha256: "0".repeat(64) },
    hint_required: false, command_timeout_ms: 1000, command_output_max_bytes: 1024, command_environment: { home: join(run, "command-home"), tmp: join(run, "tmp") }, protected_roots: [artifacts, policyPath], immutable_paths: [input],
    pre_load: { read_roots: [] }, post_load: { read_roots: [workspace], write_roots: [workspace], write_paths: [allowed], contained_write_roots: [outputs], commands: [] },
  }));
  const previous = process.env.SKILLROSTER_PI_GATE_POLICY; process.env.SKILLROSTER_PI_GATE_POLICY = policyPath;
  let fileHook;
  try {
    registerGate({ registerTool() {}, on(name, callback) { if (name === "tool_call") fileHook = callback; } });
    assert.equal(await fileHook({ toolName: "write", input: { path: allowed } }), undefined);
    const blocked = await fileHook({ toolName: "write", input: { path: sibling } });
    assert.equal(blocked.block, true);
  } finally {
    if (previous === undefined) delete process.env.SKILLROSTER_PI_GATE_POLICY; else process.env.SKILLROSTER_PI_GATE_POLICY = previous;
  }
  const events = readFileSync(ledger, "utf8").trim().split("\n").map(JSON.parse);
  assert(events.some((event) => event.kind === "file_tool" && event.tool === "write"));
  assert(events.some((event) => event.kind === "file_tool_blocked" && event.classification === "policy_denial" && event.failure_type === "output_path_denied" && event.contained === true));
  assert(!events.some((event) => event.classification === "safety_violation"));
});

test("gate ledger append path enforces its dedicated eight MiB cap", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-gate-ledger-cap-")); const ledger = join(root, "events.jsonl");
  appendGateLedgerLine(ledger, "x".repeat(8 * 1024 * 1024));
  assert.throws(() => appendGateLedgerLine(ledger, "x"), /total bytes/u);
});

test("policy paths allow a symlink-aliased run root but reject real escapes", () => {
  const parent = mkdtempSync(join(tmpdir(), "skillroster-policy-alias-"));
  const realRunRoot = join(parent, "real-run"); const aliasRunRoot = join(parent, "alias-run"); const outside = join(parent, "outside");
  mkdirSync(realRunRoot); mkdirSync(outside); symlinkSync(realRunRoot, aliasRunRoot);
  const missingLedger = join(aliasRunRoot, "artifacts", "gate-events.jsonl");
  assert.equal(canonicalPolicyPathInRun(missingLedger, aliasRunRoot), join(realpathSync(realRunRoot), "artifacts", "gate-events.jsonl"));
  assert.throws(() => canonicalPolicyPathInRun(join(outside, "gate-events.jsonl"), aliasRunRoot), /escapes run root/u);
  symlinkSync(outside, join(realRunRoot, "escape"));
  assert.throws(() => canonicalPolicyPathInRun(join(aliasRunRoot, "escape", "missing.jsonl"), aliasRunRoot), /escapes run root/u);
});

test("only the exact injected Bootstrap path is eligible for route-order classification", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-bootstrap-exact-")); const frozen = join(root, "frozen-inputs"); mkdirSync(frozen);
  const bootstrap = join(frozen, "bootstrap-SKILL.md"); const other = join(frozen, "runner.mjs"); const alias = join(root, "bootstrap-alias.md");
  writeFileSync(bootstrap, "trusted bootstrap"); writeFileSync(other, "trusted control"); symlinkSync(bootstrap, alias);
  assert.equal(isExactInjectedBootstrapPath(bootstrap, bootstrap), true);
  assert.equal(isExactInjectedBootstrapPath(alias, bootstrap), false);
  assert.equal(isExactInjectedBootstrapPath(other, bootstrap), false);
  assert.equal(isRouteOrderViolation("on_demand", "initial"), true);
  const runRoot = join(root, "run"); mkdirSync(runRoot);
  assert.equal(deniedFileAttemptClassification("read", other, runRoot), "safety_violation");
  assert.equal(deniedFileAttemptClassification("read", alias, runRoot), "safety_violation");
});

test("find parser accepts quoted tasks and rejects aliases and shell syntax", () => {
  assert.deepEqual(parseFindCommand('skillroster find "中文 task" --hint "English hint" --json'), {
    task: "中文 task",
    hints: ["English hint"],
  });
  assert.throws(() => parseFindCommand('/tmp/skillroster find "task" --json'), /literal/);
  assert.throws(() => parseFindCommand('skillroster find "task" --json; touch /tmp/pwned'), /shell syntax/);
  assert.throws(() => parseFindCommand('skillroster find "task" --json $(touch /tmp/pwned)'), /shell syntax/);
  assert.equal(findParseFailureClassification('skillroster find "unterminated'), "protocol_denial");
  assert.equal(findParseFailureClassification('skillroster find "task"; whoami'), "safety_violation");
});

test("quoted natural punctuation is literal while unquoted operators remain unsafe", () => {
  const natural = 'skillroster find "为什么失败?! $() [草稿]*" --hint "中文?!" --json';
  assert.equal(containsUnquotedShellSyntax(natural), false);
  assert.deepEqual(parseFindCommand(natural), { task: "为什么失败?! $() [草稿]*", hints: ["中文?!"] });
  assert.equal(containsUnquotedShellSyntax('skillroster find "safe" --json && whoami'), true);
  assert.equal(containsUnquotedShellSyntax('skillroster find "safe" --json $(whoami)'), true);
  assert.equal(violatesHintContract(true, 0), true);
  assert.equal(violatesHintContract(true, 2), true);
  assert.equal(violatesHintContract(true, 1), false);
  assert.equal(violatesHintContract(false, 0), false);
});

test("manifest command arguments enforce literals and canonical roots", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-gate-"));
  const input = join(root, "input.txt");
  const output = join(root, "out", "result.html");
  writeFileSync(input, "input");
  assert.deepEqual(
    validateCommandArguments(["render", input, output], [
      { kind: "literal", value: "render" },
      { kind: "read_path" },
      { kind: "write_path" },
    ], [root], [root]),
    ["render", realpathSync(input), join(realpathSync(root), "out", "result.html")],
  );
  assert.throws(() => validateCommandArguments(["other"], [{ kind: "literal", value: "render" }], [root], [root]), /literal/);
  const exactOutput = join(root, "outputs", "order-architecture.html");
  assert.deepEqual(validateCommandArguments([exactOutput], [{ kind: "write_path" }], [root], [root], root, [exactOutput]), [join(realpathSync(root), "outputs", "order-architecture.html")]);
  assert.throws(() => validateCommandArguments([join(root, "outputs", "extra.html")], [{ kind: "write_path" }], [root], [root], root, [exactOutput]), /explicitly allowlisted/u);
});

test("only future-permitted reads are denials; auth and control reads are unsafe", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-gate-"));
  const workspace = join(root, "workspace"); const auth = join(root, "pi-config"); const control = join(root, "artifacts");
  mkdirSync(workspace); mkdirSync(auth); mkdirSync(control);
  const inside = join(workspace, "input.txt"); const authFile = join(auth, "auth.json"); const ledger = join(control, "ledger.json");
  const outsideRoot = mkdtempSync(join(tmpdir(), "skillroster-outside-"));
  const outside = join(outsideRoot, "secret.txt");
  writeFileSync(inside, "input");
  writeFileSync(authFile, "secret"); writeFileSync(ledger, "control");
  writeFileSync(outside, "secret");
  assert.equal(deniedFileAttemptClassification("read", inside, root, [workspace], [auth, control]), "policy_denial");
  assert.equal(deniedFileAttemptClassification("read", authFile, root, [workspace], [auth, control]), "safety_violation");
  assert.equal(deniedFileAttemptClassification("read", ledger, root, [workspace], [auth, control]), "safety_violation");
  assert.equal(deniedFileAttemptClassification("read", outside, root, [workspace], [auth, control]), "safety_violation");
  assert.equal(deniedFileAttemptClassification("write", inside, root, [workspace], [auth, control]), "safety_violation");
});

test("command argument and child process failures have typed classifications", () => {
  assert.equal(commandArgumentFailureClassification(new Error("literal command argument does not match policy")), "protocol_denial");
  assert.equal(commandArgumentFailureClassification(new Error("write target escapes declared roots")), "safety_violation");
  assert.equal(processFailureType({ failureType: "timeout" }), "timeout");
  assert.equal(processFailureType(new Error("spawn")), "spawn_error");
  const root = mkdtempSync(join(tmpdir(), "skillroster-command-root-")); const outside = mkdtempSync(join(tmpdir(), "skillroster-command-outside-"));
  let escaped; try { validateCommandArguments([join(outside, "missing.txt")], [{ kind: "read_path" }], [root], [root]); } catch (error) { escaped = error; }
  assert.equal(commandArgumentFailureClassification(escaped), "safety_violation");
  assert.deepEqual(commandFailureDetail("deliver", "invalid_arguments", "argument_validation", null), { name: "deliver", failure_type: "invalid_arguments", stage: "argument_validation", args_sha256: null });
  const execution = commandFailureDetail("deliver", "timeout", "execution", ["validated", "/safe/path"]);
  assert.equal(execution.stage, "execution"); assert.equal(execution.failure_type, "timeout"); assert.equal(execution.args_sha256.length, 2); assert(execution.args_sha256.every((digest) => /^[a-f0-9]{64}$/u.test(digest)));
  const diagnostics = boundedCommandDiagnostics('{"error":"validation"}', "warning", 64);
  assert.match(diagnostics, /stdout:/u); assert.match(diagnostics, /validation/u); assert.match(diagnostics, /stderr:/u); assert(Buffer.byteLength(diagnostics) <= 80);
});

test("allowlisted subprocess output is streamed into a hard cap", async () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-output-cap-")); const home = join(root, "home"); const temp = join(root, "tmp"); mkdirSync(home); mkdirSync(temp);
  const policy = { command_timeout_ms: 5000, command_output_max_bytes: 1024, command_environment: { home, tmp: temp } };
  const small = await runAllowlistedProcess(policy, process.execPath, ["-e", "process.stdout.write('ok')"]);
  assert.equal(small.stdout, "ok");
  await assert.rejects(runAllowlistedProcess(policy, process.execPath, ["-e", "process.stdout.write('x'.repeat(4096))"]), (error) => error.failureType === "output_limit");
});

test("validator evidence records normalized graph facts rather than HTML labels", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-graph-facts-")); const source = join(root, "architecture.json");
  const spec = {
    schema_version: 1, diagram_type: "architecture",
    components: ["desk", "hub", "identity", "east", "vault", "rules", "pager", "west"].map((id) => ({ id, label: id })),
    boundaries: [{ label: "one", wraps: ["desk"] }, { label: "two", wraps: ["hub", "identity"] }, { label: "three", wraps: ["east", "west", "vault", "rules", "pager"] }],
    connections: [
      { from: "desk", to: "hub", label: "HTTPS" }, { from: "hub", to: "identity", label: "gRPC" }, { from: "hub", to: "east", label: "mTLS" }, { from: "hub", to: "west", label: "mTLS" },
      { from: "east", to: "vault", label: "MQTT" }, { from: "vault", to: "rules", label: "async event" }, { from: "rules", to: "pager", label: "notify" },
    ],
  };
  writeFileSync(source, JSON.stringify(spec)); const receipt = JSON.stringify({ schemaVersion: 1, ok: true, command: "validate", input: realpathSync(source), checks: [], composition: {} });
  const evidence = validatedArchitectureEvidence(receipt, source); assert.equal(evidence.graph_facts.component_count, 8); assert.equal(evidence.graph_facts.boundary_count, 3); assert.equal(evidence.graph_facts.has_directed_cycle, false);
  const cyclic = structuredClone(spec); cyclic.connections.push({ from: "pager", to: "hub", label: "loop" }); assert.equal(architectureGraphFacts(cyclic).has_directed_cycle, true);
  assert.throws(() => validatedArchitectureEvidence(JSON.stringify({ schemaVersion: 1, ok: true, command: "validate", input: join(root, "other.json") }), source), /bind/u);
});

test("task mismatch is a typed retrieval failure even with the expected result", () => {
  assert.equal(retrievalFailureType(true, true), "task_mismatch");
  assert.equal(retrievalFailureType(false, false), "wrong_result");
  assert.equal(retrievalFailureType(false, true), null);
});

test("Core retrieval failures cannot downgrade post-load write permission", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-core-stage-"));
  const stage = retrievalStageAfter("core", "task_loaded", "retrieval_wrong");
  assert.equal(stage, "task_loaded"); assert.equal(isRouteOrderViolation("core", stage), false);
  assert.equal(canonicalPathInRoots(join(root, "result.md"), [root], "write"), join(realpathSync(root), "result.md"));
  assert.equal(retrievalStageAfter("on_demand", "retrieval_correct", "retrieval_wrong"), "retrieval_wrong");
  assert.equal(isRouteOrderViolation("on_demand", "retrieval_correct"), true);
  assert.equal(isRouteOrderViolation("on_demand", "task_loaded"), false);
});
