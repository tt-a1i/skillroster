#!/usr/bin/env node

import { accessSync, chmodSync, constants, existsSync, lstatSync, mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { createHash, randomBytes } from "node:crypto";
import { homedir, tmpdir } from "node:os";
import { basename, delimiter, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { boundedFileSha256, boundedReadFile, boundedWriteFile, copyBoundedFile, copyBoundedTree, createReadBudget, GATE_LEDGER_MAX_BYTES, HARNESS_IO_LIMITS, walkBoundedTree } from "./bounds.mjs";

export function modulePathFromUrl(url) { return fileURLToPath(url); }
const RUNNER_PATH = modulePathFromUrl(import.meta.url);
const HERE = dirname(RUNNER_PATH);
const REPO = resolve(HERE, "../../..");
const ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,79}$/u;
const CLI_TIMEOUT_MS = 30_000;
const OFFICIAL_SUITE_POLICIES = new Map([
  ["cold-routing-training-v10", { task_count: 4, model: "seal/gpt-5.6-sol", seal_contract: null, arm_schedule_seed: "7b1e09a46d2c8f3054ea91b763cd20af", gate: { core_task_success_minimum: 4, on_demand_task_success_minimum: 3, on_demand_load_success_minimum: 3 } }],
  ["cold-routing-holdout-v2", { task_count: 4, model: "seal/gpt-5.6-sol", seal_contract: "pi-cold-routing-holdout-v2.seal.json", arm_schedule_seed: "4d9f318a6c72e501b8d4430faec96725", gate: { core_task_success_minimum: 4, on_demand_task_success_minimum: 4, on_demand_load_success_minimum: 3 } }],
]);

function fail(message) { throw new Error(message); }
function sha(value) { return createHash("sha256").update(value).digest("hex"); }
function fileSha(path, options = {}) { return boundedFileSha256(path, options); }
function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}
export function sealPayloadDigest(sourceRevision, facts) { return sha(canonicalJson({ source_revision: sourceRevision, facts })); }
function within(candidate, root) {
  const suffix = relative(root, candidate);
  return suffix === "" || (!suffix.startsWith(`..${sep}`) && suffix !== ".." && !isAbsolute(suffix));
}
function safeRelativePath(path, label = "path") {
  if (typeof path !== "string" || !path || isAbsolute(path)) fail(`${label} must be a relative path`);
  const parts = path.split(/[\\/]/u);
  if (parts.some((part) => !part || part === "." || part === "..")) fail(`${label} contains an unsafe segment`);
  return parts.join("/");
}
function safeDestination(root, path, label) {
  const destination = resolve(root, safeRelativePath(path, label));
  if (!within(destination, resolve(root))) fail(`${label} escapes its root`);
  return destination;
}
function canonicalInside(path, root, label = "source") {
  const canonicalRoot = realpathSync(root);
  const canonical = realpathSync(path);
  if (!within(canonical, canonicalRoot)) fail(`${label} escapes its root`);
  return canonical;
}

export function parseArgs(argv) {
  const options = {
    manifest: join(REPO, "tests/fixtures/pi-cold-routing-training.json"), task: "all", arm: "both",
    skillsRoot: join(homedir(), ".agents_skills"), bootstrap: join(REPO, "skill/skillroster/SKILL.md"),
    cli: join(REPO, "target/debug/skillroster"), pi: "pi", piConfigSource: join(homedir(), ".pi/agent"),
    runsDir: join(tmpdir(), "skillroster-pi-cold-routing"), timeoutMs: 300_000, timeoutOverridden: false, diagnostic: false, generateSeal: null,
  };
  const keys = new Map([["--manifest", "manifest"], ["--task", "task"], ["--arm", "arm"], ["--skills-root", "skillsRoot"], ["--bootstrap", "bootstrap"], ["--cli", "cli"], ["--pi", "pi"], ["--pi-config-source", "piConfigSource"], ["--runs-dir", "runsDir"], ["--timeout-ms", "timeoutMs"], ["--generate-seal", "generateSeal"]]);
  for (let index = 0; index < argv.length;) {
    if (argv[index] === "--diagnostic") { options.diagnostic = true; index += 1; continue; }
    const key = keys.get(argv[index]); const value = argv[index + 1];
    if (!key || !value) fail(`unknown or incomplete argument: ${argv[index] ?? "<missing>"}`);
    options[key] = key === "timeoutMs" ? Number(value) : ["task", "arm", "pi"].includes(key) ? value : resolve(value);
    if (key === "timeoutMs") options.timeoutOverridden = true;
    index += 2;
  }
  if (!["core", "on_demand", "both"].includes(options.arm)) fail("--arm must be core, on_demand, or both");
  if (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs < 1_000 || options.timeoutMs > 900_000) fail("--timeout-ms must be between 1000 and 900000");
  return options;
}

export function validateTimeoutOverride(options, manifest) {
  if (options.timeoutOverridden && OFFICIAL_SUITE_POLICIES.has(manifest.suite_id) && !options.diagnostic) fail("official formal suites forbid --timeout-ms; use the frozen task/default timeout");
  return { formal_eligible: OFFICIAL_SUITE_POLICIES.has(manifest.suite_id) && !options.diagnostic && !options.timeoutOverridden };
}

export function validateManifest(manifest) {
  if (manifest?.schema_version !== 1 || manifest?.harness !== "pi" || !ID_PATTERN.test(manifest.suite_id ?? "")) fail("unsupported manifest identity");
  if (!Array.isArray(manifest.tasks) || !manifest.tasks.length) fail("manifest tasks are required");
  const gate = manifest.aggregate_gate;
  for (const field of ["core_task_success_minimum", "on_demand_task_success_minimum", "on_demand_load_success_minimum"]) {
    if (!Number.isSafeInteger(gate?.[field]) || gate[field] < 1 || gate[field] > manifest.tasks.length) fail(`aggregate_gate.${field} must be between 1 and task count`);
  }
  if (gate.core_task_success_minimum !== manifest.tasks.length) fail("aggregate_gate.core_task_success_minimum must require every Core task");
  const officialPolicy = OFFICIAL_SUITE_POLICIES.get(manifest.suite_id);
  if (officialPolicy) {
    if (manifest.tasks.length !== officialPolicy.task_count || manifest.model !== officialPolicy.model || (manifest.seal_contract ?? null) !== officialPolicy.seal_contract || manifest.arm_schedule_seed !== officialPolicy.arm_schedule_seed || Object.entries(officialPolicy.gate).some(([field, value]) => gate[field] !== value) || Object.keys(gate).some((field) => !(field in officialPolicy.gate))) fail(`${manifest.suite_id} does not match its frozen suite policy`);
  } else if (!manifest.suite_id.startsWith("test-fixture-")) fail("unknown suite_id has no frozen suite policy");
  if (manifest.suite_id === "cold-routing-holdout-v2") {
    safeRelativePath(manifest.seal_contract, "holdout seal contract");
    if (!/^[a-f0-9]{32}$/u.test(manifest.arm_schedule_seed ?? "")) fail("sealed holdout requires a fixed 128-bit arm_schedule_seed");
  }
  else if (manifest.seal_contract !== undefined) fail("seal_contract is only valid for the sealed holdout suite");
  if (!Array.isArray(manifest.common?.tools) || manifest.common.tools.some((tool) => !["read", "write", "edit", "bash"].includes(tool))) fail("manifest tools exceed the gated surface");
  if (!Array.isArray(manifest.common?.forbidden_prompt_terms) || manifest.common.forbidden_prompt_terms.some((term) => typeof term !== "string" || !term)) fail("forbidden prompt terms are required");
  const ids = new Set();
  for (const task of manifest.tasks) {
    if (!ID_PATTERN.test(task.id ?? "") || ids.has(task.id)) fail("task id must be unique and path-safe");
    ids.add(task.id);
    if (!ID_PATTERN.test(task.expected_skill ?? "")) fail(`${task.id} expected_skill is unsafe`);
    if (typeof task.prompt !== "string" || !task.prompt) fail(`${task.id} prompt is required`);
    const forbiddenPromptTerm = promptContainsForbiddenTerm(task.prompt, [...manifest.common.forbidden_prompt_terms, task.expected_skill]); if (forbiddenPromptTerm) fail(`${task.id} leaks forbidden prompt identity: ${forbiddenPromptTerm}`);
    if (task.timeout_ms !== undefined && (!Number.isSafeInteger(task.timeout_ms) || task.timeout_ms < 1_000 || task.timeout_ms > 900_000)) fail(`${task.id} timeout_ms must be between 1000 and 900000`);
    planWorkspaceInputs(task);
    for (const path of task.target_package_include ?? []) safeRelativePath(path, `${task.id} package include`);
    if (!Array.isArray(task.allowed_changed_paths)) fail(`${task.id} allowed_changed_paths is required`);
    for (const path of task.allowed_changed_paths ?? []) safeRelativePath(path, `${task.id} allowed changed path`);
    if (new Set(task.allowed_changed_paths).size !== task.allowed_changed_paths.length) fail(`${task.id} allowed changed paths must be unique`);
    if (task.allowed_changed_paths.some((path) => Object.hasOwn(task.workspace_files ?? {}, path))) fail(`${task.id} cannot allow mutation of an input path`);
    for (const path of task.contained_write_roots ?? []) safeRelativePath(path, `${task.id} contained write root`);
    for (const root of [...(task.post_load_permissions?.read_roots ?? []), ...(task.post_load_permissions?.write_roots ?? [])]) if (!["workspace", "target_package"].includes(root)) fail(`${task.id} permission root is unsupported`);
    for (const field of ["path", "private_path", "report_path", "required_paths", "required_nonempty_paths", "public_seed_paths", "forbidden_paths"]) {
      const value = task.oracle?.[field];
      for (const path of Array.isArray(value) ? value : value ? [value] : []) safeRelativePath(path, `${task.id} oracle ${field}`);
    }
    const allowed = new Set(task.allowed_changed_paths); const oraclePaths = [task.oracle?.path, task.oracle?.private_path, task.oracle?.report_path, ...(task.oracle?.required_paths ?? [])].filter(Boolean);
    if (oraclePaths.some((path) => !allowed.has(path))) fail(`${task.id} oracle outputs must be exactly allowlisted`);
    if ((task.oracle?.required_nonempty_paths ?? []).some((path) => !(task.oracle?.required_paths ?? []).includes(path))) fail(`${task.id} nonempty outputs must also be required`);
    if ((task.oracle?.public_seed_paths ?? []).some((path) => !(task.oracle?.required_paths ?? []).includes(path))) fail(`${task.id} public outputs must also be required`);
    if ((task.oracle?.forbidden_paths ?? []).some((path) => allowed.has(path))) fail(`${task.id} forbidden and allowed outputs overlap`);
    if (task.allowed_changed_paths.length > 0 && !(task.post_load_permissions?.write_roots ?? []).includes("workspace")) fail(`${task.id} workspace outputs require workspace write permission`);
    for (const field of ["required_regex", "forbidden_regex", "public_required_regex", "private_required_regex", "report_required_regex"]) {
      const values = task.oracle?.[field] ?? [];
      if (!Array.isArray(values) || values.some((value) => typeof value !== "string" || !value)) fail(`${task.id} oracle ${field} must contain strings`);
      for (const value of values) compileRegex(value);
    }
    if (task.oracle?.deprecated_terms !== undefined && (!Array.isArray(task.oracle.deprecated_terms) || task.oracle.deprecated_terms.some((term) => typeof term !== "string" || !term) || new Set(task.oracle.deprecated_terms).size !== task.oracle.deprecated_terms.length)) fail(`${task.id} deprecated_terms must be unique strings`);
    if (task.oracle?.type === "markdown_glossary" && !(task.oracle.deprecated_terms?.length > 0)) fail(`${task.id} glossary must enumerate deprecated_terms`);
    for (const command of task.post_load_permissions?.commands ?? []) {
      if (command.kind !== "node_script") fail(`${task.id} command kind is unsupported`);
      safeRelativePath(command.path_in_target, `${task.id} command path`);
      if (!Array.isArray(command.subcommands) || command.subcommands.some((name) => !["validate", "deliver"].includes(name))) fail(`${task.id} command subcommands are unsupported`);
    }
    const requiredCommands = task.oracle?.required_successful_commands ?? [];
    const declaredCommands = new Set((task.post_load_permissions?.commands ?? []).flatMap((command) => command.subcommands ?? []));
    if (requiredCommands.some((name) => !declaredCommands.has(name))) fail(`${task.id} oracle requires an undeclared command`);
    if (requiredCommands.includes("validate") || requiredCommands.includes("deliver")) {
      const chain = task.command_chain; if (!chain || task.oracle?.type !== "html") fail(`${task.id} validated delivery requires an HTML command_chain`);
      const source = safeRelativePath(chain.source_path, `${task.id} command source`); const artifact = safeRelativePath(chain.artifact_path, `${task.id} command artifact`);
      if (artifact !== task.oracle.path || !allowed.has(source) || !allowed.has(artifact) || source === artifact) fail(`${task.id} command_chain must bind one source and the oracle artifact`);
      if (!requiredCommands.includes("validate") || !requiredCommands.includes("deliver")) fail(`${task.id} command_chain requires validate and deliver`);
      const topology = task.oracle.topology_contract;
      if (!topology || !Number.isSafeInteger(topology.component_count) || !Number.isSafeInteger(topology.boundary_count) || !Number.isSafeInteger(topology.connection_count) || topology.connection_count < 1 || topology.forbid_directed_cycle !== true || topology.require_all_components_in_boundaries !== true || topology.require_partitioned_boundaries !== true || topology.forbid_unlisted_edges !== true || !Array.isArray(topology.required_boundaries) || topology.required_boundaries.length !== topology.boundary_count) fail(`${task.id} architecture oracle requires a bounded topology_contract`);
      for (const boundary of topology.required_boundaries) if (typeof boundary?.label !== "string" || !Array.isArray(boundary.wraps) || boundary.wraps.some((value) => typeof value !== "string")) fail(`${task.id} topology boundary is invalid`);
      for (const edge of topology.required_edges ?? []) if (typeof edge?.from !== "string" || typeof edge?.to !== "string" || (edge.label_regex !== undefined && typeof edge.label_regex !== "string")) fail(`${task.id} topology edge is invalid`); else if (edge.label_regex) compileRegex(edge.label_regex);
      if ((topology.hub !== undefined && typeof topology.hub !== "string") || (topology.spokes ?? []).some((value) => typeof value !== "string")) fail(`${task.id} topology hub contract is invalid`);
      for (const path of topology.required_paths ?? []) {
        if (!Array.isArray(path.nodes) || path.nodes.length < 2 || path.nodes.some((value) => typeof value !== "string") || (path.label_regex ?? []).length > path.nodes.length - 1) fail(`${task.id} topology path is invalid`);
        for (const pattern of path.label_regex ?? []) compileRegex(pattern);
      }
      const designedEdges = new Set([...(topology.required_edges ?? []).map((edge) => `${edge.from}\0${edge.to}`), ...(topology.required_paths ?? []).flatMap((path) => path.nodes.slice(0, -1).map((from, index) => `${from}\0${path.nodes[index + 1]}`))]);
      if (designedEdges.size !== topology.connection_count) fail(`${task.id} topology connection_count is inconsistent with required edges and paths`);
    } else if (task.command_chain !== undefined) fail(`${task.id} command_chain is not applicable`);
    if (task.oracle?.type === "redaction_bundle") {
      if (!task.oracle.private_path || !task.oracle.report_path || !(task.oracle.required_paths ?? []).includes(task.oracle.private_path) || !(task.oracle.required_paths ?? []).includes(task.oracle.report_path)) fail(`${task.id} redaction oracle must declare private_path and report_path`);
    }
  }
  return manifest;
}
export function effectiveTaskTimeout(task, defaultTimeoutMs) { return task.timeout_ms ?? defaultTimeoutMs; }

function copyTargetPackage(source, destination, includes) {
  const canonicalSource = realpathSync(source);
  const sourceTree = walkBoundedTree(canonicalSource, { label: "target package" });
  if (includes?.length) {
    if (existsSync(destination)) fail("package destination already exists");
    mkdirSync(dirname(destination), { recursive: true }); mkdirSync(destination);
    for (const item of includes) {
      const from = canonicalInside(join(canonicalSource, safeRelativePath(item)), canonicalSource, "package include");
      const to = safeDestination(destination, item, "package destination");
      if (existsSync(to)) fail(`package destination already exists: ${item}`);
      const member = sourceTree.files.find((file) => file.absolutePath === from);
      if (member) { mkdirSync(dirname(to), { recursive: true }); copyBoundedFile(from, to, { label: `package include ${item}` }); }
      else { mkdirSync(dirname(to), { recursive: true }); copyBoundedTree(from, to, { label: `package include ${item}` }); }
    }
  } else copyBoundedTree(canonicalSource, destination, { label: "target package" });
}

function treeState(root, options = {}) {
  const state = new Map(); if (!existsSync(root)) return state;
  const walked = walkBoundedTree(root, { label: options.label ?? "audited tree", allowSpecial: options.allowSpecial ?? true });
  const budget = createReadBudget();
  for (const file of walked.files) state.set(file.relativePath, fileSha(file.absolutePath, { label: options.label ?? "audited tree", budget, expectedIdentity: file.identity }));
  for (const special of walked.specials) state.set(special.relativePath, `special:${special.kind}`);
  return state;
}
function treeDigest(root) { return sha([...treeState(root, { label: "hashed tree", allowSpecial: false })].map(([path, digest]) => `${path}\0${digest}`).join("\n")); }
function sortedPathDigest(namedPaths) {
  const rows = [];
  for (const [label, path] of namedPaths) {
    if (!existsSync(path)) fail(`sealed bound path is missing: ${label}`);
    const stat = lstatSync(path); if (stat.isFile()) rows.push(`${label}\0${fileSha(path)}`);
    else if (stat.isDirectory()) for (const [member, digestValue] of treeState(path, { label: `sealed path ${label}`, allowSpecial: false })) rows.push(`${label}/${member}\0${digestValue}`);
    else fail(`sealed bound path is special: ${label}`);
  }
  return sha(rows.sort().join("\n"));
}
function changedPaths(before, after) { return [...new Set([...before.keys(), ...after.keys()])].filter((path) => before.get(path) !== after.get(path)).sort(); }
function chmodDirectories(root, mode) {
  const walked = walkBoundedTree(root, { label: "chmod tree" });
  for (const directory of [...walked.directories].sort((a, b) => b.depth - a.depth)) chmodSync(directory.absolutePath, mode);
  chmodSync(root, mode);
}
function sealTree(root) {
  const walked = walkBoundedTree(root, { label: "frozen inputs" });
  for (const file of walked.files) chmodSync(file.absolutePath, 0o444);
  for (const directory of [...walked.directories].sort((a, b) => b.depth - a.depth)) chmodSync(directory.absolutePath, 0o555);
  chmodSync(root, 0o555);
}

function runProcess(executable, args, options = {}) {
  const result = spawnSync(executable, args, { encoding: "utf8", shell: false, maxBuffer: options.maxBuffer ?? 64 * 1024 * 1024, timeout: options.timeout ?? CLI_TIMEOUT_MS, input: options.input, cwd: options.cwd, env: options.env });
  if (result.error) fail(`${basename(executable)} failed: ${result.error.message}`);
  if (result.signal) fail(`${basename(executable)} terminated by ${result.signal}`);
  return result;
}
export function classifyPiTermination(result) {
  if (result.error?.code === "ETIMEDOUT") return { kind: "timeout", signal: result.signal ?? null, exit_code: null };
  if (result.signal) return { kind: "signal", signal: result.signal, exit_code: null };
  if (result.error) return { kind: "spawn_error", signal: null, exit_code: null, error_code: result.error.code ?? "unknown" };
  if (result.status !== 0) return { kind: "exit_nonzero", signal: null, exit_code: result.status };
  return { kind: "completed", signal: null, exit_code: 0 };
}
export function piProcessFacts(result) {
  return { pi_exit_code: Number.isInteger(result.status) ? result.status : null, pi_termination: result.termination ?? classifyPiTermination(result) };
}
function runPiProcess(executable, args, options) {
  const result = spawnSync(executable, args, { encoding: "utf8", shell: false, maxBuffer: options.maxBuffer, timeout: options.timeout, cwd: options.cwd, env: options.env });
  return { ...result, stdout: result.stdout ?? "", stderr: result.stderr ?? "", termination: classifyPiTermination(result) };
}
function resolveExecutable(command) {
  const candidates = isAbsolute(command) ? [command] : (process.env.PATH ?? "").split(delimiter).flatMap((directory) => {
    if (!directory) return [];
    if (process.platform !== "win32") return [join(directory, command)];
    const extensions = (process.env.PATHEXT ?? ".EXE;.CMD;.BAT").split(";");
    return [join(directory, command), ...extensions.map((extension) => join(directory, `${command}${extension.toLowerCase()}`))];
  });
  for (const candidate of candidates) {
    try { accessSync(candidate, constants.X_OK); return realpathSync(candidate); } catch { /* keep searching */ }
  }
  fail(`Pi executable not found: ${command}`);
}
function readPiIdentity(executable) {
  const env = { PATH: process.env.PATH ?? "/usr/bin:/bin" };
  for (const name of ["LANG", "LC_ALL"]) if (process.env[name]) env[name] = process.env[name];
  const version = runProcess(executable, ["--version"], { timeout: CLI_TIMEOUT_MS, env });
  if (version.status !== 0) fail(`Pi --version exited ${version.status}`);
  return { executable_sha256: fileSha(executable), version_sha256: sha(version.stdout.trim()), executable };
}
export function readSourceRevision(repo = REPO) {
  const result = runProcess("git", ["rev-parse", "HEAD"], { cwd: repo, env: { PATH: process.env.PATH ?? "/usr/bin:/bin" } });
  if (result.status !== 0 || !/^[a-f0-9]{40}$/u.test(result.stdout.trim())) fail("unable to freeze CLI source revision");
  return result.stdout.trim();
}
function assertPiIdentity(identity) {
  const current = readPiIdentity(identity.executable);
  if (current.executable_sha256 !== identity.executable_sha256 || current.version_sha256 !== identity.version_sha256) fail("Pi runtime identity drifted during the suite");
}
function runJson(executable, common, args, input, artifactPath, artifactBudget) {
  const result = runProcess(executable, [...common, ...args], { input });
  if (artifactPath) boundedWriteFile(artifactPath, result.stdout, { mode: 0o600, label: `CLI ${args[0]} artifact`, budget: artifactBudget });
  if (result.status !== 0) fail(`${basename(executable)} ${args[0]} exited ${result.status}: ${result.stderr}`);
  const envelope = JSON.parse(result.stdout); if (envelope.schema_version !== 1 || envelope.ok !== true) fail(`invalid ${args[0]} JSON envelope`);
  return envelope;
}

export function planWorkspaceInputs(task) {
  const entries = Object.entries(task.workspace_files ?? {}); const label = task.id ?? "task";
  if (entries.length > HARNESS_IO_LIMITS.maxEntries) fail(`${label} workspace file count exceeds ${HARNESS_IO_LIMITS.maxEntries}`);
  let totalBytes = 0; const normalized = new Map();
  for (const [path, content] of entries) {
    if (typeof content !== "string") fail(`${label} workspace content must be text`);
    const normalizedPath = safeRelativePath(path, `${label} workspace path`); const depth = normalizedPath.split("/").length;
    if (depth > HARNESS_IO_LIMITS.maxDepth) fail(`${label} workspace path depth exceeds ${HARNESS_IO_LIMITS.maxDepth}`);
    if (normalized.has(normalizedPath)) fail(`${label} workspace paths collide after normalization`);
    const bytes = Buffer.byteLength(content, "utf8");
    if (bytes > HARNESS_IO_LIMITS.maxSingleFileBytes) fail(`${label} workspace file exceeds ${HARNESS_IO_LIMITS.maxSingleFileBytes} bytes`);
    totalBytes += bytes; if (totalBytes > HARNESS_IO_LIMITS.maxTotalBytes) fail(`${label} workspace total exceeds ${HARNESS_IO_LIMITS.maxTotalBytes} bytes`);
    normalized.set(normalizedPath, { path: normalizedPath, content, bytes });
  }
  const paths = [...normalized.keys()].sort();
  const directories = new Set();
  for (const path of paths) {
    const parts = path.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const directory = parts.slice(0, index).join("/"); directories.add(directory);
      if (normalized.has(directory)) fail(`${label} workspace file/directory prefix conflict`);
    }
  }
  if (paths.length + directories.size > HARNESS_IO_LIMITS.maxEntries) fail(`${label} workspace materialized entry count exceeds ${HARNESS_IO_LIMITS.maxEntries}`);
  return { entries: paths.map((path) => normalized.get(path)), totalBytes };
}

export function writeWorkspaceInputs(task, destination) {
  const plan = planWorkspaceInputs(task); const budget = createReadBudget();
  mkdirSync(destination, { recursive: true });
  for (const entry of plan.entries) {
    const target = safeDestination(destination, entry.path, "workspace input"); mkdirSync(dirname(target), { recursive: true }); boundedWriteFile(target, entry.content, { flag: "wx", budget, label: "workspace input" });
  }
}

export function freezeSuite(options, manifest, manifestBytes, tasks) {
  mkdirSync(options.runsDir, { recursive: true });
  const suiteRoot = mkdtempSync(join(options.runsDir, `${manifest.suite_id}-`)); chmodSync(suiteRoot, 0o700);
  const frozen = join(suiteRoot, "frozen-inputs"); mkdirSync(frozen, { recursive: true });
  const paths = { manifest: join(frozen, "manifest.json"), bootstrap: join(frozen, "bootstrap-SKILL.md"), cli: join(frozen, process.platform === "win32" ? "skillroster.exe" : "skillroster"), gate: join(frozen, "gate.ts"), runner: join(frozen, "runner.mjs"), bounds: join(frozen, "bounds.mjs") };
  const frozenSources = [[options.bootstrap, paths.bootstrap], [options.cli, paths.cli], [join(HERE, "gate.ts"), paths.gate], [RUNNER_PATH, paths.runner], [join(HERE, "bounds.mjs"), paths.bounds]];
  for (const [source] of frozenSources) fileSha(source, { label: "frozen source" });
  boundedWriteFile(paths.manifest, manifestBytes, { mode: 0o444, flag: "wx", label: "frozen manifest" });
  const frozenSourceBudget = createReadBudget();
  for (const [source, destination] of frozenSources) copyBoundedFile(source, destination, { label: "frozen source", budget: frozenSourceBudget });
  chmodSync(paths.cli, 0o555); for (const path of [paths.bootstrap, paths.gate, paths.runner, paths.bounds]) chmodSync(path, 0o444);
  const skillsRoot = realpathSync(options.skillsRoot); const frozenTasks = new Map();
  for (const task of tasks) {
    const taskRoot = join(frozen, "tasks", task.id); const packageRoot = join(taskRoot, "target-package"); const workspaceRoot = join(taskRoot, "workspace");
    const source = canonicalInside(join(skillsRoot, task.expected_skill), skillsRoot, "Skill source");
    copyTargetPackage(source, packageRoot, task.target_package_include); writeWorkspaceInputs(task, workspaceRoot);
    frozenTasks.set(task.id, { packageRoot, workspaceRoot, packageDigest: treeDigest(packageRoot), workspaceDigest: treeDigest(workspaceRoot) });
  }
  const evaluationContract = buildEvaluationContract(manifest, tasks);
  if (manifest.seal_contract) {
    const boundPaths = sealSourceBoundPaths(options);
    assertBoundPathsClean([...boundPaths, join(dirname(options.manifest), manifest.seal_contract)]);
    verifySealContract(options.sealContract, sealFacts(manifest, paths, frozenTasks, evaluationContract, options), { repo: REPO, contractPath: join(dirname(options.manifest), manifest.seal_contract), boundPaths });
  }
  if (!options.piConfigSnapshot) options.piConfigSnapshot = options.loadPiConfig();
  const base = { suite_id: manifest.suite_id, manifest_sha256: fileSha(paths.manifest), bootstrap_sha256: fileSha(paths.bootstrap), cli_sha256: fileSha(paths.cli), cli_source_revision: options.cliSourceRevision, gate_sha256: fileSha(paths.gate), runner_sha256: fileSha(paths.runner), bounds_sha256: fileSha(paths.bounds), pi_executable_sha256: options.piIdentity.executable_sha256, pi_version_sha256: options.piIdentity.version_sha256, pi_model_mapping_sha256: options.piConfigSnapshot.modelMappingDigest, evaluation_contract_sha256: sha(JSON.stringify(evaluationContract)), arm_schedule_seed: options.armSchedule.seed, arm_schedule_sha256: sha(JSON.stringify(options.armSchedule.order)) };
  const suiteSnapshotSha = sha(JSON.stringify({ base, tasks: [...frozenTasks].map(([id, task]) => [id, task.packageDigest, task.workspaceDigest]) }));
  sealTree(frozen); chmodSync(paths.cli, 0o555);
  const taskDigests = Object.fromEntries([...frozenTasks].map(([id, task]) => [id, { target_package_sha256: task.packageDigest, workspace_inputs_sha256: task.workspaceDigest }]));
  const snapshotPath = join(suiteRoot, "suite-snapshot.json"); boundedWriteFile(snapshotPath, `${JSON.stringify({ schema_version: 1, suite_snapshot_sha256: suiteSnapshotSha, ...base, tasks: taskDigests }, null, 2)}\n`, { mode: 0o444, flag: "wx", label: "suite snapshot" });
  return { suiteRoot, paths, tasks: frozenTasks, base, suiteSnapshotSha };
}

export function buildEvaluationContract(manifest, tasks = manifest.tasks) {
  return { model: manifest.model, tools: manifest.common.tools, aggregate_gate: manifest.aggregate_gate, tasks: tasks.map((task) => ({ id: task.id, permissions: task.post_load_permissions, command_chain: task.command_chain ?? null, oracle: task.oracle })) };
}
function sealSourceBoundPaths(options) {
  return [options.manifest, dirname(options.bootstrap), join(REPO, "src"), join(REPO, "Cargo.toml"), join(REPO, "Cargo.lock"), join(HERE, "runner.mjs"), join(HERE, "gate.ts"), join(HERE, "bounds.mjs")];
}
export function assertBoundPathsClean(paths, repo = REPO) {
  const relativePaths = paths.map((path) => relative(repo, resolve(path)));
  if (relativePaths.some((path) => path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path))) fail("sealed repo path escapes source root");
  const result = spawnSync("git", ["status", "--porcelain=v1", "--untracked-files=all", "--", ...relativePaths], { cwd: repo, encoding: "utf8", shell: false });
  if (result.status !== 0) fail("unable to inspect sealed bound paths");
  assertNoBoundPathDrift(result.stdout);
  return true;
}
export function assertNoBoundPathDrift(statusText) {
  if (String(statusText).trim()) fail("sealed bound paths have staged, unstaged, or untracked drift");
  return true;
}
export function sealFacts(manifest, paths, frozenTasks, evaluationContract, options) {
  const sourcePaths = [["cli-src", join(REPO, "src")], ["cargo-toml", join(REPO, "Cargo.toml")], ["cargo-lock", join(REPO, "Cargo.lock")]];
  const harnessPaths = [["runner", paths.runner], ["gate", paths.gate], ["bounds", paths.bounds]];
  return {
    suite_id: manifest.suite_id,
    manifest_sha256: fileSha(paths.manifest),
    bootstrap_package_sha256: treeDigest(dirname(options.bootstrap)),
    cli_binary_sha256: fileSha(paths.cli),
    cli_source_tree_sha256: sortedPathDigest(sourcePaths),
    harness_tree_sha256: sortedPathDigest(harnessPaths),
    target_packages_sha256: Object.fromEntries([...frozenTasks].map(([id, task]) => [id, task.packageDigest])),
    materialized_workspaces_sha256: Object.fromEntries([...frozenTasks].map(([id, task]) => [id, task.workspaceDigest])),
    oracle_contract_sha256: sha(JSON.stringify(manifest.tasks.map((task) => ({ id: task.id, oracle: task.oracle })))),
    evaluation_contract_sha256: sha(JSON.stringify(evaluationContract)),
    public_model_profile_sha256: sha(JSON.stringify({ model: manifest.model, tools: manifest.common.tools })),
    arm_schedule_seed: options.armSchedule.seed,
    arm_schedule_sha256: sha(JSON.stringify(options.armSchedule.order)),
    git_bound_tree_sha256: gitBoundTreeDigest(options.cliSourceRevision, sealSourceBoundPaths(options), REPO),
  };
}
function repoRelativePaths(paths, repo) {
  const values = paths.map((path) => relative(repo, resolve(path)));
  if (values.some((path) => path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path))) fail("sealed repo path escapes source root");
  return values;
}
export function gitBoundTreeDigest(revision, paths, repo = REPO) {
  if (!/^[a-f0-9]{40}$/u.test(revision ?? "")) fail("sealed source revision is invalid");
  const verify = spawnSync("git", ["cat-file", "-e", `${revision}^{commit}`], { cwd: repo, encoding: "utf8", shell: false });
  if (verify.status !== 0) fail("sealed source revision does not exist");
  const result = spawnSync("git", ["ls-tree", "-r", "-z", "--full-tree", revision, "--", ...repoRelativePaths(paths, repo)], { cwd: repo, encoding: null, shell: false });
  if (result.status !== 0 || !result.stdout?.length) fail("unable to read sealed source tree");
  return sha(result.stdout);
}
export function verifyFirstSealBlob(contractPath, sourceRevision, repo = REPO) {
  const relativePath = repoRelativePaths([contractPath], repo)[0]; const live = boundedReadFile(contractPath, { label: "seal contract" });
  const log = spawnSync("git", ["log", "--reverse", "--diff-filter=A", "--format=%H", "--", relativePath], { cwd: repo, encoding: "utf8", shell: false });
  const addCommit = log.status === 0 ? log.stdout.trim().split("\n").filter(Boolean)[0] : null;
  if (!addCommit) fail("holdout seal has no first-add Git blob");
  const original = spawnSync("git", ["show", `${addCommit}:${relativePath}`], { cwd: repo, encoding: null, shell: false });
  if (original.status !== 0 || !original.stdout.equals(live)) fail("holdout seal differs from its immutable first-add blob");
  const existedAtSource = spawnSync("git", ["cat-file", "-e", `${sourceRevision}:${relativePath}`], { cwd: repo, encoding: "utf8", shell: false });
  if (existedAtSource.status === 0) fail("holdout seal must be added after its bound source revision");
  return { add_commit: addCommit, blob_sha256: sha(live) };
}
export function verifySealContract(contract, expectedFacts, provenance = null) {
  if (contract?.schema_version !== 1 || contract?.suite_id !== expectedFacts.suite_id || contract?.seal_state !== "frozen_before_first_run") fail("invalid holdout seal contract identity");
  if (!/^[a-f0-9]{40}$/u.test(contract.source_revision ?? "")) fail("holdout seal source revision is invalid");
  if (contract.seal_sha256 !== sealPayloadDigest(contract.source_revision, contract.facts)) fail("holdout seal contract digest is invalid");
  if (canonicalJson(contract.facts) !== canonicalJson(expectedFacts)) fail("holdout seal contract does not match frozen inputs");
  if (provenance) {
    const sourceTree = gitBoundTreeDigest(contract.source_revision, provenance.boundPaths, provenance.repo);
    if (sourceTree !== contract.facts.git_bound_tree_sha256 || sourceTree !== gitBoundTreeDigest(readSourceRevision(provenance.repo), provenance.boundPaths, provenance.repo)) fail("holdout seal source tree does not match live bound paths");
    verifyFirstSealBlob(provenance.contractPath, contract.source_revision, provenance.repo);
  }
  return true;
}

export function generateSealContract(options, manifest, manifestBytes) {
  if (!manifest.seal_contract) fail("seal generation requires a sealed manifest");
  const output = safeDestination(dirname(options.manifest), manifest.seal_contract, "seal contract");
  if (resolve(options.generateSeal) !== output) fail("--generate-seal must name the manifest seal_contract path");
  if (existsSync(output)) fail("seal contract already exists; suite ids cannot be re-signed");
  const boundPaths = sealSourceBoundPaths(options); assertBoundPathsClean(boundPaths);
  options.cliSourceRevision = readSourceRevision(); options.armSchedule = buildArmSchedule(manifest.tasks, "both", manifest.common.randomize_arm_order, manifest.arm_schedule_seed);
  options.piIdentity = { executable_sha256: "not_used_for_seal", version_sha256: "not_used_for_seal" }; options.piConfigSnapshot = { modelMappingDigest: "not_used_for_seal" };
  const unsealed = structuredClone(manifest); delete unsealed.seal_contract;
  const frozen = freezeSuite(options, unsealed, manifestBytes, unsealed.tasks); const facts = sealFacts(manifest, frozen.paths, frozen.tasks, buildEvaluationContract(manifest), options);
  const contract = { schema_version: 1, suite_id: manifest.suite_id, seal_state: "frozen_before_first_run", source_revision: options.cliSourceRevision, facts, seal_sha256: sealPayloadDigest(options.cliSourceRevision, facts) };
  boundedWriteFile(output, `${JSON.stringify(contract, null, 2)}\n`, { mode: 0o444, flag: "wx", label: "seal contract" });
  return { output, contract };
}

function modelMappingFacts(files, requestedModel) {
  const [provider, ...modelParts] = requestedModel.split("/"); const modelId = modelParts.join("/"); const sources = [];
  for (const name of ["models.json", "models-store.json"]) {
    const content = files.get(name); if (!content) { sources.push({ name, present: false }); continue; }
    let parsed; try { parsed = JSON.parse(content); } catch { fail(`${name} is not valid JSON`); }
    const providerConfig = parsed?.providers?.[provider] ?? parsed?.[provider];
    const models = Array.isArray(providerConfig?.models) ? providerConfig.models : [];
    const matchingModels = models.filter((model) => (typeof model === "string" ? model : model?.id) === modelId || (typeof model === "string" ? model : model?.id) === requestedModel).map((model) => {
      if (typeof model === "string") return { id: model };
      return Object.fromEntries(["id", "api", "reasoning", "contextWindow", "maxTokens"].filter((key) => ["string", "number", "boolean"].includes(typeof model?.[key])).map((key) => [key, model[key]]));
    });
    sources.push({ name, present: true, provider_present: Boolean(providerConfig), provider_api: typeof providerConfig?.api === "string" ? providerConfig.api : null, matching_models: matchingModels });
  }
  return { requested_model: requestedModel, provider, sources };
}

export function snapshotPiConfig(source, requestedModel) {
  const files = new Map(); const budget = createReadBudget();
  for (const name of ["auth.json", "models.json", "models-store.json"]) {
    const path = join(source, name); if (existsSync(path)) files.set(name, boundedReadFile(path, { label: `Pi config ${name}`, budget }));
  }
  if (!files.has("auth.json")) fail("isolated Pi config requires auth.json");
  const privateFingerprint = sha([...files].map(([name, content]) => `${name}\0${sha(content)}`).join("\n"));
  const modelMappingDigest = sha(JSON.stringify(modelMappingFacts(files, requestedModel)));
  return { files, privateFingerprint, modelMappingDigest };
}
export function copyPiConfig(snapshot, destination) {
  const before = sha([...snapshot.files].map(([name, content]) => `${name}\0${sha(content)}`).join("\n"));
  if (before !== snapshot.privateFingerprint) fail("in-memory Pi config snapshot drifted");
  mkdirSync(destination, { recursive: true, mode: 0o700 });
  for (const [name, content] of snapshot.files) { const to = join(destination, name); boundedWriteFile(to, content, { mode: 0o600, flag: "wx", label: `Pi config ${name}` }); }
  const after = sha([...snapshot.files.keys()].map((name) => `${name}\0${fileSha(join(destination, name))}`).join("\n"));
  if (after !== snapshot.privateFingerprint) fail("copied Pi config differs from suite snapshot");
}
export function cleanupPrivateConfig(path, runRoot) {
  const candidate = resolve(path); const lexicalRoot = resolve(runRoot); const root = realpathSync(runRoot);
  if (!within(candidate, lexicalRoot) || candidate === lexicalRoot) fail("refusing unsafe private-config cleanup");
  if (existsSync(candidate) && !within(realpathSync(candidate), root)) fail("refusing unsafe private-config cleanup");
  if (existsSync(candidate)) rmSync(candidate, { recursive: true, force: true });
}
function isolatedPiEnvironment(home, config, sessions, policy, isolatedTmp) {
  const env = { PATH: process.env.PATH ?? "/usr/bin:/bin" };
  for (const name of ["LANG", "LC_ALL", "SSL_CERT_FILE"]) if (process.env[name]) env[name] = process.env[name];
  return { ...env, HOME: home, TMPDIR: isolatedTmp, PI_CODING_AGENT_DIR: config, PI_CODING_AGENT_SESSION_DIR: sessions, PI_OFFLINE: "1", PI_TELEMETRY: "0", SKILLROSTER_PI_GATE_POLICY: policy };
}
function resolveNamedRoots(names, workspace, targetPackage) {
  return (names ?? []).map((name) => name === "workspace" ? workspace : name === "target_package" ? targetPackage : fail(`unknown permission root: ${name}`));
}
function commandPolicies(commands, targetPackage) {
  return (commands ?? []).flatMap((command) => {
    const script = canonicalInside(join(targetPackage, command.path_in_target), targetPackage, "allowlisted script");
    return command.subcommands.map((name) => ({
      name, executable: process.execPath, fixed_argv: [script, name],
      arguments: name === "validate" ? [
        { kind: "enum", values: ["architecture", "workflow", "sequence", "dataflow", "lifecycle"] }, { kind: "read_path" },
        { kind: "literal", value: "--quality" }, { kind: "literal", value: "showcase" }, { kind: "literal", value: "--json" },
      ] : [
        { kind: "enum", values: ["architecture", "workflow", "sequence", "dataflow", "lifecycle"] }, { kind: "read_path" }, { kind: "write_path" },
        { kind: "literal", value: "--quality" }, { kind: "literal", value: "showcase" }, { kind: "literal", value: "--json" },
      ],
    }));
  });
}
export function commandUsage(command) {
  const render = (rule) => rule.kind === "literal" ? rule.value : rule.kind === "enum" ? `<${rule.values.join("|")}>` : rule.kind === "read_path" ? "<READ_PATH>" : rule.kind === "write_path" ? "<WRITE_PATH>" : "<TEXT>";
  return `harness_command name=${command.name} args=[${(command.arguments ?? []).map(render).join(", ")}]`;
}
function extractFinalText(transcript) {
  let answer = "";
  for (const line of transcript.split("\n")) {
    if (!line) continue; let event; try { event = JSON.parse(line); } catch { continue; }
    if (event.type === "message_end" && event.message?.role === "assistant") answer = (event.message.content ?? []).filter((part) => part.type === "text").map((part) => part.text).join("");
  }
  return answer;
}
export function assessTranscriptCompletion(transcript) {
  const events = []; let malformed = 0;
  for (const line of transcript.split("\n").filter(Boolean)) { try { events.push(JSON.parse(line)); } catch { malformed += 1; } }
  if (malformed > 0) return { status: "failed", failure_type: "transcript_invalid", malformed_event_count: malformed, assistant_completion_count: 0 };
  const completions = events.filter((event) => event?.type === "message_end" && event?.message?.role === "assistant");
  const successful = completions.filter((event) => ["stop", "end_turn", "completed"].includes(event.message?.stopReason)); const finalCompletion = completions.at(-1);
  if (finalCompletion && ["stop", "end_turn", "completed"].includes(finalCompletion.message?.stopReason)) return { status: "completed", failure_type: null, malformed_event_count: 0, assistant_completion_count: successful.length };
  const errors = completions.filter((event) => event.message?.stopReason === "error");
  if (errors.length > 0) {
    const transport = errors.some((event) => /fetch|websocket|network|transport|econn|socket|connection/iu.test(String(event.message?.errorMessage ?? "")));
    return { status: "failed", failure_type: transport ? "provider_transport_failure" : "provider_completion_error", malformed_event_count: 0, assistant_completion_count: 0, provider_error_count: errors.length };
  }
  return { status: "failed", failure_type: "assistant_completion_missing", malformed_event_count: 0, assistant_completion_count: 0 };
}
export function parseGateEvents(path) {
  const events = []; const errors = [];
  if (!existsSync(path)) return { events, errors, source: "missing" };
  for (const [index, line] of boundedReadFile(path, { encoding: "utf8", label: "gate event ledger", maxSingleFileBytes: GATE_LEDGER_MAX_BYTES }).split("\n").filter(Boolean).entries()) {
    try { events.push(JSON.parse(line)); } catch { errors.push(`gate_event_parse_error:${index + 1}`); }
  }
  return { events, errors, source: "file" };
}
export function gateEventsBinding(path) {
  return existsSync(path) ? { source: "file", sha256: fileSha(path, { label: "gate event ledger", maxSingleFileBytes: GATE_LEDGER_MAX_BYTES }) } : { source: "missing_sentinel", sha256: sha("skillroster:gate-events:missing:v1") };
}
export function gateEventIntegrity(events, parseErrors, arm, source) {
  const violations = [];
  if (source === "missing") violations.push("harness_violation:gate_events_missing");
  else if (events.length === 0 && parseErrors.length === 0) violations.push("harness_violation:gate_events_empty");
  for (const error of parseErrors) violations.push(`harness_violation:${error}`);
  if (events.some((event) => event?.schema_version !== 1)) violations.push("harness_violation:gate_events_wrong_schema");
  const ready = events.filter((event) => event?.kind === "gate_ready");
  if (ready.length === 0) violations.push("harness_violation:gate_ready_missing");
  if (ready.length > 1) violations.push("harness_violation:gate_ready_duplicate");
  if (ready.length === 1 && ready[0].arm !== arm) violations.push("safety_violation:gate_ready_wrong_arm");
  return [...new Set(violations)];
}
function compileRegex(value) {
  const inline = value.match(/^\(\?([ims]+)\)/u); return new RegExp(inline ? value.slice(inline[0].length) : value, `u${inline?.[1] ?? ""}`);
}

export function evaluateOracle(oracle, workspace, successfulCommands, changed = []) {
  const failures = []; const budget = createReadBudget();
  const read = (path) => {
    const absolute = safeDestination(workspace, path, "oracle path");
    if (!existsSync(absolute) || !lstatSync(absolute).isFile()) { failures.push(`missing:${path}`); return ""; }
    return boundedReadFile(canonicalInside(absolute, workspace, "oracle output"), { encoding: "utf8", label: `oracle output ${path}`, budget });
  };
  const requireStrings = (text, values, prefix) => { for (const value of values ?? []) if (!text.includes(value)) failures.push(`${prefix}:missing:${value}`); };
  const forbidStrings = (text, values, prefix) => { for (const value of values ?? []) if (text.includes(value)) failures.push(`${prefix}:forbidden:${value}`); };
  const requireRegex = (text, values, prefix) => { for (const value of values ?? []) if (!compileRegex(value).test(text)) failures.push(`${prefix}:regex:${value}`); };
  const forbidRegex = (text, values, prefix) => { for (const value of values ?? []) if (compileRegex(value).test(text)) failures.push(`${prefix}:forbidden_regex:${value}`); };
  if (["html", "text", "markdown_glossary"].includes(oracle.type)) {
    const text = read(oracle.path); if (oracle.minimum_bytes && Buffer.byteLength(text) < oracle.minimum_bytes) failures.push("minimum_bytes");
    requireStrings(text, oracle.required_substrings, oracle.path); forbidStrings(text, oracle.forbidden_substrings, oracle.path);
    requireRegex(text, oracle.required_regex, oracle.path); forbidRegex(text, oracle.forbidden_regex, oracle.path);
    const count = [...text.trim()].length;
    if (count < (oracle.minimum_characters ?? 0)) failures.push("minimum_characters");
    if (count > (oracle.maximum_characters ?? Number.MAX_SAFE_INTEGER)) failures.push("maximum_characters");
    for (const path of oracle.forbidden_paths ?? []) if (existsSync(safeDestination(workspace, path, "forbidden path"))) failures.push(`forbidden_path:${path}`);
    if (oracle.type === "markdown_glossary") for (const term of oracle.deprecated_terms ?? []) {
      const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
      const escaped = escape(term); const relation = "(避免|停用|旧称|不再使用|淘汰|弃用|不要使用)";
      const otherTerms = (oracle.deprecated_terms ?? []).filter((candidate) => candidate !== term).map(escape);
      const between = otherTerms.length ? `(?:(?!${otherTerms.join("|")})[^\\r\\n]){0,60}` : "[^\\r\\n]{0,60}";
      if (!new RegExp(`(${escaped}${between}${relation}|${relation}${between}${escaped})`, "u").test(text)) failures.push(`${oracle.path}:deprecated_relation:${term}`);
    }
  } else if (oracle.type === "redaction_bundle") {
    const required = new Map((oracle.required_paths ?? []).map((path) => [path, read(path)]));
    const publicText = (oracle.public_seed_paths ?? []).map(read).join("\n");
    requireRegex(publicText, oracle.public_required_regex, "public");
    requireStrings(required.get(oracle.private_path) ?? "", oracle.private_required_substrings, "private");
    requireRegex(required.get(oracle.private_path) ?? "", oracle.private_required_regex, "private");
    requireStrings(required.get(oracle.report_path) ?? "", oracle.report_required_substrings, "report");
    requireRegex(required.get(oracle.report_path) ?? "", oracle.report_required_regex, "report");
    const allChanged = changed.map((path) => safeDestination(workspace, path, "changed output")).filter((path) => existsSync(path) && lstatSync(path).isFile()).map((path) => boundedReadFile(canonicalInside(path, workspace, "changed output"), { encoding: "utf8", label: "changed output", budget })).join("\n");
    forbidStrings(allChanged, oracle.forbidden_across_outputs, "changed_outputs");
  } else failures.push(`unsupported_oracle:${oracle.type}`);
  for (const path of oracle.required_nonempty_paths ?? []) {
    const absolute = safeDestination(workspace, path, "required nonempty path");
    if (!existsSync(absolute) || !lstatSync(absolute).isFile() || boundedReadFile(absolute, { encoding: "utf8", label: `required output ${path}`, budget }).trim().length === 0) failures.push(`empty:${path}`);
  }
  for (const command of oracle.required_successful_commands ?? []) if (!successfulCommands.has(command)) failures.push(`command_not_successful:${command}`);
  return { passed: !failures.length, failures };
}

export function assessWorkspaceChanges(before, after, inputPaths, allowedPaths) {
  const changed = changedPaths(before, after); const allowed = new Set(allowedPaths);
  return {
    changed,
    input_mutations: inputPaths.filter((path) => before.get(path) !== after.get(path)),
    unexpected_changes: changed.filter((path) => !allowed.has(path)),
    special_outputs: changed.filter((path) => after.get(path)?.startsWith("special:")),
  };
}
export function summarizePolicyDenials(events) {
  const denials = events.filter((event) => event.classification === "policy_denial");
  const contained = denials.filter((event) => event.failure_type === "output_path_denied" && event.contained === true);
  return { policy_outcome: denials.length ? "denied" : "clean", contained_denial_count: contained.length, contained_denials: contained, policy_denials: denials };
}
export function evaluateTopologyContract(facts, contract) {
  if (!contract) return [];
  const failures = []; if (!facts) return ["topology:validated_graph_missing"];
  if (facts.component_count !== contract.component_count) failures.push("topology:component_count");
  if (facts.boundary_count !== contract.boundary_count) failures.push("topology:boundary_count");
  if (facts.connections.length !== contract.connection_count) failures.push("topology:connection_count");
  if (contract.forbid_directed_cycle && facts.has_directed_cycle) failures.push("topology:directed_cycle");
  if (contract.require_all_components_in_boundaries) { const covered = new Set(facts.boundaries.flatMap((boundary) => boundary.wraps)); if (facts.components.some((component) => !covered.has(component))) failures.push("topology:boundary_coverage"); }
  if (contract.require_partitioned_boundaries) { const membership = new Map(facts.components.map((component) => [component, 0])); for (const boundary of facts.boundaries) for (const component of boundary.wraps) membership.set(component, (membership.get(component) ?? 0) + 1); if ([...membership.values()].some((count) => count !== 1)) failures.push("topology:boundary_partition"); }
  const boundaryKey = (boundary) => `${boundary.label}\0${[...boundary.wraps].sort().join("\0")}`; const actualBoundaries = new Set(facts.boundaries.map(boundaryKey));
  for (const boundary of contract.required_boundaries ?? []) if (!actualBoundaries.has(boundaryKey(boundary))) failures.push(`topology:boundary:${boundary.label}`);
  const edge = (from, to, pattern = null) => facts.connections.some((candidate) => candidate.from === from && candidate.to === to && (!pattern || compileRegex(pattern).test(candidate.label)));
  for (const required of contract.required_edges ?? []) if (!edge(required.from, required.to, required.label_regex)) failures.push(`topology:edge:${required.from}->${required.to}`);
  if (contract.hub) for (const spoke of contract.spokes ?? []) if (!edge(contract.hub, spoke)) failures.push(`topology:hub_spoke:${contract.hub}->${spoke}`);
  for (const path of contract.required_paths ?? []) for (let index = 0; index < path.nodes.length - 1; index += 1) if (!edge(path.nodes[index], path.nodes[index + 1], path.label_regex?.[index] ?? null)) failures.push(`topology:path:${path.nodes[index]}->${path.nodes[index + 1]}`);
  if (contract.forbid_unlisted_edges) { const listed = new Set([...(contract.required_edges ?? []).map((item) => `${item.from}\0${item.to}`), ...(contract.required_paths ?? []).flatMap((path) => path.nodes.slice(0, -1).map((from, index) => `${from}\0${path.nodes[index + 1]}`))]); for (const candidate of facts.connections) if (!listed.has(`${candidate.from}\0${candidate.to}`)) failures.push(`topology:unlisted_edge:${candidate.from}->${candidate.to}`); }
  return failures;
}
export function assessCommandReceipt(events, oracle, workspace) {
  const required = oracle.required_successful_commands ?? [];
  if (!required.includes("validate") && !required.includes("deliver")) return { status: "not_applicable", failures: [], topology_failures: [], final_artifact_sha256: null, receipt_chain_sha256: null };
  const failures = []; const commands = events.map((event, index) => ({ event, index })).filter(({ event }) => event.kind === "command" && event.exit_code === 0);
  const deliver = commands.filter(({ event }) => event.name === "deliver").at(-1); const validate = deliver ? commands.filter(({ event, index }) => event.name === "validate" && index < deliver.index && event.receipt_chain_sha256 === deliver.event.validation_receipt_sha256).at(-1) : null;
  if (!deliver) failures.push("command_receipt:deliver_missing");
  if (!validate) failures.push("command_receipt:validation_chain_missing");
  const artifactPath = safeDestination(workspace, oracle.path, "command receipt artifact"); let finalArtifactSha = null;
  if (!existsSync(artifactPath) || !lstatSync(artifactPath).isFile()) failures.push("command_receipt:artifact_missing");
  else finalArtifactSha = fileSha(canonicalInside(artifactPath, workspace, "command receipt artifact"), { label: "command receipt artifact" });
  if (deliver) {
    const canonicalArtifact = existsSync(artifactPath) ? canonicalInside(artifactPath, workspace, "command receipt artifact") : resolve(artifactPath);
    if (deliver.event.artifact_path_sha256 !== sha(canonicalArtifact)) failures.push("command_receipt:artifact_path_mismatch");
    if (deliver.event.artifact_sha256 !== finalArtifactSha) failures.push("command_receipt:artifact_digest_drift");
    if (!deliver.event.receipt_chain_sha256) failures.push("command_receipt:deliver_chain_missing");
    if (validate && (validate.event.source_sha256 !== deliver.event.source_sha256 || validate.event.receipt_chain_sha256 !== deliver.event.validation_receipt_sha256)) failures.push("command_receipt:source_chain_mismatch");
  }
  const topologyFailures = evaluateTopologyContract(validate?.event.graph_facts, oracle.topology_contract);
  return { status: failures.length || topologyFailures.length ? "failed" : "passed", failures, topology_failures: topologyFailures, final_artifact_sha256: finalArtifactSha, receipt_chain_sha256: deliver?.event.receipt_chain_sha256 ?? null };
}
function redactionViolations(workspace, changed) {
  const patterns = [/\/Users\//u, /<REDACTED_[A-Z_]+>/u, /\b(?:sk|pk|ghp)_[A-Za-z0-9_-]{12,}\b/u, /Bearer\s+[A-Za-z0-9._-]{12,}/iu]; const violations = []; const budget = createReadBudget();
  for (const path of changed) {
    const absolute = safeDestination(workspace, path, "redaction output"); if (!existsSync(absolute) || !lstatSync(absolute).isFile()) continue;
    if (patterns.some((pattern) => pattern.test(boundedReadFile(canonicalInside(absolute, workspace, "redaction output"), { encoding: "utf8", label: "redaction output", budget })))) violations.push(`redaction:${path}`);
  }
  return violations;
}

export function classifyExecutionFailure(piExitCode, termination, transcriptCompletion) {
  if (termination?.kind === "timeout") return "wall_timeout";
  if (termination?.kind === "signal") return "pi_signal_termination";
  if (termination?.kind === "spawn_error") return "pi_spawn_failure";
  if (termination?.kind === "exit_nonzero" || piExitCode !== 0) return "pi_process_failure";
  return transcriptCompletion.status !== "completed" ? transcriptCompletion.failure_type : null;
}

export function deriveOutcomes(arm, events, oraclePassed, piExitCode, safetyViolations, transcriptCompletion = { status: "completed", failure_type: null }, termination = null) {
  const retrievalAttempts = events.filter((event) => event.kind === "retrieval_succeeded" || event.kind === "retrieval_failed").length;
  const retrieved = events.some((event) => event.kind === "retrieval_succeeded" && event.task_mismatch === false);
  const retrievalFailed = events.some((event) => event.kind === "retrieval_failed"); const loaded = arm === "core" || events.some((event) => event.kind === "target_skill_loaded");
  const contractViolation = arm === "on_demand" && (retrievalAttempts > 2 || events.some((event) => event.contract_violation === true || (event.kind === "retrieval_failed" && event.failure_type === "task_mismatch")));
  const executionFailureType = classifyExecutionFailure(piExitCode, termination, transcriptCompletion);
  const execution = executionFailureType ? "execution_failed" : oraclePassed ? "task_succeeded" : "task_failed";
  const protocol = arm === "core" ? "core_control" : loaded && retrieved ? "retrieval_loaded" : retrieved ? "load_wrong" : retrievalFailed ? "retrieval_wrong" : "no_retrieval_call";
  const safety = safetyViolations.length ? "failed" : "passed"; const protocolPassed = arm === "core" || protocol === "retrieval_loaded";
  const deepestStage = arm === "core" ? execution === "task_succeeded" ? "task_succeeded" : "task_execution_failed" : protocol !== "retrieval_loaded" ? protocol : execution === "task_succeeded" ? "task_succeeded" : "task_execution_failed";
  return { execution_outcome: execution, execution_failure_type: executionFailureType, protocol_outcome: protocol, deepest_stage: deepestStage, safety_outcome: safety, contract_violation: contractViolation, retrieval_attempt_count: retrievalAttempts, accepted: execution === "task_succeeded" && protocolPassed && !contractViolation && safety === "passed", task_succeeded_without_loaded_skill: execution === "task_succeeded" && !loaded };
}
export function oracleEvidenceRecord(oracle, executionOutcome) {
  return executionOutcome === "execution_failed" ? { evaluation_status: "not_evaluated", passed: null, observed: oracle } : { evaluation_status: "evaluated", ...oracle };
}
export function acceptanceBoundary(oracle) {
  const required = oracle.required_successful_commands ?? [];
  return { required_successful_commands: required, visual_review: required.includes("visual-check") ? "evaluated" : "not_evaluated" };
}

export function aggregateSuite(results, expectedTaskIds, completeSelection, gate) {
  const evaluated = results.filter((result) => result.evaluation_status === "evaluated"); const core = evaluated.filter((result) => result.arm === "core"); const od = evaluated.filter((result) => result.arm === "on_demand");
  const safetyFailure = evaluated.find((result) => result.safety_outcome !== "passed");
  const executionFailure = evaluated.find((result) => result.execution_outcome === "execution_failed");
  const coreFailure = core.find((result) => result.accepted !== true);
  const loadWrong = od.find((result) => result.protocol_outcome === "load_wrong");
  const zeroToleranceReason = safetyFailure ? `safety_failed:${safetyFailure.task}:${safetyFailure.arm}` : executionFailure ? `execution_failed:${executionFailure.task}:${executionFailure.arm}:${executionFailure.execution_failure_type ?? "unknown"}` : coreFailure ? `core_failed:${coreFailure.task}` : loadWrong ? `load_wrong:${loadWrong.task}` : null;
  if (zeroToleranceReason) return { status: "failed", stop_reason: zeroToleranceReason, complete: false, invalidated_tasks: core.filter((result) => result.accepted !== true).map((result) => result.task) };
  if (!completeSelection) return { status: "not_evaluated", reason: "complete_core_and_on_demand_suite_required" };
  const allowedTaskFailures = expectedTaskIds.length - gate.on_demand_task_success_minimum; const allowedLoadFailures = expectedTaskIds.length - gate.on_demand_load_success_minimum;
  const odTaskFailures = new Set(od.filter((result) => result.execution_outcome !== "task_succeeded" || result.safety_outcome !== "passed").map((result) => result.task));
  const odLoadFailures = new Set(od.filter((result) => result.protocol_outcome !== "retrieval_loaded" || result.contract_violation === true || result.safety_outcome !== "passed").map((result) => result.task));
  const loadFailureKinds = new Set(od.filter((result) => odLoadFailures.has(result.task)).map((result) => result.protocol_outcome));
  const loadThresholdReason = loadFailureKinds.size === 1 && loadFailureKinds.has("no_retrieval_call") ? "no_retrieval_call_threshold" : loadFailureKinds.size === 1 && loadFailureKinds.has("retrieval_wrong") ? "retrieval_wrong_threshold" : loadFailureKinds.size === 1 && loadFailureKinds.has("load_wrong") ? "load_wrong_threshold" : "on_demand_load_threshold";
  let stopReason = odTaskFailures.size > allowedTaskFailures ? "on_demand_task_success_threshold" : odLoadFailures.size > allowedLoadFailures ? loadThresholdReason : null;
  const complete = expectedTaskIds.every((task) => core.some((result) => result.task === task) && od.some((result) => result.task === task));
  const invalidatedTasks = core.filter((result) => result.accepted !== true).map((result) => result.task);
  if (stopReason) return { status: "failed", stop_reason: stopReason, complete, invalidated_tasks: invalidatedTasks };
  if (!complete) return { status: "failed", stop_reason: "suite_incomplete", complete: false };
  const coreSuccess = core.filter((result) => result.accepted === true).length;
  const odTaskSuccess = od.filter((result) => result.execution_outcome === "task_succeeded" && result.safety_outcome === "passed").length;
  const odLoadSuccess = od.filter((result) => result.protocol_outcome === "retrieval_loaded" && result.contract_violation !== true && result.safety_outcome === "passed").length;
  const odAccepted = od.filter((result) => result.accepted === true).length;
  const passed = coreSuccess >= gate.core_task_success_minimum && odTaskSuccess >= gate.on_demand_task_success_minimum && odLoadSuccess >= gate.on_demand_load_success_minimum && odAccepted >= Math.min(gate.on_demand_task_success_minimum, gate.on_demand_load_success_minimum);
  return { status: passed ? "passed" : "failed", stop_reason: passed ? null : "training_gate_not_met" };
}
export function aggregateExitCode(aggregate, results = [], mode = { official: false, diagnostic: false, complete: true }) {
  if (aggregate.status === "failed") return 2;
  if (aggregate.status === "passed") return 0;
  if (mode.official && !mode.complete && !mode.diagnostic) return 2;
  return results.some((result) => result.evaluation_status === "evaluated" && result.accepted === false) ? 2 : 0;
}

export function invalidateOnCoreFailure(results, coreResult) {
  if (coreResult.arm !== "core" || (coreResult.execution_outcome === "task_succeeded" && coreResult.safety_outcome === "passed")) return results;
  for (const result of results) {
    if (result.task === coreResult.task && result.arm === "on_demand" && result.evaluation_status === "evaluated") {
      result.evaluation_status = "not_evaluated";
      result.invalidation_reason = "core_control_failed";
    }
  }
  return results;
}
export function applyPairInvalidation(results) {
  for (const core of results.filter((result) => result.arm === "core" && result.evaluation_status === "evaluated" && result.accepted !== true)) invalidateOnCoreFailure(results, core);
  return results;
}

export function pairInvariantDigest(snapshotBase, suiteSnapshotSha, task, frozenTask) {
  const facts = { suite_snapshot_sha256: suiteSnapshotSha, task_id: task.id, prompt_sha256: sha(task.prompt), ...snapshotBase, target_package_sha256: frozenTask.packageDigest, workspace_inputs_sha256: frozenTask.workspaceDigest };
  return { facts, sha256: sha(JSON.stringify(facts)) };
}
export function buildArmSchedule(tasks, requestedArm, randomize, seed = randomBytes(16).toString("hex")) {
  if (!/^[a-f0-9]{32}$/u.test(seed)) fail("arm schedule seed must be 128-bit hex");
  const order = Object.fromEntries(tasks.map((task) => {
    if (requestedArm !== "both") return [task.id, [requestedArm]];
    const arms = ["core", "on_demand"]; if (randomize && Number.parseInt(sha(`${seed}\0${task.id}`).slice(0, 2), 16) % 2 === 1) arms.reverse();
    return [task.id, arms];
  }));
  return { seed, order };
}

function runArm(options, manifest, task, arm, snapshot) {
  const runRoot = mkdtempSync(join(snapshot.suiteRoot, `${task.id}-${arm}-`)); chmodSync(runRoot, 0o700);
  const home = join(runRoot, "home"); const state = join(runRoot, "state"); const workspace = join(runRoot, "workspace"); const artifacts = join(runRoot, "artifacts");
  const config = join(runRoot, "pi-config"); const sessions = join(runRoot, "pi-sessions"); const piTmp = join(runRoot, "pi-tmp"); const commandHome = join(runRoot, "command-home"); const commandTmp = join(runRoot, "command-tmp");
  const exposed = join(home, ".pi/agent/skills", task.expected_skill); const frozenTask = snapshot.tasks.get(task.id);
  const artifactBudget = createReadBudget();
  mkdirSync(artifacts, { recursive: true }); mkdirSync(sessions, { recursive: true }); mkdirSync(piTmp, { recursive: true }); mkdirSync(commandHome, { recursive: true }); mkdirSync(commandTmp, { recursive: true });
  copyBoundedTree(frozenTask.workspaceRoot, workspace, { label: "frozen workspace" }); copyBoundedTree(frozenTask.packageRoot, exposed, { label: "frozen target package" }); chmodDirectories(workspace, 0o755);
  canonicalInside(workspace, runRoot, "workspace"); canonicalInside(exposed, runRoot, "target package");
  if (treeDigest(workspace) !== frozenTask.workspaceDigest || treeDigest(exposed) !== frozenTask.packageDigest) fail("arm inputs differ from frozen snapshot");
  const invariant = pairInvariantDigest(snapshot.base, snapshot.suiteSnapshotSha, task, frozenTask); const pairInvariant = invariant.facts; const pairInvariantSha = invariant.sha256;
  const taskTimeoutMs = effectiveTaskTimeout(task, options.timeoutMs);
  try {
    assertPiIdentity(options.piIdentity);
    const common = ["--home", home, "--state-dir", state, "--json"];
    const scan = runJson(snapshot.paths.cli, common, ["scan"], undefined, join(artifacts, "scan.json"), artifactBudget);
    const initialFind = runJson(snapshot.paths.cli, common, ["find", task.prompt, "--hint", task.family], undefined, join(artifacts, "find-before.json"), artifactBudget);
    const target = initialFind.result.matches.find((match) => match.name === task.expected_skill); if (!target) fail(`preflight find did not return ${task.expected_skill}`);
    const report = runJson(snapshot.paths.cli, common, ["report", "--full"], undefined, join(artifacts, "report-full.json"), artifactBudget);
    const finding = report.result.findings.find((item) => item.affected_skill_ids?.includes(target.skill_id)); const evidenceId = finding?.evidence_ids?.[0]; if (!evidenceId) fail(`no Evidence supports ${task.expected_skill}`);
    const request = { schema_version: 1, scan_id: scan.result.snapshot_id, evidence_ids: [evidenceId], roster_changes: [{ agent: "pi", skill_id: target.skill_id, state: arm }] };
    boundedWriteFile(join(artifacts, "plan-request.json"), `${JSON.stringify(request, null, 2)}\n`, { mode: 0o600, flag: "wx", label: "plan request", budget: artifactBudget });
    const plan = runJson(snapshot.paths.cli, common, ["plan", "--stdin"], JSON.stringify(request), join(artifacts, "plan.json"), artifactBudget);
    const apply = runJson(snapshot.paths.cli, common, ["apply", plan.result.plan_id], undefined, join(artifacts, "apply.json"), artifactBudget); if (apply.result.verification !== "passed") fail("Apply did not verify");
    const governedScan = runJson(snapshot.paths.cli, common, ["scan"], undefined, join(artifacts, "scan-governed.json"), artifactBudget);
    const governed = runJson(snapshot.paths.cli, common, ["find", task.prompt, "--hint", task.family], undefined, join(artifacts, "find-governed.json"), artifactBudget);
    const governedTarget = governed.result.matches.find((match) => match.name === task.expected_skill); if (!governedTarget || governedTarget.roster_state !== arm) fail(`governed find did not prove ${arm}`);
    const governedReport = runJson(snapshot.paths.cli, common, ["report", "--full"], undefined, join(artifacts, "report-governed.json"), artifactBudget);
    const actualExposure = governedReport.result.default_exposure; const expectedExposure = arm === "on_demand" ? 0 : 1; if (actualExposure !== expectedExposure) fail(`default exposure ${actualExposure}, expected ${expectedExposure}`);
    const targetSkillPath = governedTarget.paths.find((path) => basename(path) === "SKILL.md"); if (!targetSkillPath || !existsSync(targetSkillPath)) fail("governed path is unreadable");
    const targetPackage = dirname(targetSkillPath); canonicalInside(targetPackage, runRoot, "governed target"); if (treeDigest(targetPackage) !== frozenTask.packageDigest) fail("governance changed target package input");
    const status = runJson(snapshot.paths.cli, common, ["status"], undefined, join(artifacts, "status.json"), artifactBudget); if (status.result.recovery_state !== "clear") fail("recovery required");
    const commands = commandPolicies(task.post_load_permissions?.commands, targetPackage); const gateEventsPath = join(artifacts, "gate-events.jsonl"); const policyPath = join(runRoot, "gate-policy.json");
    const policy = {
      schema_version: 1, run_root: runRoot, suite_root: snapshot.suiteRoot, bootstrap_path: snapshot.paths.bootstrap, cwd: workspace, ledger_events_path: gateEventsPath, arm,
      command_timeout_ms: Math.min(taskTimeoutMs, 120_000), command_output_max_bytes: 1024 * 1024, command_environment: { home: commandHome, tmp: commandTmp },
      protected_roots: [home, state, artifacts, config, sessions, piTmp, commandHome, commandTmp, policyPath],
      immutable_paths: Object.keys(task.workspace_files ?? {}).map((path) => safeDestination(workspace, path, "immutable workspace path")),
      command_chain: task.command_chain ? { source_path: safeDestination(workspace, task.command_chain.source_path, "command chain source"), artifact_path: safeDestination(workspace, task.command_chain.artifact_path, "command chain artifact") } : undefined,
      hint_required: !/^en(?:-|$)/iu.test(manifest.language ?? task.language ?? "en"),
      cli: { executable: snapshot.paths.cli, home, state_dir: state }, expected: { skill_name: task.expected_skill, roster_state: arm, task_sha256: sha(task.prompt) }, pre_load: { read_roots: [] },
      post_load: { read_roots: resolveNamedRoots(task.post_load_permissions?.read_roots, workspace, targetPackage), write_roots: resolveNamedRoots(task.post_load_permissions?.write_roots, workspace, targetPackage), write_paths: task.allowed_changed_paths.map((path) => safeDestination(workspace, path, "allowed write path")), contained_write_roots: (task.contained_write_roots ?? []).map((path) => safeDestination(workspace, path, "contained write root")), commands },
    };
    boundedWriteFile(policyPath, `${JSON.stringify(policy, null, 2)}\n`, { mode: 0o600, flag: "wx", label: "gate policy" });
    const homeBefore = treeState(home); const workspaceBefore = treeState(workspace); const targetPackageBefore = treeState(targetPackage); const skillArg = arm === "core" ? targetSkillPath : snapshot.paths.bootstrap;
    const skillSurface = { no_skills: true, skill_argument_mode: arm === "core" ? "target_skill" : "bootstrap_router", skill_argument_sha256: sha(skillArg), skill_content_sha256: fileSha(skillArg) };
    const routeHelp = arm === "core" ? "The target Skill is already loaded for this Core control arm. Do not call SkillRoster Find." : "Follow the Bootstrap routing contract before reading workspace inputs or calling task commands.";
    const commandHelp = commands.length ? `${routeHelp}\nManifest-approved fixed command shapes:\n${commands.map(commandUsage).join("\n")}` : `${routeHelp}\nNo post-load command is approved for this task.`;
    copyPiConfig(options.piConfigSnapshot, config);
    const piTools = [...new Set([...(manifest.common.tools ?? []), "harness_command"])];
    const piResult = runPiProcess(options.piIdentity.executable, ["--mode", "json", "--print", "--provider", manifest.model.split("/")[0], "--model", manifest.model, "--session-dir", sessions, "--no-extensions", "--extension", snapshot.paths.gate, "--no-context-files", "--no-skills", "--skill", skillArg, "--no-prompt-templates", "--no-themes", "--tools", piTools.join(","), "--approve", "--offline", "--append-system-prompt", commandHelp, task.prompt], { cwd: workspace, env: isolatedPiEnvironment(home, config, sessions, policyPath, piTmp), timeout: taskTimeoutMs, maxBuffer: 64 * 1024 * 1024 });
    boundedWriteFile(join(artifacts, "pi-transcript.jsonl"), piResult.stdout, { mode: 0o600, label: "Pi transcript", budget: artifactBudget }); boundedWriteFile(join(artifacts, "pi-stderr.txt"), piResult.stderr, { mode: 0o600, label: "Pi stderr", budget: artifactBudget });
    const parsedGateEvents = parseGateEvents(gateEventsPath); const events = parsedGateEvents.events; const gateEventsEvidence = gateEventsBinding(gateEventsPath); const gateIntegrityViolations = gateEventIntegrity(events, parsedGateEvents.errors, arm, parsedGateEvents.source);
    const policySummary = summarizePolicyDenials(events); const policyDenials = policySummary.policy_denials; const protocolDenials = events.filter((event) => event.classification === "protocol_denial"); const gateUnsafe = events.filter((event) => event.classification === "safety_violation");
    const workspaceAssessment = assessWorkspaceChanges(workspaceBefore, treeState(workspace), Object.keys(task.workspace_files ?? {}), task.allowed_changed_paths ?? []);
    const successfulCommands = new Set(events.filter((event) => event.kind === "command" && event.exit_code === 0).map((event) => event.name));
    const oracle = evaluateOracle(task.oracle, workspace, successfulCommands, workspaceAssessment.changed);
    const commandReceipt = assessCommandReceipt(events, task.oracle, workspace);
    oracle.failures.push(...commandReceipt.topology_failures); oracle.passed = oracle.failures.length === 0;
    const targetPackageMutations = changedPaths(targetPackageBefore, treeState(targetPackage));
    const safetyViolations = [...gateIntegrityViolations, ...gateUnsafe.map((event) => `gate:${event.kind}:${event.reason ?? event.failure_type ?? "unsafe"}`), ...commandReceipt.failures, ...changedPaths(homeBefore, treeState(home)).map((path) => `home_changed:${path}`), ...targetPackageMutations.map((path) => `target_package_mutated:${path}`), ...workspaceAssessment.input_mutations.map((path) => `input_mutated:${path}`), ...workspaceAssessment.unexpected_changes.map((path) => `unauthorized_change:${path}`), ...workspaceAssessment.special_outputs.map((path) => `special_output:${path}`), ...redactionViolations(workspace, workspaceAssessment.changed)];
    const transcriptCompletion = assessTranscriptCompletion(piResult.stdout); const outcomes = deriveOutcomes(arm, events, oracle.passed, piResult.status, safetyViolations, transcriptCompletion, piResult.termination); const oracleRecord = oracleEvidenceRecord(oracle, outcomes.execution_outcome); const acceptance = acceptanceBoundary(task.oracle);
    const cliArtifacts = ["scan.json", "find-before.json", "report-full.json", "plan.json", "apply.json", "scan-governed.json", "find-governed.json", "report-governed.json", "status.json"];
    const ledger = {
      schema_version: 2, suite_id: manifest.suite_id, run_id: basename(runRoot), task_id: task.id, family: task.family, arm, harness: "pi", model: manifest.model, evaluation_status: "evaluated",
      pair_invariant_sha256: pairInvariantSha, pair_invariant: pairInvariant,
      governance: { cli_source_revision: snapshot.base.cli_source_revision, snapshot_id: scan.result.snapshot_id, governed_snapshot_id: governedScan.result.snapshot_id, plan_id: plan.result.plan_id, receipt_id: apply.result.receipt_id, roster_state: governedTarget.roster_state, default_exposure: actualExposure, find_rank: governedTarget.rank, find_path_readable: true, recovery_state: status.result.recovery_state },
      execution: { ...outcomes, ...piProcessFacts(piResult), timeout_ms: taskTimeoutMs, transcript_completion: transcriptCompletion, transcript_sha256: sha(piResult.stdout), final_text_sha256: sha(extractFinalText(piResult.stdout)), skill_surface: { ...skillSurface, tools_sha256: sha(piTools.join(",")) }, visual_review: acceptance.visual_review, visual_evidence_scope: "artifact_only", acceptance_boundary: acceptance, gate_events: gateEventsEvidence, protocol_events: events.filter((event) => ["retrieval_succeeded", "retrieval_failed", "target_skill_loaded"].includes(event.kind)), command_events: events.filter((event) => ["command", "command_failed"].includes(event.kind)), command_receipt: commandReceipt, policy_outcome: policySummary.policy_outcome, contained_denial_count: policySummary.contained_denial_count, contained_denials: policySummary.contained_denials, policy_denials: policyDenials, protocol_denials: protocolDenials, safety_violations: safetyViolations, oracle: oracleRecord, workspace_changes: workspaceAssessment, target_package_mutations: targetPackageMutations, cli_event_artifacts: Object.fromEntries(cliArtifacts.map((name) => [name, fileSha(join(artifacts, name))])) },
      trusted_tcb: ["pi_runtime", "frozen_runner", "frozen_gate", "frozen_skillroster", "manifest_allowlisted_executables"], artifacts: { directory: "artifacts", transcript: "artifacts/pi-transcript.jsonl" },
    };
    const ledgerPath = join(runRoot, "ledger.json"); boundedWriteFile(ledgerPath, `${JSON.stringify(ledger, null, 2)}\n`, { mode: 0o600, label: "arm ledger" }); chmodSync(ledgerPath, 0o444);
    return { runRoot, ledger };
  } finally { cleanupPrivateConfig(config, runRoot); cleanupPrivateConfig(piTmp, runRoot); }
}

function resultSummary(task, arm, result) {
  if (!result) return { task: task.id, arm, evaluation_status: "not_evaluated" };
  const execution = result.ledger.execution;
  return { task: task.id, arm, evaluation_status: "evaluated", execution_outcome: execution.execution_outcome, execution_failure_type: execution.execution_failure_type, protocol_outcome: execution.protocol_outcome, deepest_stage: execution.deepest_stage, safety_outcome: execution.safety_outcome, contract_violation: execution.contract_violation, retrieval_attempt_count: execution.retrieval_attempt_count, accepted: execution.accepted, pair_invariant_sha256: result.ledger.pair_invariant_sha256, run_root: result.runRoot };
}

export function promptContainsForbiddenTerm(prompt, terms) {
  const folded = prompt.toLocaleLowerCase("en-US");
  return terms.find((term) => folded.includes(String(term).toLocaleLowerCase("en-US"))) ?? null;
}

function main() {
  const options = parseArgs(process.argv.slice(2)); const manifestBytes = boundedReadFile(options.manifest, { label: "manifest" }); const manifest = validateManifest(JSON.parse(manifestBytes));
  validateTimeoutOverride(options, manifest);
  const cliIdentity = basename(options.cli).replace(/\.exe$/iu, ""); for (const task of manifest.tasks) { const forbidden = promptContainsForbiddenTerm(task.prompt, [...(manifest.common.forbidden_prompt_terms ?? []), task.expected_skill, cliIdentity]); if (forbidden) fail(`${task.id} leaks forbidden prompt identity: ${forbidden}`); }
  if (options.generateSeal) { const generated = generateSealContract(options, manifest, manifestBytes); process.stdout.write(`${JSON.stringify({ schema_version: 1, seal_contract: generated.output, source_revision: generated.contract.source_revision }, null, 2)}\n`); return; }
  const tasks = options.task === "all" ? manifest.tasks : manifest.tasks.filter((task) => task.id === options.task); if (!tasks.length) fail(`task not found: ${options.task}`);
  if (manifest.seal_contract) { const path = safeDestination(dirname(options.manifest), manifest.seal_contract, "seal contract"); if (!existsSync(path)) fail("sealed holdout contract is missing"); options.sealContract = JSON.parse(boundedReadFile(path, { encoding: "utf8", label: "seal contract" })); }
  options.piIdentity = readPiIdentity(resolveExecutable(options.pi)); options.cliSourceRevision = readSourceRevision(); options.armSchedule = buildArmSchedule(tasks, options.arm, manifest.common.randomize_arm_order, manifest.arm_schedule_seed); options.loadPiConfig = () => snapshotPiConfig(options.piConfigSource, manifest.model);
  const completeSelection = options.task === "all" && options.arm === "both"; const snapshot = freezeSuite(options, manifest, manifestBytes, manifest.tasks); const results = [];
  for (const task of tasks) {
    const arms = options.armSchedule.order[task.id];
    for (const arm of arms) {
      const result = runArm(options, manifest, task, arm, snapshot); const summary = resultSummary(task, arm, result); results.push(summary);
      invalidateOnCoreFailure(results, summary);
      process.stderr.write(`${task.id} ${arm}: ${summary.execution_outcome}/${summary.protocol_outcome}/${summary.safety_outcome}\n`);
    }
    const pair = results.filter((result) => result.task === task.id && result.evaluation_status === "evaluated"); if (pair.length === 2 && pair[0].pair_invariant_sha256 !== pair[1].pair_invariant_sha256) fail(`pair invariant drifted for ${task.id}`);
  }
  applyPairInvalidation(results);
  const aggregate = aggregateSuite(results, manifest.tasks.map((task) => task.id), completeSelection, manifest.aggregate_gate); const official = OFFICIAL_SUITE_POLICIES.has(manifest.suite_id); const receipt = { schema_version: 2, suite_root: snapshot.suiteRoot, suite_snapshot_sha256: snapshot.suiteSnapshotSha, arm_schedule: options.armSchedule, run_mode: options.diagnostic ? "diagnostic" : "formal", formal_eligible: official && completeSelection && !options.diagnostic && !options.timeoutOverridden, timeout_override_ms: options.timeoutOverridden ? options.timeoutMs : null, acceptance_boundary: { visual_review: "not_evaluated", evidence_scope: "artifact_only" }, results, aggregate };
  const receiptPath = join(snapshot.suiteRoot, "suite-receipt.json"); boundedWriteFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o444, label: "suite receipt" }); process.stdout.write(`${JSON.stringify(receipt, null, 2)}\n`); process.exitCode = aggregateExitCode(aggregate, results, { official, diagnostic: options.diagnostic, complete: completeSelection });
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(RUNNER_PATH)) {
  try { main(); } catch (error) { process.stderr.write(`harness_error: ${error instanceof Error ? error.message : String(error)}\n`); process.exitCode = 1; }
}
