#!/usr/bin/env node

import { createHash } from "node:crypto";
import { chmodSync, closeSync, constants as fsConstants, copyFileSync, cpSync, existsSync, fchmodSync, fstatSync, lstatSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, delimiter, dirname, isAbsolute, join, parse, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "../../..");
const SYSTEM_SKILLS = ["imagegen", "openai-docs", "plugin-creator", "skill-creator", "skill-installer"];
const DEFAULT_MANIFEST = join(REPO, "tests/fixtures/codex-cold-routing-transfer.json");
const EVALUATION_MODES = ["paired", "on_demand_transfer"];
const TRANSFER_STAGES = ["no_retrieval_call", "retrieval_wrong", "load_wrong", "task_execution_failed", "task_succeeded"];
const ACTIVE_AUTH_COPIES = new Set();
const ACTIVE_RUNTIME_DIRS = new Set();
for (const [signal, code] of [["SIGINT", 130], ["SIGTERM", 143]]) process.once(signal, () => {
  for (const path of ACTIVE_AUTH_COPIES) rmSync(path, { force: true });
  for (const path of ACTIVE_RUNTIME_DIRS) rmSync(path, { recursive: true, force: true });
  process.exit(code);
});

function fail(message) { throw new Error(message); }
const sha = (value) => createHash("sha256").update(value).digest("hex");
function inside(candidate, root) {
  const suffix = relative(resolve(root), resolve(candidate));
  return suffix === "" || (!suffix.startsWith(`..${sep}`) && suffix !== ".." && !isAbsolute(suffix));
}
function existingAncestorResolvesInside(candidate, root) {
  let current = resolve(candidate);
  while (!existsSync(current)) { const parent = dirname(current); if (parent === current) break; current = parent; }
  return existsSync(current) && inside(realpathSync(current), realpathSync(root));
}
function safeRelative(path, label = "path") {
  if (typeof path !== "string" || !path || isAbsolute(path) || path.split(/[\\/]/u).some((part) => !part || part === "." || part === "..")) fail(`${label} must be a safe relative path`);
  return path.replaceAll("\\", "/");
}

export function parseArgs(argv) {
  const options = {
    manifest: DEFAULT_MANIFEST, task: "all", arm: "both", model: "gpt-5.6-sol", reasoningEffort: "medium",
    codex: "codex", bootstrap: join(REPO, "skill/skillroster/SKILL.md"),
    skillsRoot: join(homedir(), ".agents_skills"), cli: join(REPO, "target/debug/skillroster"),
    runsDir: join(tmpdir(), "skillroster-codex-transfer"), authSource: null,
    timeoutMs: 300_000, execute: false, reevaluateRoot: null, reevaluateOutput: null, summaryOutput: null,
    armExplicit: false,
  };
  const values = new Map([
    ["--manifest", "manifest"], ["--task", "task"], ["--arm", "arm"], ["--model", "model"], ["--reasoning-effort", "reasoningEffort"],
    ["--codex", "codex"], ["--bootstrap", "bootstrap"], ["--skills-root", "skillsRoot"],
    ["--cli", "cli"], ["--runs-dir", "runsDir"], ["--auth-source", "authSource"], ["--timeout-ms", "timeoutMs"],
    ["--reevaluate-root", "reevaluateRoot"], ["--reevaluate-output", "reevaluateOutput"], ["--summary-output", "summaryOutput"],
  ]);
  for (let index = 0; index < argv.length;) {
    if (argv[index] === "--execute") { options.execute = true; index += 1; continue; }
    const key = values.get(argv[index]); const value = argv[index + 1];
    if (!key || value === undefined) fail(`unknown or incomplete argument: ${argv[index] ?? "<missing>"}`);
    options[key] = key === "timeoutMs" ? Number(value) : ["task", "arm", "model", "reasoningEffort", "codex"].includes(key) ? value : resolve(value);
    if (key === "arm") options.armExplicit = true;
    index += 2;
  }
  if (!["core", "on_demand", "both"].includes(options.arm)) fail("--arm must be core, on_demand, or both");
  if (!["low", "medium", "high", "xhigh"].includes(options.reasoningEffort)) fail("--reasoning-effort must be low, medium, high, or xhigh");
  if (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs < 1_000 || options.timeoutMs > 900_000) fail("--timeout-ms must be between 1000 and 900000");
  if (options.execute && options.reevaluateRoot) fail("--execute and --reevaluate-root are mutually exclusive");
  if (options.execute && !options.authSource) fail("--execute requires an explicit --auth-source");
  if (options.reevaluateOutput && !options.reevaluateRoot) fail("--reevaluate-output requires --reevaluate-root");
  if (options.summaryOutput && options.reevaluateRoot) fail("--summary-output and --reevaluate-root are mutually exclusive");
  return options;
}

export function validateManifest(manifest) {
  if (manifest?.schema_version !== 1 || manifest?.harness !== "codex-transfer" || !Array.isArray(manifest.tasks) || manifest.tasks.length < 1) fail("unsupported Codex transfer manifest");
  const trialsPerArm = manifest.trials_per_arm ?? 1;
  if (!Number.isSafeInteger(trialsPerArm) || trialsPerArm < 1 || trialsPerArm > 10) fail("trials_per_arm must be between 1 and 10");
  if (manifest.formal_protocol_gate !== undefined && typeof manifest.formal_protocol_gate !== "boolean") fail("formal_protocol_gate must be boolean");
  const evaluationMode = manifest.evaluation_mode ?? "paired";
  if (!EVALUATION_MODES.includes(evaluationMode)) fail("evaluation_mode must be paired or on_demand_transfer");
  const ids = new Set();
  const families = new Set();
  for (const task of manifest.tasks) {
    if (!/^[a-z0-9][a-z0-9-]*$/u.test(task.id ?? "") || ids.has(task.id)) fail("task id must be unique and path-safe");
    ids.add(task.id);
    if (!/^[a-z0-9][a-z0-9_-]*$/u.test(task.family ?? "") || families.has(task.family)) fail("family must be unique and path-safe");
    families.add(task.family);
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/u.test(task.expected_skill ?? "")) fail(`${task.id} expected_skill is unsafe`);
    if (typeof task.prompt !== "string" || !task.prompt || typeof task.hint !== "string" || !task.hint.trim()) fail(`${task.id} requires prompt and hint`);
    const escapedSkill = task.expected_skill.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
    const protocolTerms = new RegExp(`skillroster|${escapedSkill}|capability\\s+search|能力检索|技能检索|\\bfind\\b`, "iu");
    if (protocolTerms.test(task.prompt)) fail(`${task.id} prompt must not disclose the routing protocol or target Skill`);
    if (task.required_find_calls !== undefined && (!Number.isSafeInteger(task.required_find_calls) || task.required_find_calls < 1 || task.required_find_calls > 2)) fail(`${task.id} required_find_calls must be 1 or 2`);
    if (task.one_call_load !== undefined && typeof task.one_call_load !== "boolean") fail(`${task.id} one_call_load must be boolean`);
    if (!Array.isArray(task.allowed_changed_paths)) fail(`${task.id} allowed_changed_paths is required`);
    for (const path of [...Object.keys(task.workspace_files ?? {}), ...task.allowed_changed_paths]) safeRelative(path, `${task.id} path`);
    if (task.allowed_changed_paths.some((path) => Object.hasOwn(task.workspace_files ?? {}, path))) fail(`${task.id} cannot mutate an input`);
    safeRelative(task.oracle?.path, `${task.id} oracle path`);
    if (!task.allowed_changed_paths.includes(task.oracle.path)) fail(`${task.id} oracle path must be allowlisted`);
    for (const requiredFile of task.oracle.required_files ?? []) {
      safeRelative(requiredFile?.path, `${task.id} required file path`);
      if (!task.allowed_changed_paths.includes(requiredFile.path)) fail(`${task.id} required file path must be allowlisted`);
      for (const value of requiredFile.required_substrings ?? []) if (typeof value !== "string" || !value) fail(`${task.id} required file substring must be non-empty`);
      for (const value of requiredFile.required_regex ?? []) new RegExp(value, "u");
    }
    for (const pattern of [...(task.oracle.required_regex ?? []), ...(task.oracle.forbidden_regex ?? [])]) new RegExp(pattern, "u");
    if (task.oracle?.json_equals !== undefined && (task.oracle.json_equals === null || typeof task.oracle.json_equals !== "object")) fail(`${task.id} json_equals must be an object or array`);
    const receipt = task.oracle.archify_receipt_contract;
    if (receipt && (receipt.type !== "architecture" || safeRelative(receipt.spec_path, `${task.id} receipt spec`) !== receipt.spec_path || safeRelative(receipt.artifact_path, `${task.id} receipt artifact`) !== task.oracle.path || receipt.quality !== "showcase" || receipt.validation_check_count !== 9)) fail(`${task.id} Archify receipt contract is invalid`);
    const spec = task.oracle.architecture_spec_contract;
    if (receipt && (!spec || spec.spec_path !== receipt.spec_path || spec.components?.length !== 8 || spec.boundaries?.length !== 3 || spec.connections?.length !== 7)) fail(`${task.id} architecture spec contract is invalid`);
    for (const edge of spec?.connections ?? []) new RegExp(edge.label_regex, "u");
  }
  if (evaluationMode === "on_demand_transfer") {
    if (manifest.formal_protocol_gate !== true) fail("on_demand_transfer requires formal_protocol_gate");
    if (manifest.tasks.some((task) => task.one_call_load !== true || task.required_find_calls !== 1)) fail("on_demand_transfer requires exactly one audited one-call load per task");
    const schedule = manifest.formal_schedule;
    if (schedule?.arm !== "on_demand" || schedule?.family_count !== manifest.tasks.length || schedule?.trials_per_family !== trialsPerArm || schedule?.run_count !== manifest.tasks.length * trialsPerArm) fail("on_demand_transfer formal_schedule must match the manifest task and trial schedule");
    if (typeof schedule.model !== "string" || !schedule.model.trim()
      || !["low", "medium", "high", "xhigh"].includes(schedule.reasoning_effort)
      || !Number.isSafeInteger(schedule.timeout_ms) || schedule.timeout_ms < 1_000 || schedule.timeout_ms > 900_000) fail("on_demand_transfer formal_schedule must freeze model, reasoning effort, and timeout");
    const stop = manifest.stop_decision;
    if (![stop?.all_scheduled_runs_pass, stop?.any_run_fails, stop?.post_result_tuning].every((value) => typeof value === "string" && value.trim())) fail("on_demand_transfer stop_decision must freeze all outcomes and post-result tuning");
  }
  return manifest;
}

export function extractVisibleSkills(promptInput) {
  let messages;
  try { messages = JSON.parse(promptInput); } catch { return []; }
  const texts = [];
  const visit = (node) => {
    if (!node || typeof node !== "object") return;
    if (node.type === "input_text" && typeof node.text === "string") texts.push(node.text);
    for (const child of Object.values(node)) if (child && typeof child === "object") Array.isArray(child) ? child.forEach(visit) : visit(child);
  };
  visit(messages);
  const names = [];
  for (const text of texts) {
    const section = text.match(/### Available skills\n([\s\S]*?)(?:\n<\/skills_instructions>|\n## |$)/u)?.[1] ?? "";
    for (const match of section.matchAll(/^- ([A-Za-z0-9][A-Za-z0-9._:-]*): /gmu)) names.push(match[1]);
  }
  return [...new Set(names)].sort();
}

export function assessSkillSurface(visibleSkills, arm, expectedSkill) {
  const expected = [...SYSTEM_SKILLS, arm === "core" ? expectedSkill : "skillroster"].sort();
  const actual = [...new Set(visibleSkills)].sort();
  return {
    passed: actual.length === expected.length && actual.every((name, index) => name === expected[index]),
    expected, actual, digest: sha(actual.join("\n")),
    missing: expected.filter((name) => !actual.includes(name)), unexpected: actual.filter((name) => !expected.includes(name)),
  };
}

function specialKind(stat) {
  if (stat.isSymbolicLink()) return "symlink";
  if (stat.isSocket()) return "socket";
  if (stat.isFIFO()) return "fifo";
  if (stat.isBlockDevice()) return "block_device";
  if (stat.isCharacterDevice()) return "character_device";
  return "unknown";
}

function walk(root, prefix = "") {
  const state = new Map();
  if (!existsSync(root)) return state;
  if (!prefix) {
    const rootStat = lstatSync(root);
    if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) { state.set(".", `special:${specialKind(rootStat)}`); return state; }
  }
  for (const name of readdirSync(join(root, prefix)).sort()) {
    const rel = prefix ? `${prefix}/${name}` : name; const path = join(root, rel); const stat = lstatSync(path);
    if (stat.isDirectory()) for (const entry of walk(root, rel)) state.set(entry[0], entry[1]);
    else if (stat.isFile()) state.set(rel, sha(readFileSync(path)));
    else state.set(rel, `special:${specialKind(stat)}`);
  }
  return state;
}

export function snapshotWorkspace(root) { return walk(root); }

export function assessWorkspaceChanges(before, after, inputs, allowed) {
  const changed = [...new Set([...before.keys(), ...after.keys()])].filter((path) => before.get(path) !== after.get(path)).sort();
  const inputMutations = changed.filter((path) => inputs.includes(path));
  const unexpectedChanges = changed.filter((path) => !allowed.includes(path));
  const specialEntries = [...after].filter(([, digest]) => digest.startsWith("special:")).map(([path, digest]) => ({ path, kind: digest.slice("special:".length) }));
  return { changed, input_mutations: inputMutations, unexpected_changes: unexpectedChanges, special_entries: specialEntries, passed: unexpectedChanges.length === 0 && specialEntries.length === 0 };
}

function noFollowWorkspaceFile(workspace, relativePath) {
  const canonicalWorkspace = realpathSync(workspace); let current = workspace;
  const segments = safeRelative(relativePath, "oracle path").split("/");
  for (const [index, segment] of segments.entries()) {
    current = join(current, segment);
    let stat; try { stat = lstatSync(current); } catch { return null; }
    if (stat.isSymbolicLink()) return null;
    if (index < segments.length - 1 && !stat.isDirectory()) return null;
  }
  const stat = lstatSync(current);
  if (!stat.isFile()) return null;
  const canonical = realpathSync(current);
  return inside(canonical, canonicalWorkspace) ? canonical : null;
}

export function evaluateOracle(workspace, oracle, evidence = {}) {
  const path = noFollowWorkspaceFile(workspace, oracle.path); const failures = [];
  if (!path) return { passed: false, failures: [`unsafe_or_missing:${oracle.path}`] };
  const bytes = readFileSync(path); const text = bytes.toString("utf8");
  if (oracle.minimum_bytes && bytes.length < oracle.minimum_bytes) failures.push(`minimum_bytes:${bytes.length}`);
  if (oracle.minimum_characters && [...text.trim()].length < oracle.minimum_characters) failures.push(`minimum_characters:${[...text.trim()].length}`);
  if (oracle.maximum_characters && [...text.trim()].length > oracle.maximum_characters) failures.push(`maximum_characters:${[...text.trim()].length}`);
  for (const value of oracle.required_substrings ?? []) if (!text.includes(value)) failures.push(`missing_substring:${value}`);
  for (const value of oracle.forbidden_substrings ?? []) if (text.includes(value)) failures.push(`forbidden_substring:${value}`);
  for (const value of oracle.required_regex ?? []) if (!new RegExp(value, "u").test(text)) failures.push(`missing_regex:${value}`);
  for (const value of oracle.forbidden_regex ?? []) if (new RegExp(value, "u").test(text)) failures.push(`forbidden_regex:${value}`);
  for (const requiredFile of oracle.required_files ?? []) {
    const requiredPath = noFollowWorkspaceFile(workspace, requiredFile.path);
    if (!requiredPath) { failures.push(`required_file:unsafe_or_missing:${requiredFile.path}`); continue; }
    const requiredText = readFileSync(requiredPath, "utf8");
    for (const value of requiredFile.required_substrings ?? []) if (!requiredText.includes(value)) failures.push(`required_file:missing_substring:${requiredFile.path}:${value}`);
    for (const value of requiredFile.required_regex ?? []) if (!new RegExp(value, "u").test(requiredText)) failures.push(`required_file:missing_regex:${requiredFile.path}:${value}`);
  }
  if (oracle.json_equals !== undefined) {
    let actual; try { actual = JSON.parse(text); } catch { failures.push("json_equals:invalid_json"); }
    if (actual !== undefined && JSON.stringify(canonicalJson(actual)) !== JSON.stringify(canonicalJson(oracle.json_equals))) failures.push("json_equals:mismatch");
  }
  failures.push(...evaluateArchitectureSpec(workspace, oracle.architecture_spec_contract));
  const archifyAttempts = evaluateArchifyReceipts(evidence.transcript ?? "", workspace, oracle.archify_receipt_contract); failures.push(...archifyAttempts);
  const parentVerification = verifyArchifyParent(workspace, evidence.targetPackage, oracle.archify_receipt_contract, evidence.parentVerificationAuthority); if (parentVerification.passed === false) failures.push(...parentVerification.failures);
  return { passed: failures.length === 0, failures, output_sha256: sha(bytes), output_bytes: bytes.length, audit_scope: "independent transcript-attempt and parent-owned frozen correctness evidence", archify_agent_attempts: { passed: archifyAttempts.length === 0, failures: archifyAttempts }, archify_parent_verification: parentVerification };
}

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value && typeof value === "object") return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonicalJson(value[key])]));
  return value;
}

export function evaluateArchitectureSpec(workspace, contract) {
  if (!contract) return [];
  const path = noFollowWorkspaceFile(workspace, contract.spec_path); if (!path) return ["architecture_spec:missing_or_unsafe"];
  let value; try { value = JSON.parse(readFileSync(path, "utf8")); } catch { return ["architecture_spec:invalid_json"]; }
  const failures = []; const actualComponents = Array.isArray(value.components) ? value.components : []; const actualBoundaries = Array.isArray(value.boundaries) ? value.boundaries : []; const actualConnections = Array.isArray(value.connections) ? value.connections : [];
  const expectedComponents = new Map(contract.components.map((component) => [component.id, component.label])); const actualIds = actualComponents.map((component) => component?.id);
  if (new Set(actualIds).size !== actualIds.length) failures.push("architecture_spec:duplicate_component_id");
  if (actualComponents.length !== expectedComponents.size || actualComponents.some((component) => expectedComponents.get(component?.id) !== component?.label)) failures.push("architecture_spec:component_set_mismatch");
  const normalizeBoundary = (label) => typeof label === "string" ? label.replace(/^信任边界：/u, "") : null;
  const expectedBoundaries = new Map(contract.boundaries.map((boundary) => [boundary.label, [...boundary.members].sort()])); const seenMembers = [];
  if (actualBoundaries.length !== expectedBoundaries.size) failures.push("architecture_spec:boundary_count_mismatch");
  const seenBoundaryLabels = new Set();
  for (const boundary of actualBoundaries) {
    const label = normalizeBoundary(boundary?.label); if (!expectedBoundaries.has(label) || seenBoundaryLabels.has(label)) { failures.push("architecture_spec:boundary_label_mismatch"); continue; }
    seenBoundaryLabels.add(label); const members = Array.isArray(boundary.wraps) ? [...boundary.wraps].sort() : [];
    if (JSON.stringify(members) !== JSON.stringify(expectedBoundaries.get(label))) failures.push(`architecture_spec:boundary_members_mismatch:${label}`);
    seenMembers.push(...members);
  }
  if (seenMembers.length !== expectedComponents.size || new Set(seenMembers).size !== expectedComponents.size || seenMembers.some((id) => !expectedComponents.has(id))) failures.push("architecture_spec:boundary_partition_invalid");
  const expectedEdges = new Map(contract.connections.map((edge) => [`${edge.from}\0${edge.to}`, edge.label_regex])); const seenEdges = new Set();
  if (actualConnections.length !== expectedEdges.size) failures.push("architecture_spec:connection_count_mismatch");
  for (const edge of actualConnections) {
    const key = `${edge?.from}\0${edge?.to}`;
    if (!expectedEdges.has(key) || seenEdges.has(key) || !expectedComponents.has(edge?.from) || !expectedComponents.has(edge?.to)) failures.push("architecture_spec:connection_identity_mismatch");
    else if (!new RegExp(expectedEdges.get(key), "u").test(edge?.label ?? "")) failures.push(`architecture_spec:connection_label_mismatch:${edge.id}`);
    seenEdges.add(key);
  }
  return [...new Set(failures)];
}

export function parseFindAudit(text, expected) {
  const records = text.trim() ? text.trim().split("\n").map((line) => JSON.parse(line)) : [];
  const calls = records.filter((record) => record.kind === "find_call");
  const first = calls[0] ?? null; const successful = calls.find((call) => call.exit_code === 0 && call.envelope_valid === true && call.top1_skill === expected.skill) ?? null;
  return {
    count: calls.length,
    first_call_task_complete: first?.task_sha256 === sha(expected.task),
    first_call_hint_valid: Boolean(first?.hint_count > 0 && first?.hint_nonempty && first?.hint_sha256?.every(Boolean)),
    first_call_argv_exact: first?.argv_shape_valid === true,
    top1_correct: Boolean(successful),
    returned_path_exact: successful?.top1_path_sha256 === sha(canonicalPath(expected.path)),
    loaded_content_complete: expected.oneCall ? Boolean(successful?.loaded_content_complete) : null,
    loaded_content_exact: expected.oneCall ? successful?.loaded_content_sha256 === sha(readFileSync(expected.path)) : null,
    retry_classification: calls.length === 0 ? "no_retrieval" : calls.length === 1 ? "single_attempt" : first?.task_sha256 !== sha(expected.task) || !first?.hint_nonempty ? "recovered_after_argument_mismatch" : "retried_after_valid_call",
    contract_violation: calls.length > 2 || !first || first.argv_shape_valid !== true || first.envelope_valid !== true || first.task_sha256 !== sha(expected.task) || !(first.hint_count > 0 && first.hint_nonempty) || (expected.oneCall === true && (!successful?.loaded_content_complete || successful?.loaded_content_sha256 !== sha(readFileSync(expected.path)))),
    calls,
  };
}

export function extractCommandEvents(jsonl) {
  const events = [];
  for (const line of jsonl.split("\n")) {
    if (!line.trim()) continue;
    let value; try { value = JSON.parse(line); } catch { continue; }
    const item = value?.item;
    if (value?.type === "item.completed" && item?.type === "command_execution" && typeof item.command === "string") events.push({ kind: "command", command: item.command, output: item.aggregated_output ?? item.output ?? null, exit_code: item.exit_code ?? null, status: item.status ?? "completed" });
    else if (value?.type === "item.completed" && item?.type && !["reasoning", "agent_message", "todo_list"].includes(item.type)) events.push({ kind: "unclassified_action", item_type: item.type });
  }
  return events;
}

function extractRouteEvents(jsonl) {
  const events = []; const states = new Map();
  for (const line of jsonl.split("\n")) {
    if (!line.trim()) continue;
    let value; try { value = JSON.parse(line); } catch { continue; }
    const item = value?.item;
    if (value?.type === "item.started" && item?.type === "command_execution" && typeof item.command === "string") {
      if (!item.id) events.push({ kind: "event_protocol_violation", violation: "command_started_id_missing" });
      else if (states.has(item.id)) events.push({ kind: "event_protocol_violation", violation: `command_started_duplicate_or_late:${item.id}` });
      else states.set(item.id, { command: item.command, completed: false });
      events.push({ kind: "command_start", id: item.id ?? null, command: item.command });
    }
    else if (value?.type === "item.completed" && item?.type === "command_execution" && typeof item.command === "string") {
      const prior = item.id ? states.get(item.id) : null;
      if (!item.id) events.push({ kind: "event_protocol_violation", violation: "command_completed_id_missing" });
      else if (!prior) { events.push({ kind: "event_protocol_violation", violation: `command_completed_without_start:${item.id}` }); states.set(item.id, { command: item.command, completed: true }); }
      else if (prior.completed) events.push({ kind: "event_protocol_violation", violation: `command_completed_duplicate:${item.id}` });
      else { if (prior.command !== item.command) events.push({ kind: "event_protocol_violation", violation: `command_changed:${item.id}` }); prior.completed = true; }
      if (!prior) events.push({ kind: "command_start", id: item.id ?? null, command: item.command });
      events.push({ kind: "command_complete", id: item.id ?? null, command: item.command, output: item.aggregated_output ?? item.output ?? null, exit_code: item.exit_code ?? null, status: item.status ?? "completed" });
    } else if (["item.started", "item.completed"].includes(value?.type) && item?.type && !["reasoning", "agent_message"].includes(item.type)) events.push({ kind: "unclassified_action", item_type: item.type });
  }
  for (const [id, state] of states) if (!state.completed) events.push({ kind: "event_protocol_violation", violation: `command_completion_missing:${id}` });
  return events;
}

export function assessTranscriptIntegrity(jsonl) {
  let malformed = 0; let turnCompleted = 0; let turnFailed = 0;
  for (const line of jsonl.split("\n")) {
    if (!line.trim()) continue;
    let value; try { value = JSON.parse(line); } catch { malformed += 1; continue; }
    if (value?.type === "turn.completed") turnCompleted += 1;
    if (value?.type === "turn.failed") turnFailed += 1;
  }
  const commands = extractCommandEvents(jsonl).filter((event) => event.kind === "command");
  const incompleteCommands = commands.filter((event) => !["completed", "failed"].includes(event.status) || !Number.isInteger(event.exit_code)).length;
  const successfulCommands = commands.filter((event) => event.status === "completed" && event.exit_code === 0).length;
  const unsuccessfulCommands = commands.filter((event) => Number.isInteger(event.exit_code) && (event.status === "failed" || event.exit_code !== 0)).length;
  const violations = [];
  if (malformed) violations.push(`malformed_jsonl:${malformed}`);
  if (turnCompleted !== 1 || turnFailed) violations.push(`turn_completion:${turnCompleted}:${turnFailed}`);
  if (incompleteCommands) violations.push(`incomplete_command_events:${incompleteCommands}`);
  if (!successfulCommands) violations.push("successful_command_event_missing");
  return { passed: violations.length === 0, violations, turn_completed_count: turnCompleted, successful_command_count: successfulCommands, unsuccessful_command_count: unsuccessfulCommands };
}

function writeTargetAllowed(path, workspace, tempRoot) {
  const candidate = isAbsolute(path) ? resolve(path) : resolve(workspace, path);
  return inside(candidate, workspace) || inside(candidate, tempRoot);
}

function writeTargetStatic(path) { return !/(?:^~|[$`*?\[\]{}])/u.test(path); }

function explicitWritePaths(command) {
  const paths = [...redirectionTargets(command)];
  const tokens = simpleCommandTokens(command); if (!tokens?.length) return paths;
  const executable = basename(tokens[0]).toLowerCase(); const args = tokens.slice(1).filter((token) => token !== "--");
  const nonOptions = args.filter((token) => !token.startsWith("-"));
  if (["touch", "mkdir", "tee"].includes(executable)) paths.push(...nonOptions);
  else if (["cp", "mv", "install"].includes(executable) && nonOptions.length >= 2) paths.push(nonOptions.at(-1));
  else if (executable === "sed" && args.includes("-i") && nonOptions.length) paths.push(nonOptions.at(-1));
  return paths;
}

export function assessExternalWrites(jsonl, workspace, tempRoot) {
  const targets = []; const violations = [];
  const commands = extractCommandEvents(jsonl).filter((event) => event.kind === "command");
  for (const [eventIndex, event] of commands.entries()) {
    if (unsupportedShellWrapperWrite(event.command)) violations.push("unresolved_shell_wrapper_write");
    for (const raw of explicitWritePaths(event.command)) {
      if (["/dev/null", "NUL", "nul"].includes(raw)) continue;
      if (!writeTargetStatic(raw)) {
        targets.push({ raw, canonical: null, event_index: eventIndex, command: event.command, allowed: false });
        violations.push("unresolved_write_target"); continue;
      }
      const target = { raw, canonical: canonicalPath(isAbsolute(raw) ? raw : join(workspace, raw)), event_index: eventIndex, command: event.command };
      const allowed = writeTargetAllowed(raw, workspace, tempRoot); targets.push({ ...target, allowed });
      if (!allowed) violations.push(`external_write_target:${target.canonical}`);
    }
  }
  return { passed: violations.length === 0, violations: [...new Set(violations)], targets, audit_scope: "command-visible explicit redirection targets; OS sandbox is the actual write boundary" };
}

export function extractCommandTexts(jsonl) { return extractCommandEvents(jsonl).filter((event) => event.kind === "command").map((event) => event.command); }

function unwrapSimpleShell(command) {
  const trimmed = command.trim();
  const match = simpleShellWrapper(trimmed);
  return match ? match[2] : trimmed;
}

function simpleShellWrapper(command) { return command.match(/^\S*(?:ba|z)?sh\s+-l?c\s+(['"])([\s\S]*)\1$/u); }

function unsupportedShellWrapperWrite(command) {
  const trimmed = command.trim();
  return !simpleShellWrapper(trimmed)
    && /(?:^|\s)\S*(?:ba|z)?sh(?=\s)/u.test(trimmed)
    && /(?:^|\s)-[A-Za-z]*c[A-Za-z]*(?:\s|$)/u.test(trimmed)
    && trimmed.includes(">");
}

function redirectionTargets(command) {
  const trimmed = command.trim(); const wrapper = simpleShellWrapper(trimmed);
  const value = wrapper?.[1] === '"'
    ? wrapper[2].replace(/\\(["\\$`])/gu, "$1").replace(/\\\r?\n/gu, "")
    : wrapper?.[2] ?? trimmed;
  const paths = []; let quote = null;
  for (let index = 0; index < value.length; index += 1) {
    const char = value[index];
    if (quote) {
      if (char === quote) quote = null;
      else if (quote === '"' && char === "\\") index += 1;
      continue;
    }
    if (char === "'" || char === '"') { quote = char; continue; }
    if (char === "\\") { index += 1; continue; }
    if (char !== ">") continue;
    let cursor = index + 1;
    if (value[cursor] === ">") cursor += 1;
    if (value[cursor] === "|" || value[cursor] === "!") cursor += 1;
    while (/[ \t]/u.test(value[cursor] ?? "")) cursor += 1;
    if (value[cursor] === "&") {
      cursor += 1;
      while (/[ \t]/u.test(value[cursor] ?? "")) cursor += 1;
      if (/\d/u.test(value[cursor] ?? "") || value[cursor] === "-") continue;
    }
    let target = ""; let targetQuote = null;
    for (; cursor < value.length; cursor += 1) {
      const targetChar = value[cursor];
      if (targetQuote) {
        if (targetChar === targetQuote) targetQuote = null;
        else if (targetQuote === '"' && targetChar === "\\") { cursor += 1; if (cursor < value.length) target += value[cursor]; }
        else target += targetChar;
        continue;
      }
      if (targetChar === "'" || targetChar === '"') { targetQuote = targetChar; continue; }
      if (targetChar === "\\") { cursor += 1; if (cursor < value.length) target += value[cursor]; continue; }
      if (/\s/u.test(targetChar) || /[;&|<>]/u.test(targetChar)) break;
      target += targetChar;
    }
    if (target) paths.push(target);
    index = Math.max(index, cursor - 1);
  }
  return paths;
}

function simpleCommandTokens(command) {
  const value = unwrapSimpleShell(command);
  const tokens = []; let token = ""; let quote = null;
  for (let index = 0; index < value.length; index += 1) {
    const char = value[index];
    if (char === "`" || (char === "$" && value[index + 1] === "(")) return null;
    if (quote) {
      if (char === quote) quote = null;
      else if (quote === '"' && char === "\\") { index += 1; if (index >= value.length || /[\r\n]/u.test(value[index])) return null; token += value[index]; }
      else token += char;
      continue;
    }
    if (char === "'" || char === '"') { quote = char; continue; }
    if (/[|;&<>]/u.test(char)) return null;
    if (/[\r\n]/u.test(char)) return null;
    if (/\s/u.test(char)) { if (token) { tokens.push(token); token = ""; } continue; }
    if (char === "\\") { index += 1; if (index >= value.length || /[\r\n]/u.test(value[index])) return null; token += value[index]; continue; }
    token += char;
  }
  if (quote) return null;
  if (token) tokens.push(token);
  return tokens;
}

function parseArchifyTranscriptCommand(command, workspace, contract) {
  const inner = unwrapSimpleShell(command); let nodePart = inner; let preparedArtifactParent = false;
  if (inner.includes("&&")) {
    const parts = inner.split(/\s+&&\s+/u); if (parts.length !== 2) return null;
    const mkdir = simpleCommandTokens(parts[0]);
    if (mkdir?.length !== 3 || basename(mkdir[0]) !== "mkdir" || mkdir[1] !== "-p" || canonicalPath(isAbsolute(mkdir[2]) ? mkdir[2] : join(workspace, mkdir[2])) !== canonicalPath(join(workspace, dirname(contract.artifact_path)))) return null;
    nodePart = parts[1]; preparedArtifactParent = true;
  }
  const tokens = simpleCommandTokens(nodePart); if (!tokens || !["node", "node.exe"].includes(basename(tokens[0]).toLowerCase()) || tokens[1] !== "bin/archify.mjs") return null;
  const commandPath = (value) => canonicalPath(isAbsolute(value ?? "") ? value : join(workspace, value ?? ""));
  const common = tokens.at(-3) === "--quality" && tokens.at(-2) === contract.quality && tokens.at(-1) === "--json";
  const validate = tokens.length === 8 && tokens[2] === "validate" && tokens[3] === contract.type && commandPath(tokens[4]) === canonicalPath(join(workspace, contract.spec_path));
  const deliver = tokens.length === 9 && tokens[2] === "deliver" && tokens[3] === contract.type && commandPath(tokens[4]) === canonicalPath(join(workspace, contract.spec_path)) && commandPath(tokens[5]) === canonicalPath(join(workspace, contract.artifact_path));
  return common && ((validate && !preparedArtifactParent) || deliver) ? { command: validate ? "validate" : "deliver", argv: tokens.slice(1), argv_sha256: tokens.slice(1).map(sha) } : null;
}

export function evaluateArchifyReceipts(transcript, workspace, contract) {
  if (!contract) return [];
  const failures = []; let validateSucceeded = false; let deliverSucceeded = false;
  for (const event of extractRouteEvents(transcript)) {
    if (event.kind === "event_protocol_violation") { failures.push(`archify_receipt:event_protocol:${event.violation}`); continue; }
    if (!event.command?.includes("archify.mjs")) continue;
    const shape = parseArchifyTranscriptCommand(event.command, workspace, contract);
    if (!shape) { failures.push("archify_receipt:command_shape_invalid"); continue; }
    if (event.kind === "command_start") {
      if (shape.command === "deliver" && !validateSucceeded) failures.push("archify_receipt:deliver_started_before_validate_completed");
      continue;
    }
    if (event.kind !== "command_complete" || event.exit_code !== 0 || event.status !== "completed" || typeof event.output !== "string") continue;
    if (shape.command === "validate") validateSucceeded = true;
    if (shape.command === "deliver" && validateSucceeded) deliverSucceeded = true;
  }
  if (!validateSucceeded) failures.push("archify_receipt:validate_attempt_missing_or_failed");
  if (!deliverSucceeded) failures.push("archify_receipt:deliver_attempt_missing_or_failed");
  return [...new Set(failures)];
}

export function verifyArchifyParent(workspace, targetPackage, contract, authority = {}) {
  if (!contract) return { passed: true, failures: [], audit_scope: "not_applicable" };
  const failures = []; const spec = noFollowWorkspaceFile(workspace, contract.spec_path); const artifact = noFollowWorkspaceFile(workspace, contract.artifact_path);
  if (!targetPackage || !spec || !artifact) return { passed: false, failures: ["archify_parent:bound_input_missing_or_unsafe"] };
  if (authority.mode === "historical_untrusted") return { passed: null, performed: false, failures: ["archify_parent:historical_tool_identity_untrusted"], audit_scope: "not performed; no trustworthy external snapshot ledger" };
  if (authority.protected_scopes_passed !== true || !authority.expected?.package_tree_sha256 || !authority.expected?.script_content_sha256) return { passed: false, performed: false, failures: ["archify_parent:fresh_tool_identity_untrusted"] };
  const bin = join(targetPackage, "bin"); const script = join(bin, "archify.mjs"); let rootStat; let binStat; let scriptStat;
  try { rootStat = lstatSync(targetPackage); binStat = lstatSync(bin); scriptStat = lstatSync(script); } catch { return { passed: false, performed: false, failures: ["archify_parent:frozen_tool_missing"] }; }
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink() || !binStat.isDirectory() || binStat.isSymbolicLink() || !scriptStat.isFile() || scriptStat.isSymbolicLink()) return { passed: false, performed: false, failures: ["archify_parent:frozen_tool_unsafe"] };
  const canonicalPackage = realpathSync(targetPackage); const canonicalScript = realpathSync(script);
  if (!inside(canonicalScript, canonicalPackage)) return { passed: false, performed: false, failures: ["archify_parent:frozen_tool_escape"] };
  const actual = { package_tree_sha256: stateDigest(targetPackage), script_content_sha256: sha(readFileSync(canonicalScript)) };
  if (actual.package_tree_sha256 !== authority.expected.package_tree_sha256 || actual.script_content_sha256 !== authority.expected.script_content_sha256) return { passed: false, performed: false, failures: ["archify_parent:frozen_tool_digest_mismatch"], expected_identity: authority.expected, actual_identity: actual };
  const realNode = canonicalPath(process.execPath); const workspaceDigestBefore = stateDigest(workspace); const specificationSha256 = sha(readFileSync(spec)); const agentArtifactSha256 = sha(readFileSync(artifact)); const temp = mkdtempSync(join(tmpdir(), "skillroster-archify-parent-")); let independentBytes = null; let deliverReceipt = null; let validate = { status: null }; let validatePassed = false;
  try {
    if (inside(temp, dirname(workspace))) fail("parent verification temp must stay outside the source run root");
    const copiedSpec = join(temp, "spec.json"); const independentArtifact = join(temp, "artifact.html"); copyFileSync(spec, copiedSpec);
    const value = JSON.parse(readFileSync(copiedSpec, "utf8")); value.meta = { ...(value.meta ?? {}), output: independentArtifact }; writeFileSync(copiedSpec, JSON.stringify(value));
    validate = run(realNode, [canonicalScript, "validate", contract.type, copiedSpec, "--quality", contract.quality, "--json"], { cwd: temp, timeout: 60_000 }); let validateReceipt = null; try { validateReceipt = JSON.parse(validate.stdout); } catch {}
    validatePassed = validate.status === 0 && validateReceipt?.schemaVersion === 1 && validateReceipt?.ok === true && validateReceipt?.command === "validate" && validateReceipt?.type === contract.type && canonicalPath(validateReceipt.input ?? "") === canonicalPath(copiedSpec) && validateReceipt?.checks?.length === contract.validation_check_count && validateReceipt.checks.every((check) => check?.ok === true) && validateReceipt?.composition?.status === "pass" && validateReceipt?.composition?.summary?.errors === 0 && validateReceipt?.composition?.summary?.warnings === 0;
    if (!validatePassed) failures.push("archify_parent:validate_failed");
    const deliver = run(realNode, [canonicalScript, "deliver", contract.type, copiedSpec, independentArtifact, "--quality", contract.quality, "--json"], { cwd: temp, timeout: 60_000 });
    try { deliverReceipt = JSON.parse(deliver.stdout); } catch {}
    if (deliver.status !== 0 || deliverReceipt?.schemaVersion !== 1 || deliverReceipt?.ok !== true || deliverReceipt?.command !== "deliver" || canonicalPath(deliverReceipt.output ?? "") !== canonicalPath(independentArtifact) || !existsSync(independentArtifact)) failures.push("archify_parent:independent_deliver_failed");
    else { independentBytes = readFileSync(independentArtifact); const agentBytes = readFileSync(artifact); if (independentBytes.length !== agentBytes.length || sha(independentBytes) !== sha(agentBytes)) failures.push("archify_parent:artifact_reproduction_mismatch"); }
  } catch { failures.push("archify_parent:independent_verification_error"); } finally { rmSync(temp, { recursive: true, force: true }); }
  const workspaceDigestAfter = stateDigest(workspace); if (workspaceDigestAfter !== workspaceDigestBefore) failures.push("archify_parent:source_workspace_changed");
  return { passed: failures.length === 0, performed: true, failures, expected_identity: authority.expected, actual_identity: actual, real_node_sha256: sha(realNode), frozen_script_realpath_sha256: sha(canonicalScript), frozen_script_content_sha256: actual.script_content_sha256, specification_sha256: specificationSha256, agent_artifact_sha256: agentArtifactSha256, independent_artifact_sha256: independentBytes ? sha(independentBytes) : null, source_workspace_sha256_before: workspaceDigestBefore, source_workspace_sha256_after: workspaceDigestAfter, validate: { exit_code: validate.status, ok: validatePassed }, deliver: { ok: deliverReceipt?.ok === true }, audit_scope: "parent-owned frozen validate and independent deliver reproduction" };
}

function parseReadTokens(tokens) {
  if (!tokens?.length) return null;
  const executable = basename(tokens[0]);
  if (executable === "cat") {
    const args = tokens.slice(1).filter((token) => token !== "--");
    return args.length === 1 ? { path: canonicalPath(args[0]), start: 1, end: Number.POSITIVE_INFINITY, full: true } : null;
  }
  if (executable !== "sed" || tokens.length !== 4 || tokens[1] !== "-n") return null;
  const range = tokens[2].match(/^(\d+)(?:,(\d+|\$))?p$/u); if (!range) return null;
  const start = Number(range[1]); const end = range[2] === "$" ? Number.POSITIVE_INFINITY : Number(range[2] ?? range[1]);
  return start > 0 && end >= start ? { path: canonicalPath(tokens[3]), start, end, full: start === 1 && end === Number.POSITIVE_INFINITY } : null;
}

function readTokens(tokens, targetPath) {
  const read = parseReadTokens(tokens);
  return read?.path === canonicalPath(targetPath) ? read : null;
}

function readOnlyShellSegment(segment, targetPath) {
  const tokens = simpleCommandTokens(segment); if (!tokens?.length) return false;
  const executable = basename(tokens[0]);
  if (readTokens(tokens, targetPath)) return true;
  if (executable === "printf") return true;
  if (!["rg", "rg.exe"].includes(executable.toLowerCase()) || tokens[1] !== "--files") return false;
  for (let index = 2; index < tokens.length; index += 1) {
    if (tokens[index] === "-g" && tokens[index + 1]) { index += 1; continue; }
    if (tokens[index] !== ".") return false;
  }
  return true;
}

function narrowRead(command, targetPath, allowLeading = false) {
  const direct = readTokens(simpleCommandTokens(command), targetPath); if (direct || !allowLeading) return direct;
  const inner = unwrapSimpleShell(command); const compound = inner.split(/\s+&&\s+/u);
  if (compound.length > 1) {
    const leading = readTokens(simpleCommandTokens(compound[0]), targetPath);
    return leading && compound.slice(1).every((segment) => readOnlyShellSegment(segment, targetPath)) ? { ...leading, read_only_compound: true } : null;
  }
  const parts = inner.split(/\r?\n/u).map((part) => part.trim()).filter(Boolean);
  if (parts.length < 2) return null;
  const reads = parts.map((part) => parseReadTokens(simpleCommandTokens(part)));
  const canonicalTarget = canonicalPath(targetPath);
  if (reads.some((read) => !read || read.path !== canonicalTarget)) return null;
  const index = reads.findIndex((read) => read.path === canonicalPath(targetPath));
  return index >= 0 ? { ...reads[index], read_sequence: reads } : null;
}

function targetLineCount(path) {
  let stat; try { stat = lstatSync(path); } catch { return null; }
  if (!stat.isFile() || stat.isSymbolicLink()) return null;
  const text = readFileSync(path, "utf8"); return text === "" ? 0 : text.split("\n").length - (text.endsWith("\n") ? 1 : 0);
}

function verifiedRead(command, output, targetPath, allowLeading = false) {
  const range = narrowRead(command, targetPath, allowLeading); if (!range || typeof output !== "string") return null;
  const readOutput = (read) => { const text = readFileSync(read.path, "utf8"); const lines = text.match(/[^\n]*\n|[^\n]+$/gu) ?? []; return read.full ? text : lines.slice(read.start - 1, Number.isFinite(read.end) ? read.end : undefined).join(""); };
  const expected = readOutput(range);
  if (range.read_sequence) return output === range.read_sequence.map(readOutput).join("") ? { ...range, evidence_mode: "content_echo_sequence" } : null;
  if (range.read_only_compound) return output.startsWith(expected) ? { ...range, evidence_mode: "content_echo_read_only_compound" } : null;
  return output === expected ? { ...range, evidence_mode: "content_echo" } : null;
}

function coveredThrough(ranges) {
  let through = 0;
  for (const range of [...ranges].sort((a, b) => a.start - b.start)) { if (range.start > through + 1) break; through = Math.max(through, range.end); }
  return through;
}

export function assessExactLoad(transcript, targetPath) {
  const canonical = canonicalPath(targetPath); const commands = extractCommandEvents(transcript); const lineCount = targetLineCount(canonical); const ranges = []; let loadEventIndex = null; let evidenceMode = null;
  if (lineCount !== null) for (const [index, event] of commands.entries()) {
    if (event.kind !== "command" || event.exit_code !== 0 || event.status !== "completed") continue;
    const read = verifiedRead(event.command, event.output, canonical, true); if (!read) continue;
    ranges.push(read); if (coveredThrough(ranges) >= lineCount) { loadEventIndex = index; evidenceMode = read.evidence_mode; break; }
  }
  return { passed: loadEventIndex !== null, evidence_mode: evidenceMode, target_path_sha256: sha(canonical), audited_command_count: commands.length, load_event_index: loadEventIndex, audit_scope: "completed exact-path reads with verified content echo; compound suffixes must be fully classified as read-only" };
}

function verifiedFindLoad(output, targetPath) {
  if (typeof output !== "string") return null;
  let value; try { value = JSON.parse(output); } catch { return null; }
  const loaded = value?.result?.loaded_skill; const content = loaded?.content; const verification = loaded?.verification;
  const target = readFileSync(targetPath); const text = target.toString("utf8");
  if (value?.schema_version !== 1 || value?.ok !== true || value?.command !== "find"
      || loaded?.selection?.rank !== 1 || canonicalPath(content?.path ?? "") !== canonicalPath(targetPath)
      || content?.complete !== true || content?.text !== text || content?.byte_length !== target.length
      || content?.sha256 !== sha(target) || loaded?.task_success !== "not_evaluated"
      || verification?.identity_matches_snapshot !== true
      || verification?.entrypoint_digest_matches_snapshot !== true
      || verification?.package_fingerprint_matches_snapshot !== true
      || verification?.package_fingerprint_complete !== true) return null;
  return { evidence_mode: "verified_find_json_content", target_path_sha256: sha(canonicalPath(targetPath)), content_sha256: sha(target), content_bytes: target.length };
}

export function assessOneCallLoad(transcript, targetPath) {
  const commands = extractCommandEvents(transcript); let loaded = null; let loadEventIndex = null;
  for (const [index, event] of commands.entries()) {
    if (event.kind !== "command" || !isFindCommand(event.command) || event.exit_code !== 0 || event.status !== "completed") continue;
    loaded = verifiedFindLoad(event.output, targetPath); if (loaded) { loadEventIndex = index; break; }
  }
  return { passed: Boolean(loaded), evidence_mode: loaded?.evidence_mode ?? null, target_path_sha256: sha(canonicalPath(targetPath)), content_sha256: loaded?.content_sha256 ?? null, content_bytes: loaded?.content_bytes ?? null, audited_command_count: commands.length, load_event_index: loadEventIndex, audit_scope: "completed standalone Find command with exact verified SKILL.md content in the same JSON tool result" };
}

function findCommandShape(command) {
  const tokens = simpleCommandTokens(command);
  if (tokens && basename(tokens[0]) === "skillroster" && tokens[1] === "find") return "standalone";
  const inner = unwrapSimpleShell(command); const match = inner.match(/skillroster\s+find(?:\s|$)/u); if (!match) return null;
  const prefix = inner.slice(0, match.index).trim().replace(/;$/u, "").trim().replaceAll('\\"', '"');
  const directAssignment = /^TASK='[^']*'$/u.test(prefix);
  const shellAssignment = /^\S*(?:ba|z)?sh\s+-l?c\s+["']TASK='[^']*'$/u.test(prefix);
  return directAssignment || shellAssignment ? "quoted_task_assignment" : "unsafe_compound";
}

function isFindCommand(command) { return findCommandShape(command) !== null; }

export function assessRouteOrder(transcript, { bootstrapPath, targetPath, findAudit, expectedTask = null, expectedSkill = null, oneCall = false }) {
  const events = extractRouteEvents(transcript); const violations = []; const bootstrapRanges = []; const targetRanges = []; const bootstrapLines = targetLineCount(canonicalPath(bootstrapPath)); const targetLines = targetLineCount(canonicalPath(targetPath));
  const findAttempts = new Map(); let bootstrapLoaded = false; let findStarted = false; let findAuthorized = false; let targetLoaded = false; let transcriptFindCount = 0;
  for (const [index, event] of events.entries()) {
    if (event.kind === "event_protocol_violation") { violations.push(`event_protocol:${event.violation}`); continue; }
    if (event.kind === "unclassified_action") { if (!targetLoaded) violations.push(`unclassified_action_before_load:${index}:${event.item_type}`); continue; }
    const bootstrapRead = narrowRead(event.command, bootstrapPath);
    const targetRead = narrowRead(event.command, targetPath, true);
    if (event.kind === "command_complete") {
      if (isFindCommand(event.command)) {
        const attempt = findAttempts.get(event.id); const call = Number.isInteger(attempt) ? findAudit.calls?.[attempt] : null;
        const contractValid = Boolean(call && call.argv_shape_valid === true && call.envelope_valid === true && call.exit_code === 0 && call.hint_count > 0 && call.hint_nonempty === true && (!expectedTask || call.task_sha256 === sha(expectedTask)));
        const targetMatch = Boolean(call && (!expectedSkill || call.top1_skill === expectedSkill) && call.top1_path_sha256 === sha(canonicalPath(targetPath)));
        const loadedMatch = !oneCall || (call?.loaded_content_complete === true && call?.loaded_content_sha256 === sha(readFileSync(targetPath)));
        if (event.exit_code === 0 && event.status === "completed" && contractValid && targetMatch && loadedMatch) { findAuthorized = true; if (oneCall) targetLoaded = true; }
        else if (!(event.exit_code === 0 && event.status === "completed" && contractValid)) violations.push(`find_completion_contract_invalid:${index}:${attempt ?? "unmatched"}`);
        continue;
      }
      const verifiedBootstrap = event.exit_code === 0 && event.status === "completed" ? verifiedRead(event.command, event.output, bootstrapPath) : null;
      if (verifiedBootstrap) { bootstrapRanges.push(verifiedBootstrap); if (bootstrapLines !== null && coveredThrough(bootstrapRanges) >= bootstrapLines) bootstrapLoaded = true; }
      const verified = event.exit_code === 0 && event.status === "completed" ? verifiedRead(event.command, event.output, targetPath, true) : null;
      if (verified) { targetRanges.push(verified); if (targetLines !== null && coveredThrough(targetRanges) >= targetLines) targetLoaded = true; }
      continue;
    }
    if (isFindCommand(event.command)) { const attempt = transcriptFindCount; transcriptFindCount += 1; if (event.id) findAttempts.set(event.id, attempt); if (findCommandShape(event.command) === "unsafe_compound") violations.push(`find_shell_shape_invalid:${index}`); if (targetLoaded) violations.push(`find_after_load:${index}`); findStarted = true; continue; }
    if (!findStarted && bootstrapRead) continue;
    if (targetRead) { if (!findAuthorized) violations.push(`target_load_before_find_complete:${index}`); else if (oneCall) violations.push(`redundant_target_read_after_one_call_load:${index}`); continue; }
    if (!findStarted) violations.push(`task_command_before_find:${index}`);
    else if (!findAuthorized) violations.push(`task_command_before_find_complete:${index}`);
    else if (!targetLoaded) violations.push(`task_command_before_load:${index}`);
  }
  if (transcriptFindCount !== findAudit.count) violations.push(`find_count_mismatch:${transcriptFindCount}:${findAudit.count}`);
  if (!findStarted) violations.push("find_missing");
  if (!findAuthorized) violations.push("authorized_find_completion_missing");
  if (!targetLoaded) violations.push("exact_target_load_missing");
  return { passed: violations.length === 0, violations, transcript_find_count: transcriptFindCount, bootstrap_loaded: bootstrapLoaded, audit_scope: "Codex completed command_execution order; the model-visible Bootstrap description may authorize Find without loading the full Bootstrap body" };
}

export function assessCoreOrder(transcript, targetPath) {
  const events = extractRouteEvents(transcript); const violations = []; const ranges = []; const lineCount = targetLineCount(canonicalPath(targetPath)); let loaded = false;
  for (const [index, event] of events.entries()) {
    if (event.kind === "event_protocol_violation") { violations.push(`event_protocol:${event.violation}`); continue; }
    if (event.kind === "unclassified_action") { if (!loaded) violations.push(`task_action_before_core_load:${index}:${event.item_type}`); continue; }
    const targetRead = narrowRead(event.command, targetPath, true);
    if (event.kind === "command_complete") { const verified = event.exit_code === 0 && event.status === "completed" ? verifiedRead(event.command, event.output, targetPath, true) : null; if (verified) { ranges.push(verified); if (lineCount !== null && coveredThrough(ranges) >= lineCount) loaded = true; } continue; }
    if (!targetRead && !loaded) violations.push(`task_command_before_core_load:${index}`);
  }
  if (!loaded) violations.push("exact_core_target_load_missing");
  return { passed: violations.length === 0, violations, audit_scope: "Core first-action full-load gate over Codex command event state machine" };
}

export function deriveArmOutcome({ arm, surface, retrieval, load, oracle, workspace, routeOrder = { passed: true }, coreOrder = { passed: true }, transcript = { passed: true }, protectedScopes = { passed: true }, externalWrites = { passed: true } }) {
  const safety = workspace.passed && protectedScopes.passed && externalWrites.passed ? "passed" : "failed";
  const harnessValid = surface.passed && transcript.passed && (arm !== "core" || coreOrder.passed);
  const retrievalStage = arm === "core" ? "core_control" : retrieval.count === 0 ? "no_retrieval" : retrieval.top1_correct ? "retrieved" : "retrieval_wrong";
  const loadStage = arm === "core" ? load.passed ? "loaded" : "load_wrong" : load.passed && retrieval.returned_path_exact ? "loaded" : "load_wrong";
  return {
    harness_valid: harnessValid, retrieval: retrievalStage, load: loadStage,
    task: oracle.passed ? "succeeded" : "failed", safety,
    route_order: arm === "on_demand" ? routeOrder.passed ? "passed" : "failed" : "not_applicable",
    core_order: arm === "core" ? coreOrder.passed ? "passed" : "failed" : "not_applicable",
    contract_violation: arm === "on_demand" && (retrieval.contract_violation || !routeOrder.passed),
    accepted: harnessValid && oracle.passed && safety === "passed" && loadStage === "loaded" && (arm === "core" || (retrievalStage === "retrieved" && !retrieval.contract_violation && routeOrder.passed)),
    containment: "post_hoc_only_not_pi_realtime",
  };
}

export function deriveOnDemandTransferOutcome(inputs) {
  const outcome = deriveArmOutcome({ ...inputs, arm: "on_demand" });
  const { retrieval, load, oracle, routeOrder = { passed: true } } = inputs;
  const retrievalVerified = retrieval.count === 1 && retrieval.top1_correct === true
    && retrieval.returned_path_exact === true && retrieval.contract_violation !== true;
  const loadVerified = retrievalVerified && load.passed === true && routeOrder.passed === true;
  let deepestStage = "no_retrieval_call";
  if (retrieval.count > 0) deepestStage = retrievalVerified ? "load_wrong" : "retrieval_wrong";
  if (loadVerified) deepestStage = oracle.passed === true ? "task_succeeded" : "task_execution_failed";
  const taskSucceededWithoutVerifiedSkillBytes = oracle.passed === true && load.passed !== true;
  const taskSucceededWithoutAcceptedOneCallLoadContract = oracle.passed === true && !loadVerified;
  return {
    ...outcome,
    deepest_stage: deepestStage,
    task_succeeded_without_verified_skill_bytes: taskSucceededWithoutVerifiedSkillBytes,
    task_succeeded_without_accepted_one_call_load_contract: taskSucceededWithoutAcceptedOneCallLoadContract,
    accepted: outcome.accepted && deepestStage === "task_succeeded" && !taskSucceededWithoutAcceptedOneCallLoadContract,
  };
}

export function classifyPair(core, onDemand) {
  if (!core.harness_valid || core.task !== "succeeded" || core.load !== "loaded" || core.safety !== "passed") return { attribution: "invalid_core_control", cold_routing_regression: null };
  if (!onDemand.harness_valid) return { attribution: "invalid_on_demand_harness", cold_routing_regression: null };
  if (onDemand.safety !== "passed") return { attribution: "on_demand_safety_failure", cold_routing_regression: null };
  const regression = onDemand.task !== "succeeded" || onDemand.retrieval !== "retrieved" || onDemand.load !== "loaded" || onDemand.safety !== "passed" || onDemand.contract_violation;
  return { attribution: regression ? "on_demand_specific_failure" : "no_observed_regression", cold_routing_regression: regression };
}

function resultFamily(result) { return result.family ?? "default"; }

export function groupPairedResults(tasks, results, trialsPerArm) {
  return tasks.flatMap((task) => Array.from({ length: trialsPerArm }, (_, index) => {
    const trial = index + 1; const coreResult = results.find((result) => result.task === task.id && result.trial === trial && result.arm === "core"); const onDemandResult = results.find((result) => result.task === task.id && result.trial === trial && result.arm === "on_demand");
    if (coreResult && onDemandResult && coreResult.pair_invariant !== onDemandResult.pair_invariant) return { family: task.family, task: task.id, trial, attribution: "pair_invariant_mismatch", gate: "failed", cold_routing_regression: null };
    if (!coreResult || !onDemandResult) return { family: task.family, task: task.id, trial, attribution: "pair_incomplete", gate: "failed", cold_routing_regression: null };
    const pair = classifyPair(coreResult.outcome, onDemandResult.outcome);
    return { family: task.family, task: task.id, trial, ...pair, gate: pair.attribution === "no_observed_regression" ? "passed" : "failed" };
  }));
}

export function deriveProtocolDecision(results, trialsPerArm, tasks = null, pairs = null) {
  const core = results.filter((result) => result.arm === "core"); const onDemand = results.filter((result) => result.arm === "on_demand");
  const coreAccepted = core.filter((result) => result.outcome?.accepted === true).length;
  const onDemandAccepted = onDemand.filter((result) => result.outcome?.accepted === true).length;
  const onDemandContractFailures = onDemand.filter((result) => result.outcome?.contract_violation === true || result.outcome?.load !== "loaded").length;
  const familyIds = [...new Set((tasks ?? results).map((item) => item.family ?? "default"))];
  const groupedPairs = pairs ?? [];
  const family_gates = familyIds.map((family) => {
    const familyResults = results.filter((result) => resultFamily(result) === family);
    const familyCore = familyResults.filter((result) => result.arm === "core");
    const familyOnDemand = familyResults.filter((result) => result.arm === "on_demand");
    const familyPairs = groupedPairs.filter((pair) => pair.family === family);
    const familyCoreAccepted = familyCore.filter((result) => result.outcome?.accepted === true).length;
    const familyOnDemandAccepted = familyOnDemand.filter((result) => result.outcome?.accepted === true).length;
    const familyContractFailures = familyOnDemand.filter((result) => result.outcome?.contract_violation === true || result.outcome?.load !== "loaded").length;
    const familySafetyInvalid = familyResults.some((result) => result.outcome?.safety !== "passed");
    let familyDecision = "investigate_repeatable_on_demand_gap";
    if (familySafetyInvalid) familyDecision = "stop_invalid_safety_evidence";
    else if (familyCoreAccepted < trialsPerArm) familyDecision = "fix_control_task_or_oracle";
    else if (familyContractFailures >= 2) familyDecision = "fix_bootstrap_or_cli_contract";
    else if (familyOnDemandAccepted === trialsPerArm
      && (pairs ? familyPairs.length === trialsPerArm && familyPairs.every((pair) => pair.gate === "passed") : familyCore.length === trialsPerArm && familyOnDemand.length === trialsPerArm)) familyDecision = "retain_current_design";
    return { family, required_trials_per_arm: trialsPerArm, pair_count: familyPairs.length, core_accepted: familyCoreAccepted, on_demand_accepted: familyOnDemandAccepted, on_demand_contract_failures: familyContractFailures, gate: familyDecision === "retain_current_design" ? "passed" : "failed", decision: familyDecision };
  });
  let decision = "investigate_repeatable_on_demand_gap";
  if (results.some((result) => result.outcome?.safety !== "passed")) decision = "stop_invalid_safety_evidence";
  else if (coreAccepted < familyIds.length * trialsPerArm) decision = "fix_control_task_or_oracle";
  else if (family_gates.some((gate) => gate.on_demand_contract_failures >= 2)) decision = "fix_bootstrap_or_cli_contract";
  else if (family_gates.length > 0 && family_gates.every((gate) => gate.gate === "passed")) decision = "retain_current_design";
  return { decision, required_trials_per_arm: trialsPerArm, expected_runs: familyIds.length * trialsPerArm * 2, family_count: familyIds.length, core_accepted: coreAccepted, on_demand_accepted: onDemandAccepted, on_demand_contract_failures: onDemandContractFailures, family_gates, overall_gate: family_gates.length > 0 && family_gates.every((gate) => gate.gate === "passed") };
}

export function deriveOnDemandTransferDecision(results, trialsPerFamily, tasks, stopDecision = null) {
  const expectedKeys = new Set(tasks.flatMap((task) => Array.from({ length: trialsPerFamily }, (_, index) => `${task.id}#${index + 1}#on_demand`)));
  const actualKeys = results.map((result) => `${result.task}#${result.trial}#${result.arm}`);
  const exactSchedule = results.length === expectedKeys.size
    && new Set(actualKeys).size === actualKeys.length
    && actualKeys.every((key) => expectedKeys.has(key));
  const familyGates = tasks.map((task) => {
    const familyResults = results.filter((result) => result.family === task.family);
    const stageCounts = Object.fromEntries(TRANSFER_STAGES.map((stage) => [stage, familyResults.filter((result) => result.outcome?.deepest_stage === stage).length]));
    const accepted = familyResults.filter((result) => transferEvidenceFieldsValid(result.outcome) && result.outcome?.accepted === true && result.outcome?.deepest_stage === "task_succeeded" && result.outcome.task_succeeded_without_accepted_one_call_load_contract === false).length;
    return { family: task.family, task: task.id, required_trials: trialsPerFamily, run_count: familyResults.length, accepted, stage_counts: stageCounts, gate: familyResults.length === trialsPerFamily && accepted === trialsPerFamily ? "passed" : "failed" };
  });
  const failures = results.filter((result) => !transferEvidenceFieldsValid(result.outcome) || result.outcome?.accepted !== true || result.outcome?.deepest_stage !== "task_succeeded" || result.outcome.task_succeeded_without_accepted_one_call_load_contract === true).map((result) => ({ family: result.family, task: result.task, trial: result.trial, deepest_stage: TRANSFER_STAGES.includes(result.outcome?.deepest_stage) ? result.outcome.deepest_stage : null, safety: result.outcome?.safety ?? null, task_succeeded_without_verified_skill_bytes: result.outcome?.task_succeeded_without_verified_skill_bytes === true, task_succeeded_without_accepted_one_call_load_contract: result.outcome?.task_succeeded_without_accepted_one_call_load_contract === true }));
  const overallGate = exactSchedule && familyGates.every((family) => family.gate === "passed");
  const decision = overallGate ? stopDecision?.all_scheduled_runs_pass ?? "all_scheduled_runs_pass" : stopDecision?.any_run_fails ?? "one_or_more_scheduled_runs_failed";
  return { decision, post_result_tuning: stopDecision?.post_result_tuning ?? null, expected_runs: expectedKeys.size, complete_schedule: exactSchedule, family_gates: familyGates, failures, overall_gate: overallGate };
}

function writeInputs(workspace, files) {
  for (const [relativePath, content] of Object.entries(files)) {
    const path = resolve(workspace, safeRelative(relativePath)); if (!inside(path, workspace)) fail("workspace input escapes root");
    mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, content);
  }
}

function copyAuth(authSource, codexHome) {
  if (!existsSync(authSource) || !statSync(authSource).isFile()) fail("auth source must be a regular file");
  const destination = join(codexHome, "auth.json"); copyFileSync(authSource, destination); ACTIVE_AUTH_COPIES.add(destination);
  try { chmodSync(destination, 0o600); } catch (error) { cleanupAuth(destination); throw error; }
  return destination;
}

function cleanupAuth(path) { rmSync(path, { force: true }); ACTIVE_AUTH_COPIES.delete(path); }
function cleanupRuntime(path) { rmSync(path, { recursive: true, force: true }); ACTIVE_RUNTIME_DIRS.delete(path); }

function validateSummaryOutput(path, runsDir) {
  if (!path) return null;
  const absolute = resolve(path);
  try { lstatSync(absolute); fail("summary output already exists"); } catch (error) { if (error.code !== "ENOENT") throw error; }
  const parent = dirname(absolute); let stat;
  try { stat = lstatSync(parent); } catch { fail("summary output parent must already exist"); }
  if (!stat.isDirectory() || stat.isSymbolicLink()) fail("summary output parent must be a real directory");
  let current = parse(parent).root;
  for (const part of relative(current, parent).split(sep).filter(Boolean)) {
    current = join(current, part);
    if (lstatSync(current).isSymbolicLink()) fail("summary output path must not contain linked ancestors");
  }
  if (inside(absolute, REPO) || existingAncestorResolvesInside(absolute, REPO)) fail("summary output must stay outside the repository");
  if (inside(absolute, runsDir) || existingAncestorResolvesInside(absolute, runsDir)) fail("summary output must stay outside the run root");
  return { path: absolute, parent, parentDevice: stat.dev, parentInode: stat.ino, canonicalParent: realpathSync(parent) };
}

function assertSummaryBoundary(boundary, descriptor = null) {
  const parent = lstatSync(boundary.parent);
  if (!parent.isDirectory() || parent.isSymbolicLink() || parent.dev !== boundary.parentDevice || parent.ino !== boundary.parentInode || realpathSync(boundary.parent) !== boundary.canonicalParent) fail("summary output parent changed during persistence");
  if (descriptor !== null) {
    const opened = fstatSync(descriptor); const named = lstatSync(boundary.path);
    if (!opened.isFile() || named.isSymbolicLink() || opened.dev !== named.dev || opened.ino !== named.ino || opened.nlink !== 1) fail("summary output changed during persistence");
  }
}

function persistSummary(encoded, options) {
  const boundary = validateSummaryOutput(options.summaryOutput, options.runsDir); let descriptor = null;
  try {
    descriptor = openSync(boundary.path, fsConstants.O_WRONLY | fsConstants.O_CREAT | fsConstants.O_EXCL | fsConstants.O_NOFOLLOW, 0o600);
    assertSummaryBoundary(boundary, descriptor); writeFileSync(descriptor, encoded); fchmodSync(descriptor, 0o600); assertSummaryBoundary(boundary, descriptor);
  } catch (error) {
    if (descriptor !== null) {
      try { const opened = fstatSync(descriptor); const named = lstatSync(boundary.path); if (opened.dev === named.dev && opened.ino === named.ino) rmSync(boundary.path, { force: true }); } catch {}
    }
    fail(`summary output persistence failed: ${error.message}`);
  } finally { if (descriptor !== null) closeSync(descriptor); }
}

function emitSummary(summary, options) {
  const encoded = `${JSON.stringify(summary, null, 2)}\n`;
  let persistenceError = null;
  if (options.summaryOutput) try { persistSummary(encoded, options); } catch (error) { persistenceError = error; }
  process.stdout.write(encoded);
  if (persistenceError) throw persistenceError;
}

function run(command, args, options) {
  const result = spawnSync(command, args, { encoding: "utf8", shell: false, maxBuffer: 64 * 1024 * 1024, ...options });
  return { ...result, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function canonicalPath(path) { try { return realpathSync(path); } catch { return resolve(path); } }
function stateDigest(root) { return sha([...walk(root)].map(([path, digest]) => `${path}\0${digest}`).join("\n")); }

export function assessFrozenInputIdentities(facts, options) {
  const actual = {
    manifest_sha256: sha(readFileSync(options.manifest)), cli_sha256: sha(readFileSync(options.cli)),
    bootstrap_sha256: stateDigest(dirname(options.bootstrap)), targets_sha256: stateDigest(options.skillsRoot),
    driver_sha256: sha(readFileSync(fileURLToPath(import.meta.url))), model: options.model,
    reasoning_effort: options.reasoningEffort, timeout_ms: options.timeoutMs,
  };
  const expected = {
    manifest_sha256: facts.manifest_sha256, cli_sha256: facts.cli_sha256,
    bootstrap_sha256: facts.bootstrap_sha256, targets_sha256: facts.targets_sha256,
    driver_sha256: facts.driver_sha256, model: facts.model,
    reasoning_effort: facts.reasoning_effort, timeout_ms: facts.execution_contract.timeout_ms,
  };
  const changed = Object.keys(expected).filter((key) => JSON.stringify(actual[key]) !== JSON.stringify(expected[key]));
  return { passed: changed.length === 0, changed, actual, expected };
}

function executableIdentity(command, env = process.env) {
  const pathEntries = (env.PATH ?? "").split(delimiter).filter(Boolean);
  const explicit = isAbsolute(command) || command.includes("/") || command.includes("\\");
  const extensions = process.platform === "win32" && !parse(command).ext
    ? (env.PATHEXT ?? ".COM;.EXE;.BAT;.CMD").split(";").filter(Boolean)
    : [""];
  const candidates = explicit
    ? extensions.map((extension) => `${resolve(command)}${extension}`)
    : pathEntries.flatMap((directory) => extensions.map((extension) => join(directory, `${command}${extension}`)));
  for (const candidate of candidates) {
    if (!existsSync(candidate)) continue;
    const canonical = canonicalPath(candidate); const stat = lstatSync(canonical);
    if (!stat.isFile() || stat.isSymbolicLink()) continue;
    return { path_sha256: sha(canonical), content_sha256: sha(readFileSync(canonical)) };
  }
  fail("unable to freeze Codex executable identity");
}

function repositoryIdentity() {
  const head = run("git", ["-C", REPO, "rev-parse", "HEAD"], { timeout: 30_000 });
  const tree = run("git", ["-C", REPO, "rev-parse", "HEAD^{tree}"], { timeout: 30_000 });
  const status = run("git", ["-C", REPO, "status", "--porcelain"], { timeout: 30_000 });
  if (head.status !== 0 || tree.status !== 0 || status.status !== 0) fail("unable to freeze repository source identity");
  return { commit: head.stdout.trim(), tree: tree.stdout.trim(), clean: status.stdout.trim() === "" };
}

function protectedPackage(path) {
  try {
    const stat = lstatSync(path); const state = walk(path); const specials = [...state].filter(([, digest]) => digest.startsWith("special:"));
    if (!stat.isDirectory() || stat.isSymbolicLink() || specials.length) return { valid: false, digest: null };
    return { valid: true, digest: stateDigest(path) };
  } catch { return { valid: false, digest: null }; }
}

function protectedAuth(path) {
  try {
    const stat = lstatSync(path); if (!stat.isFile() || stat.isSymbolicLink()) return { valid: false };
    return { valid: true, identity: `${stat.dev}:${stat.ino}:${stat.mode}:${stat.size}`, digest: sha(readFileSync(path)) };
  } catch { return { valid: false }; }
}

export function captureProtectedScopes(paths) {
  return { target_package: protectedPackage(paths.targetPackage), exposed_package: protectedPackage(paths.exposedPackage), auth_copy: protectedAuth(paths.authCopy) };
}

export function assessProtectedScopes(before, after) {
  const changed = Object.keys(before).filter((name) => JSON.stringify(before[name]) !== JSON.stringify(after[name]) || before[name]?.valid !== true || after[name]?.valid !== true);
  return { passed: changed.length === 0, changed_scopes: changed };
}

export function parseFindEnvelope(stdout) {
  const value = parseEnvelope(stdout, "find");
  if (value.result?.ranking_strategy !== "task_hint_reciprocal_rank_fusion") fail("Find did not use task_hint_reciprocal_rank_fusion");
  const candidates = value?.result?.matches ?? [];
  const top = candidates[0] ?? null;
  return { skill: top?.name ?? null, path: top?.paths?.[0] ?? null, roster_state: top?.roster_state ?? null, agents: top?.agents ?? [], loaded: value?.result?.loaded_skill ?? null, raw: value };
}

function parseEnvelope(stdout, command) {
  let value; try { value = JSON.parse(stdout); } catch { fail(`${command} did not return JSON`); }
  if (value?.schema_version !== 1 || value?.ok !== true || value?.command !== command || !value.result) fail(`${command} returned an invalid envelope`);
  return value;
}

export function skillRosterScanArgs(paths, sourceRoot) { return ["--home", paths.home, "--state-dir", paths.state, "--json", "scan", "--source-root", sourceRoot]; }
export function skillRosterFindArgs(paths, task) { return ["--home", paths.home, "--state-dir", paths.state, "--json", "find", "--hint", task.hint, ...(task.one_call_load ? ["--load", "--limit", "1"] : []), "--", task.prompt]; }

export function findWrapperSource(oneCallLoad = false) {
  return `#!/usr/bin/env node
import { createHash } from "node:crypto";
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { realpathSync } from "node:fs";
import { spawnSync } from "node:child_process";
const sha = (value) => createHash("sha256").update(value).digest("hex");
const args = process.argv.slice(2); const separator = args.indexOf("--");
const task = args[0] === "find" && separator === args.length - 2 ? (args[separator + 1] ?? "") : "";
const hints = []; for (let i = 0; i < args.length; i += 1) if (args[i] === "--hint" && args[i + 1] !== undefined) hints.push(args[i + 1]);
const oneCallLoad = ${JSON.stringify(oneCallLoad)};
let result = { status: 64, stdout: "", stderr: oneCallLoad ? "wrapper permits only: skillroster find --hint HINT --load --limit 1 --json -- TASK\\n" : "wrapper permits only: skillroster find --hint HINT --json -- TASK\\n" };
const argvShapeValid = oneCallLoad
  ? args.length === 9 && args[0] === "find" && args[1] === "--hint" && args[3] === "--load" && args[4] === "--limit" && args[5] === "1" && args[6] === "--json" && args[7] === "--" && task.length > 0
  : args.length === 6 && args[0] === "find" && args[1] === "--hint" && args[3] === "--json" && args[4] === "--" && task.length > 0;
if (argvShapeValid) result = spawnSync(process.env.SKILLROSTER_REAL_CLI, ["--home", process.env.SKILLROSTER_TEST_HOME, "--state-dir", process.env.SKILLROSTER_TEST_STATE, "--json", ...args], { encoding: "utf8", shell: false });
let top1 = {}; let envelopeValid = false; let loadedContentComplete = false; let loadedContentSha256 = null; try { const value = JSON.parse(result.stdout ?? ""); envelopeValid = value?.schema_version === 1 && value?.ok === true && value?.command === "find" && value?.result?.ranking_strategy === "task_hint_reciprocal_rank_fusion"; const top = envelopeValid ? value?.result?.matches?.[0] ?? {} : {}; top1 = { skill: top.name ?? null, path: top.paths?.[0] ?? null }; const loaded = value?.result?.loaded_skill; loadedContentComplete = !oneCallLoad || (loaded?.selection?.rank === 1 && loaded?.content?.complete === true && typeof loaded?.content?.text === "string" && loaded?.content?.byte_length === Buffer.byteLength(loaded.content.text, "utf8") && loaded?.content?.sha256 === sha(loaded.content.text) && loaded?.verification?.identity_matches_snapshot === true && loaded?.verification?.entrypoint_digest_matches_snapshot === true && loaded?.verification?.package_fingerprint_matches_snapshot === true && loaded?.verification?.package_fingerprint_complete === true && loaded?.task_success === "not_evaluated"); loadedContentSha256 = loadedContentComplete && oneCallLoad ? sha(loaded.content.text) : null; envelopeValid = envelopeValid && (!oneCallLoad || loadedContentComplete); } catch {}
let canonicalTopPath = null; try { canonicalTopPath = top1.path ? realpathSync(top1.path) : null; } catch { canonicalTopPath = top1.path ? resolve(top1.path) : null; }
appendFileSync(process.env.SKILLROSTER_FIND_AUDIT, JSON.stringify({ kind: "find_call", argv_count: args.length, argv_shape_valid: argvShapeValid, envelope_valid: envelopeValid, task_sha256: sha(task), hint_count: hints.length, hint_nonempty: hints.length > 0 && hints.every((hint) => hint.trim().length > 0), hint_sha256: hints.map(sha), separator_index: separator, exit_code: result.status, top1_skill: top1.skill ?? null, top1_path_sha256: canonicalTopPath ? sha(canonicalTopPath) : null, loaded_content_complete: loadedContentComplete, loaded_content_sha256: loadedContentSha256 }) + "\\n", { mode: 0o600 });
process.stdout.write(result.stdout ?? ""); process.stderr.write(result.stderr ?? ""); process.exit(result.status ?? 1);
`;
}

export function setupArm(root, task, arm, options) {
  const home = join(root, "home"); const codexHome = join(home, ".codex"); const workspace = join(root, "workspace");
  const temp = mkdtempSync(join(canonicalPath(tmpdir()), "skillroster-codex-"));
  ACTIVE_RUNTIME_DIRS.add(temp);
  try {
    for (const path of [codexHome, workspace]) mkdirSync(path, { recursive: true });
    writeInputs(workspace, task.workspace_files);
    const skillDestination = join(codexHome, "skills", arm === "core" ? task.expected_skill : "skillroster"); mkdirSync(dirname(skillDestination), { recursive: true });
    const skillSource = arm === "core" ? join(options.skillsRoot, task.expected_skill) : dirname(options.bootstrap);
    cpSync(skillSource, skillDestination, { recursive: true, dereference: true });
    const targetPath = arm === "core" ? join(skillDestination, "SKILL.md") : join(root, "source", task.expected_skill, "SKILL.md");
    if (arm === "on_demand") { const targetDir = dirname(targetPath); mkdirSync(dirname(targetDir), { recursive: true }); cpSync(join(options.skillsRoot, task.expected_skill), targetDir, { recursive: true, dereference: true }); }
    return { home, codexHome, workspace, temp, targetPath, state: join(temp, "state"), runtimeAudit: join(temp, "find-audit.jsonl"), audit: join(root, "find-audit.jsonl"), transcript: join(root, "codex-events.jsonl") };
  } catch (error) {
    cleanupRuntime(temp);
    throw error;
  }
}

function freezeSuite(manifest, options) {
  const sourceIdentity = repositoryIdentity(); if (!sourceIdentity.clean) fail("formal suite requires a clean repository worktree");
  const root = mkdtempSync(join(options.runsDir, "suite-snapshot-")); const targets = join(root, "targets"); mkdirSync(targets);
  for (const name of new Set(manifest.tasks.map((task) => task.expected_skill))) cpSync(join(options.skillsRoot, name), join(targets, name), { recursive: true, dereference: true });
  const targetPackages = Object.fromEntries([...new Set(manifest.tasks.map((task) => task.expected_skill))].map((name) => { const packageRoot = join(targets, name); const script = join(packageRoot, "bin", "archify.mjs"); return [name, { package_tree_sha256: stateDigest(packageRoot), script_content_sha256: existsSync(script) ? sha(readFileSync(script)) : null }]; }));
  const bootstrapRoot = join(root, "bootstrap"); cpSync(dirname(options.bootstrap), bootstrapRoot, { recursive: true, dereference: true });
  const cli = join(root, basename(options.cli)); copyFileSync(options.cli, cli); chmodSync(cli, statSync(options.cli).mode & 0o777);
  const manifestPath = join(root, "manifest.json"); copyFileSync(options.manifest, manifestPath);
  const version = run(options.codex, ["--version"], { encoding: "utf8", timeout: 30_000 });
  if (version.status !== 0 || !version.stdout.trim()) fail("unable to freeze Codex version identity");
  const codexExecutable = executableIdentity(options.codex);
  const trialsPerArm = manifest.trials_per_arm ?? 1;
  const driverSha256 = sha(readFileSync(fileURLToPath(import.meta.url))); const frozenTreeDigest = stateDigest(root); const realNode = canonicalPath(process.execPath); const parentVerifierIdentity = sha(`${realNode}\0${driverSha256}`);
  const executionContract = {
    model: options.model, reasoning_effort: options.reasoningEffort, timeout_ms: options.timeoutMs,
    sandbox: "workspace-write", ephemeral: true, ignore_user_config: true,
    preflight_contract: {
      command: "debug prompt-input", shared_model_config: true, fresh_codex_home_without_config: true,
      isolated_non_repository_workspace: true,
      execution_only_flags_unsupported_by_debug: ["ignore_user_config", "sandbox", "ephemeral"],
    },
    sandbox_policy: { workspace_write: { exclude_tmpdir_env_var: true, exclude_slash_tmp: true, add_dir: "unique_run_temp" } },
    ...(manifest.evaluation_mode === "on_demand_transfer" ? { evaluation_mode: manifest.evaluation_mode } : {}),
    arm_schedule: manifest.evaluation_mode === "on_demand_transfer" ? ["on_demand"] : ["core", "on_demand"], trials_per_arm: trialsPerArm,
    family_schedule: manifest.tasks.map((task) => ({ family: task.family, task: task.id })),
  };
  const snapshotDigest = sha(`${frozenTreeDigest}\0${JSON.stringify(executionContract)}\0${JSON.stringify(sourceIdentity)}\0${JSON.stringify(codexExecutable)}\0${driverSha256}\0${parentVerifierIdentity}`);
  const pairInvariants = {};
  for (const task of manifest.tasks) for (let trial = 1; trial <= trialsPerArm; trial += 1) pairInvariants[trialKey(task.id, trial, trialsPerArm)] = pairInvariant(snapshotDigest, task, trial);
  const facts = {
    snapshot_digest: snapshotDigest, manifest_sha256: sha(readFileSync(manifestPath)), cli_sha256: sha(readFileSync(cli)),
    bootstrap_sha256: stateDigest(bootstrapRoot), targets_sha256: stateDigest(targets), codex_version: version.stdout.trim(), codex_version_sha256: sha(version.stdout.trim()), codex_executable: codexExecutable, source_identity: sourceIdentity, execution_contract: executionContract, model: options.model, reasoning_effort: options.reasoningEffort, driver_sha256: driverSha256,
    real_node_sha256: sha(realNode), parent_verifier_identity_sha256: parentVerifierIdentity,
    target_packages: targetPackages, trials_per_arm: trialsPerArm, pair_invariants: pairInvariants,
  };
  return { options: { ...options, manifest: manifestPath, skillsRoot: targets, bootstrap: join(bootstrapRoot, basename(options.bootstrap)), cli }, facts };
}

export function codexSandboxPolicy(tempRoot) {
  return { exclude_tmpdir_env_var: true, exclude_slash_tmp: true, add_dirs: [canonicalPath(tempRoot)] };
}

export function codexExecutionArgs(options, paths, prompt) {
  const policy = codexSandboxPolicy(paths.temp);
  return ["exec", "--ephemeral", "--ignore-user-config", "--sandbox", "workspace-write", "--add-dir", policy.add_dirs[0], "--skip-git-repo-check", "--json", "-c", "sandbox_workspace_write.exclude_tmpdir_env_var=true", "-c", "sandbox_workspace_write.exclude_slash_tmp=true", ...codexModelConfig(options), "-C", paths.workspace, prompt];
}

function codexModelConfig(options) { return ["-c", `model=${JSON.stringify(options.model)}`, "-c", `model_reasoning_effort=${JSON.stringify(options.reasoningEffort)}`]; }

function preflightSurface(paths, task, arm, options, env) {
  const result = run(options.codex, ["-C", paths.workspace, "debug", "prompt-input", ...codexModelConfig(options), task.prompt], { cwd: paths.workspace, env, timeout: 30_000 });
  if (result.status !== 0) fail(`Codex prompt-input preflight failed: ${result.stderr.trim()}`);
  const visible = extractVisibleSkills(result.stdout); const assessment = assessSkillSurface(visible, arm, task.expected_skill);
  return { ...assessment, prompt_input_sha256: sha(result.stdout) };
}

export function prepareOnDemand(paths, task, options, env) {
  const sourceRoot = join(dirname(dirname(paths.targetPath)));
  const common = ["--home", paths.home, "--state-dir", paths.state, "--json", "--source-root", sourceRoot];
  const invoke = (command, args = [], input = undefined) => {
    const result = run(options.cli, [...common, command, ...args], { env, timeout: 60_000, input });
    if (result.status !== 0) fail(`SkillRoster ${command} failed: ${result.stderr.trim() || result.stdout.trim()}`);
    return parseEnvelope(result.stdout, command);
  };
  const scan = invoke("scan"); const report = invoke("report", ["--full"]); const canonicalTarget = canonicalPath(paths.targetPath);
  let targetEvidence = null;
  for (const finding of report.result.findings ?? []) {
    const detail = invoke("report", ["--finding", finding.id, "--full"]);
    targetEvidence = (detail.result.evidence ?? []).find((evidence) => evidence.path && canonicalPath(evidence.path) === canonicalTarget) ?? null;
    if (targetEvidence) break;
  }
  const skillId = targetEvidence?.details?.skill_id;
  if (!skillId || targetEvidence.details?.default_exposed !== false) fail("target source evidence is missing or unexpectedly exposed before governance");
  const planInput = JSON.stringify({ schema_version: 1, scan_id: scan.result.snapshot_id, evidence_ids: [targetEvidence.id], roster_changes: [{ agent: "codex", skill_id: skillId, state: "on_demand" }] });
  const plan = invoke("plan", ["--stdin"], planInput); const applied = invoke("apply", [plan.result.plan_id]);
  if (applied.result.verification !== "passed" || !applied.result.receipt_id) fail("SkillRoster Apply did not produce a verified Receipt");
  invoke("scan"); invoke("report", ["--full"]);
  const findResult = run(options.cli, skillRosterFindArgs(paths, task), { env, timeout: 30_000 });
  if (findResult.status !== 0) fail(`SkillRoster Find preflight failed: ${findResult.stderr.trim()}`);
  const top = parseFindEnvelope(findResult.stdout); const status = invoke("status");
  if (top.skill !== task.expected_skill || !top.path || canonicalPath(top.path) !== canonicalTarget || top.roster_state !== "on_demand" || top.agents.length !== 0) fail("governed Find did not return one exact, non-exposed On-demand target");
  if (task.one_call_load && !verifiedFindLoad(findResult.stdout, paths.targetPath)) fail("governed Find did not return one exact verified loaded Skill");
  if (status.result.recovery_state !== "clear" || status.result.journal_issues?.length || status.result.last_receipt?.receipt_id !== applied.result.receipt_id) fail("SkillRoster recovery or Receipt state is not clear");
  const bin = join(dirname(paths.audit), "bin"); mkdirSync(bin, { recursive: true }); const wrapper = join(bin, "skillroster"); writeFileSync(wrapper, findWrapperSource(task.one_call_load === true), { mode: 0o755 }); chmodSync(wrapper, 0o755);
  return { bin, governance: { roster_state: top.roster_state, target_default_exposure: 0, receipt_id: applied.result.receipt_id, receipt_verification: applied.result.verification, recovery_state: status.result.recovery_state } };
}

function trialKey(taskId, trial, trialsPerArm) { return trialsPerArm === 1 ? taskId : `${taskId}#${trial}`; }

export function pairInvariant(snapshotDigest, task, trial = 1) { return sha(`${snapshotDigest}\0${JSON.stringify(task)}\0trial:${trial}`); }

export function formalResultEligible(result) {
  return Boolean(result.pair_invariant
    && result.codex_exit_code === 0
    && result.surface?.passed === true
    && result.transcript?.passed === true
    && result.workspace?.passed === true
    && result.protected_scopes?.passed === true
    && result.outcome?.harness_valid === true
    && result.outcome?.safety === "passed");
}

export function formalOnDemandTransferResultEligible(result) {
  return formalResultEligible(result)
    && result.arm === "on_demand"
    && TRANSFER_STAGES.includes(result.outcome?.deepest_stage)
    && transferEvidenceFieldsValid(result.outcome);
}

function transferEvidenceFieldsValid(outcome) {
  return typeof outcome?.task_succeeded_without_verified_skill_bytes === "boolean"
    && typeof outcome?.task_succeeded_without_accepted_one_call_load_contract === "boolean"
    && !(outcome.task_succeeded_without_verified_skill_bytes && !outcome.task_succeeded_without_accepted_one_call_load_contract)
    && !(outcome.deepest_stage === "task_succeeded" && outcome.task_succeeded_without_accepted_one_call_load_contract)
    && !(outcome.accepted === true && (outcome.deepest_stage !== "task_succeeded" || outcome.task_succeeded_without_verified_skill_bytes));
}

export function formalOnDemandTransferGateEligible({ completeSchedule, sourceIdentityStable, codexExecutableStable, frozenInputsStable, results }) {
  return completeSchedule === true
    && sourceIdentityStable === true
    && codexExecutableStable === true
    && frozenInputsStable === true
    && results.every(formalOnDemandTransferResultEligible);
}

function executeArm(task, arm, trial, trialsPerArm, options, suiteFacts, evaluationMode = "paired") {
  const key = trialKey(task.id, trial, trialsPerArm); const frozenPairInvariant = suiteFacts.pair_invariants?.[key]; if (!frozenPairInvariant) fail(`pair invariant was not frozen before invocation: ${key}`);
  const runPrefix = trialsPerArm === 1 ? `${task.id}-${arm}-` : `${task.id}-trial-${trial}-${arm}-`;
  const root = mkdtempSync(join(options.runsDir, runPrefix)); const paths = setupArm(root, task, arm, options);
  const authCopy = copyAuth(options.authSource, paths.codexHome);
  const env = { ...process.env, HOME: paths.home, CODEX_HOME: paths.codexHome, TMPDIR: paths.temp };
  const protectedPaths = { targetPackage: dirname(paths.targetPath), exposedPackage: join(paths.codexHome, "skills", arm === "core" ? task.expected_skill : "skillroster"), authCopy };
  const initialWorkspace = walk(paths.workspace); const initialProtected = captureProtectedScopes(protectedPaths);
  try {
    const surface = preflightSurface(paths, task, arm, options, env);
    if (!surface.passed) {
      const workspace = assessWorkspaceChanges(initialWorkspace, walk(paths.workspace), Object.keys(task.workspace_files), task.allowed_changed_paths); const protectedScopes = assessProtectedScopes(initialProtected, captureProtectedScopes(protectedPaths));
      const externalWrites = assessExternalWrites("", paths.workspace, paths.temp);
      const safety = workspace.passed && protectedScopes.passed && externalWrites.passed ? "passed" : "failed";
      const outcome = evaluationMode === "on_demand_transfer"
        ? { harness_valid: false, safety, accepted: false, deepest_stage: "no_retrieval_call", task_succeeded_without_verified_skill_bytes: false, task_succeeded_without_accepted_one_call_load_contract: false }
        : { harness_valid: false, safety, accepted: false };
      return { family: task.family, task: task.id, trial, arm, surface, workspace, protected_scopes: protectedScopes, external_writes: externalWrites, outcome, root };
    }
    let prepared = null;
    if (arm === "on_demand") prepared = prepareOnDemand(paths, task, options, env);
    const runEnv = { ...env };
    if (prepared) Object.assign(runEnv, { SKILLROSTER_REAL_CLI: options.cli, SKILLROSTER_TEST_HOME: paths.home, SKILLROSTER_TEST_STATE: paths.state, SKILLROSTER_FIND_AUDIT: paths.runtimeAudit });
    if (prepared) runEnv.PATH = `${prepared.bin}:${process.env.PATH ?? "/usr/bin:/bin"}`;
    const result = run(options.codex, codexExecutionArgs(options, paths, task.prompt), { cwd: paths.workspace, env: runEnv, timeout: options.timeoutMs });
    const protectedScopes = assessProtectedScopes(initialProtected, captureProtectedScopes(protectedPaths));
    writeFileSync(paths.transcript, result.stdout, { mode: 0o600 });
    if (existsSync(paths.runtimeAudit)) copyFileSync(paths.runtimeAudit, paths.audit);
    const after = walk(paths.workspace); const workspace = assessWorkspaceChanges(initialWorkspace, after, Object.keys(task.workspace_files), task.allowed_changed_paths);
    const oracle = result.status === 0 ? evaluateOracle(paths.workspace, task.oracle, { transcript: result.stdout, targetPackage: dirname(paths.targetPath), parentVerificationAuthority: { protected_scopes_passed: protectedScopes.passed, expected: suiteFacts.target_packages?.[task.expected_skill] } }) : { passed: false, failures: [`codex_exit:${result.status ?? "signal"}`] };
    const retrieval = arm === "on_demand" ? parseFindAudit(existsSync(paths.audit) ? readFileSync(paths.audit, "utf8") : "", { task: task.prompt, skill: task.expected_skill, path: paths.targetPath, oneCall: task.one_call_load === true }) : { count: 0, contract_violation: false };
    if (arm === "on_demand" && task.required_find_calls !== undefined && retrieval.count !== task.required_find_calls) retrieval.contract_violation = true;
    const load = arm === "on_demand" && task.one_call_load ? assessOneCallLoad(result.stdout, paths.targetPath) : assessExactLoad(result.stdout, paths.targetPath);
    const routeOrder = arm === "on_demand" ? assessRouteOrder(result.stdout, { bootstrapPath: join(paths.codexHome, "skills", "skillroster", "SKILL.md"), targetPath: paths.targetPath, findAudit: retrieval, expectedTask: task.prompt, expectedSkill: task.expected_skill, oneCall: task.one_call_load === true }) : { passed: true, audit_scope: "not_applicable" };
    const coreOrder = arm === "core" ? assessCoreOrder(result.stdout, paths.targetPath) : { passed: true, audit_scope: "not_applicable" };
    const transcript = assessTranscriptIntegrity(result.stdout);
    const externalWrites = assessExternalWrites(result.stdout, paths.workspace, paths.temp);
    const outcomeInputs = { arm, surface, retrieval, load, oracle, workspace, routeOrder, coreOrder, transcript, protectedScopes, externalWrites };
    const outcome = evaluationMode === "on_demand_transfer" ? deriveOnDemandTransferOutcome(outcomeInputs) : deriveArmOutcome(outcomeInputs);
    return { family: task.family, task: task.id, trial, arm, root, pair_invariant: frozenPairInvariant, codex_exit_code: result.status, surface, governance: prepared?.governance ?? null, transcript, protected_scopes: protectedScopes, external_writes: externalWrites, retrieval, load, route_order: routeOrder, core_order: coreOrder, oracle, workspace, outcome };
  } finally {
    cleanupAuth(authCopy);
    cleanupRuntime(paths.temp);
  }
}

function dryRun(manifest, options) {
  const tasks = options.task === "all" ? manifest.tasks : manifest.tasks.filter((task) => task.id === options.task);
  if (!tasks.length) fail(`unknown task: ${options.task}`);
  const arms = manifest.evaluation_mode === "on_demand_transfer" ? ["on_demand"] : options.arm === "both" ? ["core", "on_demand"] : [options.arm];
  const trialsPerArm = manifest.trials_per_arm ?? 1;
  return { status: "planned", execute: false, suite_id: manifest.suite_id, model: options.model, reasoning_effort: options.reasoningEffort, task_count: tasks.length, trials_per_arm: trialsPerArm, arms, run_count: tasks.length * arms.length * trialsPerArm, note: "Pass --execute and an explicit --auth-source to invoke Codex." };
}

export function reevaluateExistingRuns(suiteRoot, manifest) {
  const sourceDigestBefore = stateDigest(suiteRoot); const results = []; const trialsPerArm = manifest.trials_per_arm ?? 1;
  for (const task of manifest.tasks) for (let trial = 1; trial <= trialsPerArm; trial += 1) for (const arm of ["core", "on_demand"]) {
    const prefix = trialsPerArm === 1 ? `${task.id}-${arm}-` : `${task.id}-trial-${trial}-${arm}-`;
    const matches = readdirSync(suiteRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory() && entry.name.startsWith(prefix));
    if (matches.length !== 1) { results.push({ family: task.family, task: task.id, trial, arm, root: null, recomputed: false, formal_evidence_accepted: null, post_hoc_only: true, failures: [`run_root_count:${matches.length}`] }); continue; }
    const root = join(suiteRoot, matches[0].name); const workspacePath = join(root, "workspace"); const transcriptPath = join(root, "codex-events.jsonl");
    if (!existsSync(workspacePath) || !existsSync(transcriptPath)) { results.push({ family: task.family, task: task.id, trial, arm, root, recomputed: false, formal_evidence_accepted: null, post_hoc_only: true, failures: ["workspace_or_transcript_missing"] }); continue; }
    const transcriptText = readFileSync(transcriptPath, "utf8"); const targetPath = arm === "core" ? join(root, "home", ".codex", "skills", task.expected_skill, "SKILL.md") : join(root, "source", task.expected_skill, "SKILL.md");
    const initial = new Map(Object.entries(task.workspace_files).map(([path, content]) => [path, sha(content)])); const workspace = assessWorkspaceChanges(initial, walk(workspacePath), Object.keys(task.workspace_files), task.allowed_changed_paths);
    const retrieval = arm === "on_demand" ? parseFindAudit(existsSync(join(root, "find-audit.jsonl")) ? readFileSync(join(root, "find-audit.jsonl"), "utf8") : "", { task: task.prompt, skill: task.expected_skill, path: targetPath, oneCall: task.one_call_load === true }) : { count: 0, contract_violation: false };
    if (arm === "on_demand" && task.required_find_calls !== undefined && retrieval.count !== task.required_find_calls) retrieval.contract_violation = true;
    const load = arm === "on_demand" && task.one_call_load ? assessOneCallLoad(transcriptText, targetPath) : assessExactLoad(transcriptText, targetPath); const transcript = assessTranscriptIntegrity(transcriptText);
    const routeOrder = arm === "on_demand" ? assessRouteOrder(transcriptText, { bootstrapPath: join(root, "home", ".codex", "skills", "skillroster", "SKILL.md"), targetPath, findAudit: retrieval, expectedTask: task.prompt, expectedSkill: task.expected_skill, oneCall: task.one_call_load === true }) : { passed: true, audit_scope: "not_applicable" };
    const coreOrder = arm === "core" ? assessCoreOrder(transcriptText, targetPath) : { passed: true, audit_scope: "not_applicable" };
    const oracle = evaluateOracle(workspacePath, task.oracle, { transcript: transcriptText, targetPackage: dirname(targetPath), parentVerificationAuthority: { mode: "historical_untrusted" } });
    results.push({ family: task.family, task: task.id, trial, arm, root, recomputed: true, formal_evidence_accepted: null, post_hoc_only: true, recomputed_dimensions: { transcript: transcript.passed, retrieval: arm === "core" ? null : Boolean(retrieval.top1_correct && retrieval.returned_path_exact && !retrieval.contract_violation), load: load.passed, route_order: arm === "core" ? null : routeOrder.passed, core_order: arm === "core" ? coreOrder.passed : null, oracle: oracle.passed, parent_correctness: task.oracle.archify_receipt_contract ? oracle.archify_parent_verification?.passed ?? null : null, workspace: workspace.passed }, transcript, retrieval, load, route_order: routeOrder, core_order: coreOrder, oracle, workspace, historical_evidence_limitations: ["post-hoc result is not a formal gate", "model-visible surface, earliest protected-scope baselines, and original suite ledger were not persisted"] });
  }
  const sourceDigestAfter = stateDigest(suiteRoot); if (sourceDigestAfter !== sourceDigestBefore) fail("reevaluation changed the source run tree");
  return { status: "reevaluated", source_root: canonicalPath(suiteRoot), source_tree_sha256_before: sourceDigestBefore, source_tree_sha256_after: sourceDigestAfter, raw_runs_modified: false, formal_gate_eligible: false, results };
}

export function main(argv = process.argv.slice(2)) {
  const options = parseArgs(argv);
  const manifest = validateManifest(JSON.parse(readFileSync(options.manifest, "utf8")));
  if (options.reevaluateRoot) {
    if (!existsSync(options.reevaluateRoot) || !statSync(options.reevaluateRoot).isDirectory()) fail("--reevaluate-root must be an existing directory");
    const canonicalSource = realpathSync(options.reevaluateRoot); const output = options.reevaluateOutput ?? join(dirname(canonicalSource), `${basename(canonicalSource)}-reevaluated-${Date.now()}.json`);
    if (inside(resolve(output), canonicalSource) || existingAncestorResolvesInside(output, canonicalSource)) fail("reevaluation output must stay outside the source run root");
    if (existsSync(output)) fail("reevaluation output already exists"); const summary = reevaluateExistingRuns(canonicalSource, manifest); writeFileSync(output, `${JSON.stringify(summary, null, 2)}\n`, { mode: 0o600 });
    process.stdout.write(`${JSON.stringify({ status: summary.status, formal_gate_eligible: false, output, results: summary.results.map(({ task, arm, formal_evidence_accepted, recomputed_dimensions }) => ({ task, arm, formal_evidence_accepted, post_hoc_only: true, recomputed_dimensions })) }, null, 2)}\n`); return 0;
  }
  if (inside(options.runsDir, REPO)) fail("--runs-dir must stay outside the repository so transcripts cannot be committed");
  if (existingAncestorResolvesInside(options.runsDir, REPO)) fail("--runs-dir ancestor resolves inside the repository so transcripts cannot be committed");
  mkdirSync(options.runsDir, { recursive: true });
  if (inside(realpathSync(options.runsDir), realpathSync(REPO))) fail("--runs-dir resolves inside the repository so transcripts cannot be committed");
  validateSummaryOutput(options.summaryOutput, options.runsDir);
  const evaluationMode = manifest.evaluation_mode ?? "paired";
  if (evaluationMode === "on_demand_transfer" && (options.task !== "all" || (options.armExplicit && options.arm !== "on_demand"))) fail("on_demand_transfer formal gate requires the complete On-demand schedule");
  if (evaluationMode === "on_demand_transfer" && (options.model !== manifest.formal_schedule.model || options.reasoningEffort !== manifest.formal_schedule.reasoning_effort || options.timeoutMs !== manifest.formal_schedule.timeout_ms)) fail("on_demand_transfer execution parameters must match the frozen formal schedule");
  if (!options.execute) { emitSummary(dryRun(manifest, options), options); return 0; }
  if (manifest.formal_protocol_gate === true && evaluationMode === "paired" && (options.task !== "all" || options.arm !== "both")) fail("formal protocol gate requires the complete task and arm schedule");
  for (const path of [options.bootstrap, options.cli, options.skillsRoot, options.authSource]) if (!existsSync(path)) fail(`required path is missing: ${path}`);
  const frozen = freezeSuite(manifest, options); const runOptions = frozen.options;
  const tasks = options.task === "all" ? manifest.tasks : manifest.tasks.filter((task) => task.id === options.task); if (!tasks.length) fail(`unknown task: ${options.task}`);
  const arms = evaluationMode === "on_demand_transfer" ? ["on_demand"] : options.arm === "both" ? ["core", "on_demand"] : [options.arm]; const results = []; const trialsPerArm = manifest.trials_per_arm ?? 1;
  for (const task of tasks) for (let trial = 1; trial <= trialsPerArm; trial += 1) for (const arm of arms) results.push(executeArm(task, arm, trial, trialsPerArm, runOptions, frozen.facts, evaluationMode));
  const pairs = evaluationMode === "paired" ? groupPairedResults(tasks, results, trialsPerArm) : [];
  const expectedArmCount = evaluationMode === "on_demand_transfer" ? 1 : 2;
  const completeSchedule = tasks.length === manifest.tasks.length && arms.length === expectedArmCount && results.length === manifest.tasks.length * trialsPerArm * expectedArmCount;
  const sourceIdentityAfter = repositoryIdentity(); const sourceIdentityStable = JSON.stringify(sourceIdentityAfter) === JSON.stringify(frozen.facts.source_identity);
  const codexExecutableAfter = executableIdentity(options.codex); const codexExecutableStable = JSON.stringify(codexExecutableAfter) === JSON.stringify(frozen.facts.codex_executable);
  let frozenInputsAfter = null; let frozenInputsStable = true;
  if (evaluationMode === "on_demand_transfer") {
    const frozenInputAssessment = assessFrozenInputIdentities(frozen.facts, runOptions);
    frozenInputsAfter = frozenInputAssessment.actual; frozenInputsStable = frozenInputAssessment.passed;
  }
  const protocolDecision = manifest.formal_protocol_gate !== true ? null : evaluationMode === "on_demand_transfer" ? deriveOnDemandTransferDecision(results, trialsPerArm, tasks, manifest.stop_decision) : deriveProtocolDecision(results, trialsPerArm, tasks, pairs);
  const resultEligibility = evaluationMode === "on_demand_transfer" ? formalOnDemandTransferResultEligible : formalResultEligible;
  const formalGateEligible = evaluationMode === "on_demand_transfer"
    ? formalOnDemandTransferGateEligible({ completeSchedule: protocolDecision.complete_schedule, sourceIdentityStable, codexExecutableStable, frozenInputsStable, results })
    : completeSchedule && sourceIdentityStable && codexExecutableStable && results.every(resultEligibility) && pairs.every((pair) => pair.attribution !== "pair_invariant_mismatch" && pair.attribution !== "pair_incomplete");
  const acceptedResults = results.every((result) => result.outcome?.accepted) && pairs.every((pair) => pair.gate === "passed");
  const statusPassed = acceptedResults && (evaluationMode !== "on_demand_transfer" || formalGateEligible);
  const summary = { status: statusPassed ? "passed" : "failed", suite_id: manifest.suite_id, ...(evaluationMode === "on_demand_transfer" ? { evaluation_mode: evaluationMode } : {}), formal_gate_eligible: formalGateEligible, source_identity_stable: sourceIdentityStable, source_identity_after: sourceIdentityAfter, codex_executable_stable: codexExecutableStable, codex_executable_after: codexExecutableAfter, ...(evaluationMode === "on_demand_transfer" ? { frozen_inputs_stable: frozenInputsStable, frozen_inputs_after: frozenInputsAfter } : {}), protocol_decision: protocolDecision, suite_snapshot: frozen.facts, signal_cleanup: "SIGINT/SIGTERM remove auth copies and runtime state best effort; SIGKILL cannot guarantee cleanup", results, pairs };
  emitSummary(summary, options); return summary.status === "passed" ? 0 : 2;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = main(); } catch (error) { process.stderr.write(`codex transfer harness: ${error.message}\n`); process.exitCode = 1; }
}
