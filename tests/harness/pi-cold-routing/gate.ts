import { appendFileSync, existsSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { spawn } from "node:child_process";

type RootMode = "read" | "write";

type ArgumentRule =
  | { kind: "literal"; value: string }
  | { kind: "enum"; values: string[] }
  | { kind: "string"; pattern?: string }
  | { kind: "read_path" }
  | { kind: "write_path" };

type CommandPolicy = {
  name: string;
  executable: string;
  fixed_argv?: string[];
  arguments?: ArgumentRule[];
};

type GatePolicy = {
  schema_version: 1;
  run_root: string;
  suite_root: string;
  bootstrap_path: string;
  cwd: string;
  ledger_events_path: string;
  arm: "core" | "on_demand";
  cli: { executable: string; home: string; state_dir: string };
  expected: { skill_name: string; roster_state: "core" | "on_demand"; task_sha256: string };
  hint_required: boolean;
  command_timeout_ms: number;
  command_output_max_bytes: number;
  command_environment: { home: string; tmp: string };
  protected_roots: string[];
  immutable_paths: string[];
  command_chain?: { source_path: string; artifact_path: string };
  pre_load?: { read_roots?: string[] };
  post_load?: {
    read_roots?: string[];
    write_roots?: string[];
    write_paths?: string[];
    contained_write_roots?: string[];
    commands?: CommandPolicy[];
  };
};

type GateState = {
  stage: "initial" | "retrieval_called" | "retrieval_correct" | "retrieval_wrong" | "task_loaded";
  returnedSkillPaths: string[];
  validatedCommand?: { source_path: string; source_sha256: string; chain_sha256: string; validator_receipt_sha256: string; graph_facts: ArchitectureGraphFacts };
};

type ArchitectureGraphFacts = {
  component_count: number;
  components: string[];
  boundary_count: number;
  boundaries: Array<{ label: string; wraps: string[] }>;
  connections: Array<{ from: string; to: string; label: string; variant: string }>;
  has_directed_cycle: boolean;
};

export function retrievalStageAfter(
  arm: "core" | "on_demand",
  current: GateState["stage"],
  requested: GateState["stage"],
): GateState["stage"] {
  return arm === "core" ? current : requested;
}

export function isRouteOrderViolation(arm: "core" | "on_demand", stage: GateState["stage"]): boolean {
  return arm === "on_demand" && stage !== "task_loaded";
}

function within(candidate: string, root: string): boolean {
  const suffix = relative(root, candidate);
  return suffix === "" || (!suffix.startsWith(`..${sep}`) && suffix !== ".." && !isAbsolute(suffix));
}

function nearestExisting(path: string): { real: string; remainder: string[] } {
  let cursor = path;
  const remainder: string[] = [];
  while (!existsSync(cursor)) {
    const parent = dirname(cursor);
    if (parent === cursor) throw new Error(`no existing ancestor for ${path}`);
    remainder.unshift(relative(parent, cursor));
    cursor = parent;
  }
  return { real: realpathSync(cursor), remainder };
}

/** Resolve a path through symlinks and prove it remains inside one declared root. */
export function canonicalPathInRoots(input: string, roots: string[], mode: RootMode): string {
  if (!isAbsolute(input)) throw new Error("path must be absolute");
  if (roots.length === 0) throw new Error(`no ${mode} roots are enabled`);
  const lexical = resolve(input);
  const canonicalRoots = roots.map((root) => {
    if (!isAbsolute(root) || !existsSync(root)) throw new Error(`invalid ${mode} root`);
    return realpathSync(root);
  });

  let canonical: string;
  if (mode === "read") {
    if (!existsSync(lexical)) throw new Error("read target does not exist");
    canonical = realpathSync(lexical);
  } else {
    const lexicalEntry = lstatSync(lexical, { throwIfNoEntry: false });
    if (lexicalEntry?.isSymbolicLink()) throw new Error("write target must not be a symbolic link");
    const found = nearestExisting(lexical);
    canonical = resolve(found.real, ...found.remainder);
  }

  if (!canonicalRoots.some((root) => within(canonical, root))) {
    throw new Error(`${mode} target escapes declared roots`);
  }
  return canonical;
}

const SHELL_SYNTAX = /[$`;&|<>\\\r\n*?()[\]{}!]/u;

export function containsUnquotedShellSyntax(command: string): boolean {
  let quote: "'" | '"' | null = null;
  for (const char of command) {
    if (quote) { if (char === quote) quote = null; continue; }
    if (char === "'" || char === '"') { quote = char; continue; }
    if (SHELL_SYNTAX.test(char)) return true;
  }
  return false;
}

/** Minimal shell-word reader. Operators, substitutions, escapes, and newlines are rejected. */
export function parseFindCommand(command: string): { task: string; hints: string[] } {
  if (containsUnquotedShellSyntax(command)) {
    throw new Error("shell syntax is not allowed");
  }
  const words: string[] = [];
  let word = "";
  let quote: "'" | '"' | null = null;
  let started = false;
  for (const char of command.trim()) {
    if (quote) {
      if (char === quote) quote = null;
      else word += char;
      started = true;
    } else if (char === "'" || char === '"') {
      quote = char;
      started = true;
    } else if (/\s/u.test(char)) {
      if (started) {
        words.push(word);
        word = "";
        started = false;
      }
    } else {
      word += char;
      started = true;
    }
  }
  if (quote) throw new Error("unterminated quote");
  if (started) words.push(word);
  if (words[0] !== "skillroster" || words[1] !== "find") {
    throw new Error("only the literal skillroster find command is allowed");
  }

  const positional: string[] = [];
  const hints: string[] = [];
  for (let index = 2; index < words.length; index += 1) {
    const value = words[index];
    if (value === "--json") continue;
    if (value === "--hint") {
      const hint = words[index + 1];
      if (!hint || hint.startsWith("--")) throw new Error("--hint requires text");
      hints.push(hint);
      index += 1;
      continue;
    }
    if (value.startsWith("--")) throw new Error(`unsupported find option: ${value}`);
    positional.push(value);
  }
  if (positional.length !== 1 || positional[0].length === 0) {
    throw new Error("find requires exactly one quoted task argument");
  }
  return { task: positional[0], hints };
}

export function findParseFailureClassification(command: string): "protocol_denial" | "safety_violation" {
  return containsUnquotedShellSyntax(command) ? "safety_violation" : "protocol_denial";
}

export function violatesHintContract(hintRequired: boolean, hintCount: number): boolean {
  return hintRequired && hintCount !== 1;
}

export function commandArgumentFailureClassification(error: unknown): "protocol_denial" | "safety_violation" {
  if ((error as any)?.policyClassification === "safety_violation") return "safety_violation";
  return /escapes declared roots|symbolic link/u.test(String(error)) ? "safety_violation" : "protocol_denial";
}

export function processFailureType(error: any): string {
  return typeof error?.failureType === "string" ? error.failureType : "spawn_error";
}

export function commandFailureDetail(name: string, failureType: string, stage: "argument_validation" | "execution", validatedArgs: string[] | null): Record<string, unknown> {
  return { name, failure_type: failureType, stage, args_sha256: validatedArgs === null ? null : validatedArgs.map(digest) };
}

export function boundedCommandDiagnostics(stdout: string, stderr: string, maxBytes = 8192): string {
  const take = (value: string, budget: number) => Buffer.from(value).subarray(0, budget).toString("utf8");
  const labels = "stdout:\n\nstderr:\n"; const contentBudget = Math.max(0, maxBytes - Buffer.byteLength(labels));
  const stdoutBudget = Math.floor(contentBudget / 2); const stderrBudget = contentBudget - stdoutBudget;
  return `stdout:\n${take(stdout, stdoutBudget)}\nstderr:\n${take(stderr, stderrBudget)}`;
}

export function validateCommandArguments(
  args: string[],
  rules: ArgumentRule[],
  readRoots: string[],
  writeRoots: string[],
  cwd?: string,
  allowedWritePaths: string[] = [],
): string[] {
  if (args.length !== rules.length) throw new Error("command argument count does not match policy");
  return args.map((value, index) => {
    const rule = rules[index];
    switch (rule.kind) {
      case "literal":
        if (value !== rule.value) throw new Error("literal command argument does not match policy");
        return value;
      case "enum":
        if (!rule.values.includes(value)) throw new Error("command argument is outside its enum");
        return value;
      case "string":
        if (/[\0\r\n]/u.test(value)) throw new Error("command string contains a control character");
        if (rule.pattern && !new RegExp(rule.pattern, "u").test(value)) {
          throw new Error("command string does not match policy");
        }
        return value;
      case "read_path":
      case "write_path": { // Path failures need lexical/canonical context to distinguish bad arguments from escape attempts.
        const mode = rule.kind === "read_path" ? "read" : "write";
        const roots = mode === "read" ? readRoots : writeRoots;
        const candidate = isAbsolute(value) ? value : resolve(cwd ?? "", value);
        try {
          const canonical = canonicalPathInRoots(candidate, roots, mode);
          if (mode === "write" && allowedWritePaths.length > 0 && !allowedWritePaths.some((path) => canonicalCandidate(path) === canonical)) {
            throw Object.assign(new Error("write target is not explicitly allowlisted"), { policyClassification: "safety_violation" });
          }
          return canonical;
        }
        catch (error) {
          let outside = true;
          try { const canonical = canonicalCandidate(candidate); outside = !roots.some((root) => within(canonical, realpathSync(root))); } catch { /* unsafe by default */ }
          if (outside) Object.assign(error as object, { policyClassification: "safety_violation" });
          throw error;
        }
      }
    }
  });
}

function minimalChildEnvironment(policy: GatePolicy): Record<string, string> {
  const env: Record<string, string> = {
    HOME: policy.command_environment.home,
    TMPDIR: policy.command_environment.tmp,
    PATH: process.env.PATH ?? "/usr/bin:/bin",
  };
  for (const name of ["LANG", "LC_ALL", "SSL_CERT_FILE"]) {
    if (process.env[name]) env[name] = process.env[name] as string;
  }
  return env;
}

export function runAllowlistedProcess(
  policy: GatePolicy,
  executable: string,
  argv: string[],
  signal?: AbortSignal,
): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolveRun, reject) => {
    const child = spawn(executable, argv, {
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      signal,
      env: minimalChildEnvironment(policy),
    });
    let timedOut = false; let outputLimitExceeded = false; let settled = false;
    const timeout = setTimeout(() => { timedOut = true; child.kill("SIGKILL"); }, policy.command_timeout_ms);
    let stdout = ""; let stderr = ""; let outputBytes = 0;
    const capture = (stream: "stdout" | "stderr", chunk: Buffer | string) => {
      const bytes = Buffer.from(chunk); const remaining = Math.max(0, policy.command_output_max_bytes - outputBytes);
      const accepted = bytes.subarray(0, remaining).toString("utf8"); if (stream === "stdout") stdout += accepted; else stderr += accepted;
      outputBytes += bytes.length;
      if (outputBytes > policy.command_output_max_bytes && !outputLimitExceeded) { outputLimitExceeded = true; child.kill("SIGKILL"); }
    };
    child.stdout.on("data", (chunk) => capture("stdout", chunk));
    child.stderr.on("data", (chunk) => capture("stderr", chunk));
    child.on("error", (error) => {
      if (settled) return; settled = true; clearTimeout(timeout);
      reject(Object.assign(new Error("allowlisted process failed to spawn"), { failureType: error.name === "AbortError" ? "signal" : "spawn_error", cause: error }));
    });
    child.on("close", (code, signalName) => {
      if (settled) return; settled = true; clearTimeout(timeout);
      if (outputLimitExceeded) reject(Object.assign(new Error("allowlisted command output exceeded the streaming cap"), { failureType: "output_limit", outputBytes }));
      else if (timedOut) reject(Object.assign(new Error("allowlisted command timed out"), { failureType: "timeout" }));
      else if (signalName) reject(Object.assign(new Error(`allowlisted command terminated by ${signalName}`), { failureType: "signal", signalName }));
      else resolveRun({ code: code ?? 1, stdout, stderr });
    });
  });
}

function digest(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

export function architectureGraphFacts(spec: any): ArchitectureGraphFacts {
  if (spec?.schema_version !== 1 || spec?.diagram_type !== "architecture" || !Array.isArray(spec.components) || !Array.isArray(spec.connections) || !Array.isArray(spec.boundaries)) throw new Error("validated architecture source has no canonical graph");
  const ids = new Map<string, string>();
  for (const component of spec.components) {
    if (typeof component?.id !== "string" || typeof component?.label !== "string" || ids.has(component.id)) throw new Error("architecture components are not uniquely identified");
    ids.set(component.id, component.label);
  }
  const connections = spec.connections.map((edge: any) => {
    if (!ids.has(edge?.from) || !ids.has(edge?.to)) throw new Error("architecture connection references an unknown component");
    return { from: ids.get(edge.from) as string, to: ids.get(edge.to) as string, label: typeof edge.label === "string" ? edge.label : "", variant: typeof edge.variant === "string" ? edge.variant : "default" };
  }).sort((left: any, right: any) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
  const boundaries = spec.boundaries.map((boundary: any) => {
    if (typeof boundary?.label !== "string" || !Array.isArray(boundary.wraps) || boundary.wraps.some((id: unknown) => typeof id !== "string" || !ids.has(id))) throw new Error("architecture boundary is invalid");
    return { label: boundary.label, wraps: boundary.wraps.map((id: string) => ids.get(id) as string).sort() };
  }).sort((left: any, right: any) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
  const adjacency = new Map([...ids.keys()].map((id) => [id, [] as string[]])); for (const edge of spec.connections) adjacency.get(edge.from)?.push(edge.to);
  const visiting = new Set<string>(); const visited = new Set<string>();
  const cyclic = (id: string): boolean => { if (visiting.has(id)) return true; if (visited.has(id)) return false; visiting.add(id); if (adjacency.get(id)?.some(cyclic)) return true; visiting.delete(id); visited.add(id); return false; };
  return { component_count: ids.size, components: [...ids.values()].sort(), boundary_count: boundaries.length, boundaries, connections, has_directed_cycle: [...ids.keys()].some(cyclic) };
}

export function validatedArchitectureEvidence(stdout: string, sourcePath: string): { validator_receipt_sha256: string; graph_facts: ArchitectureGraphFacts } {
  let receipt: any; try { receipt = JSON.parse(stdout); } catch { throw new Error("validator stdout is not a JSON receipt"); }
  if (receipt?.schemaVersion !== 1 || receipt?.ok !== true || receipt?.command !== "validate" || canonicalCandidate(receipt?.input ?? "") !== canonicalCandidate(sourcePath)) throw new Error("validator receipt does not bind the validated source");
  return { validator_receipt_sha256: digest(stdout), graph_facts: architectureGraphFacts(JSON.parse(readFileSync(sourcePath, "utf8"))) };
}

export function retrievalFailureType(taskMismatch: boolean, selectedValid: boolean): "task_mismatch" | "wrong_result" | null {
  if (taskMismatch) return "task_mismatch";
  return selectedValid ? null : "wrong_result";
}

function canonicalCandidate(input: string): string {
  const lexical = resolve(input);
  if (existsSync(lexical)) return realpathSync(lexical);
  const found = nearestExisting(lexical);
  return resolve(found.real, ...found.remainder);
}

export function canonicalPolicyPathInRun(input: string, runRoot: string): string {
  const canonical = canonicalCandidate(input);
  const canonicalRunRoot = realpathSync(runRoot);
  if (!within(canonical, canonicalRunRoot)) throw new Error("gate policy path escapes run root");
  return canonical;
}

export function isExactInjectedBootstrapPath(input: string, bootstrapPath: string): boolean {
  if (input !== bootstrapPath || !existsSync(input) || !existsSync(bootstrapPath)) return false;
  return realpathSync(input) === realpathSync(bootstrapPath);
}

export function deniedFileAttemptClassification(
  tool: "read" | "write" | "edit",
  input: string,
  runRoot: string,
  futureReadRoots: string[] = [],
  protectedRoots: string[] = [],
): "policy_denial" | "safety_violation" {
  if (tool !== "read") return "safety_violation";
  try {
    const candidate = canonicalCandidate(input);
    if (!within(candidate, realpathSync(runRoot))) return "safety_violation";
    if (futureReadRoots.some((root) => within(candidate, realpathSync(root)))) return "policy_denial";
    if (protectedRoots.some((root) => within(candidate, realpathSync(root)))) return "safety_violation";
    return "safety_violation";
  } catch {
    return "safety_violation";
  }
}

export function classifyContainedWriteDenial(
  tool: "write" | "edit",
  input: string,
  canonical: string,
  containedRoots: string[] = [],
  immutablePaths: string[] = [],
  protectedRoots: string[] = [],
): { classification: "policy_denial" | "safety_violation"; contained: boolean } {
  try {
    if (tool !== "write" || existsSync(input)) return { classification: "safety_violation", contained: false };
    if (!containedRoots.some((root) => within(canonical, canonicalCandidate(root)))) return { classification: "safety_violation", contained: false };
    if (immutablePaths.some((path) => within(canonical, canonicalCandidate(path)))) return { classification: "safety_violation", contained: false };
    if (protectedRoots.some((root) => within(canonical, canonicalCandidate(root)))) return { classification: "safety_violation", contained: false };
    return { classification: "policy_denial", contained: true };
  } catch {
    return { classification: "safety_violation", contained: false };
  }
}

export function isSafePreRouteWriteDenial(
  input: string,
  writeRoots: string[] = [],
  writePaths: string[] = [],
  containedRoots: string[] = [],
  immutablePaths: string[] = [],
  protectedRoots: string[] = [],
): boolean {
  try {
    const canonical = canonicalPathInRoots(input, writeRoots, "write");
    return classifyContainedWriteDenial("write", input, canonical, [...containedRoots, ...writePaths], immutablePaths, protectedRoots).classification === "policy_denial";
  } catch {
    return false;
  }
}

export default function registerGate(pi: any): void {
  const policyPath = process.env.SKILLROSTER_PI_GATE_POLICY;
  if (!policyPath) throw new Error("SKILLROSTER_PI_GATE_POLICY is required");
  const policy = JSON.parse(readFileSync(policyPath, "utf8")) as GatePolicy;
  if (policy.schema_version !== 1) throw new Error("unsupported gate policy schema");
  if (!Number.isSafeInteger(policy.command_timeout_ms) || policy.command_timeout_ms < 1000) throw new Error("invalid command timeout");
  if (!Number.isSafeInteger(policy.command_output_max_bytes) || policy.command_output_max_bytes < 1024 || policy.command_output_max_bytes > 16 * 1024 * 1024) throw new Error("invalid command output cap");
  for (const path of [policy.cwd, policy.cli.home, policy.cli.state_dir, policy.ledger_events_path, policy.command_environment.home, policy.command_environment.tmp, ...(policy.protected_roots ?? []), ...(policy.immutable_paths ?? []), ...(policy.post_load?.write_paths ?? []), ...(policy.post_load?.contained_write_roots ?? []), ...Object.values(policy.command_chain ?? {})]) {
    canonicalPolicyPathInRun(path, policy.run_root);
  }
  if (!within(realpathSync(policy.bootstrap_path), realpathSync(policy.suite_root))) throw new Error("Bootstrap path escapes frozen suite root");
  if (!within(realpathSync(policy.cli.executable), realpathSync(policy.suite_root))) throw new Error("SkillRoster executable escapes frozen suite root");
  const state: GateState = {
    stage: policy.arm === "core" ? "task_loaded" : "initial",
    returnedSkillPaths: [],
  };
  const event = (kind: string, detail: Record<string, unknown> = {}) => {
    appendFileSync(
      policy.ledger_events_path,
      `${JSON.stringify({ schema_version: 1, kind, stage: state.stage, ...detail })}\n`,
      { encoding: "utf8", mode: 0o600 },
    );
  };
  const denial = (kind: string, detail: Record<string, unknown> = {}) =>
    event(kind, { classification: "policy_denial", ...detail });
  const protocolDenial = (kind: string, detail: Record<string, unknown> = {}) =>
    event(kind, { classification: "protocol_denial", ...detail });
  const unsafe = (kind: string, detail: Record<string, unknown> = {}) =>
    event(kind, { classification: "safety_violation", ...detail });
  const roots = () => ({
    read: [
      ...(policy.pre_load?.read_roots ?? []),
      ...(state.stage === "task_loaded" ? (policy.post_load?.read_roots ?? []) : []),
      ...state.returnedSkillPaths,
    ],
    write: state.stage === "task_loaded" ? (policy.post_load?.write_roots ?? []) : [],
  });

  pi.registerTool({
    name: "bash",
    label: "SkillRoster Find",
    description: "Run the read-only `skillroster find \"TASK\" --json` command. No other shell command is available.",
    parameters: {
      type: "object",
      properties: { command: { type: "string", description: "Literal skillroster find command" } },
      required: ["command"],
      additionalProperties: false,
    },
    async execute(_id: string, params: { command: string }, signal: AbortSignal) {
      let parsed: { task: string; hints: string[] };
      try {
        parsed = parseFindCommand(params.command);
      } catch (error) {
        const classification = findParseFailureClassification(params.command);
        const record = classification === "safety_violation" ? unsafe : protocolDenial;
        record(classification === "safety_violation" ? "find_blocked" : "retrieval_failed", { failure_type: "invalid_find_arguments", reason: String(error) });
        throw error;
      }
      state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_called");
      const taskMismatch = digest(parsed.task) !== policy.expected.task_sha256;
      if (violatesHintContract(policy.hint_required, parsed.hints.length)) {
        state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_wrong");
        protocolDenial("retrieval_failed", { failure_type: "hint_contract", contract_violation: true, hint_count: parsed.hints.length, task_mismatch: taskMismatch });
        throw new Error("exactly one --hint is required by the routing contract");
      }
      const argv = ["--home", policy.cli.home, "--state-dir", policy.cli.state_dir, "--json", "find", parsed.task];
      for (const hint of parsed.hints) argv.push("--hint", hint);
      let result: { code: number; stdout: string; stderr: string };
      try { result = await runAllowlistedProcess(policy, policy.cli.executable, argv, signal); }
      catch (error: any) {
        state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_wrong");
        event("retrieval_failed", { failure_type: processFailureType(error), task_mismatch: taskMismatch });
        throw error;
      }
      if (result.code !== 0) {
        state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_wrong");
        event("retrieval_failed", { failure_type: taskMismatch ? "task_mismatch" : "cli_error", exit_code: result.code, task_mismatch: taskMismatch });
        throw new Error(`skillroster find exited ${result.code}: ${result.stderr}`);
      }
      let envelope: any;
      try {
        envelope = JSON.parse(result.stdout);
      } catch (error) {
        state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_wrong");
        event("retrieval_failed", { failure_type: taskMismatch ? "task_mismatch" : "invalid_envelope", reason: String(error), task_mismatch: taskMismatch });
        throw error;
      }
      const matches = Array.isArray(envelope?.result?.matches) ? envelope.result.matches : [];
      const selected = matches.find((match: any) => match?.name === policy.expected.skill_name);
      const selectedValid = Boolean(selected && selected.roster_state === policy.expected.roster_state && Array.isArray(selected.paths));
      const retrievalFailure = retrievalFailureType(taskMismatch, selectedValid);
      if (retrievalFailure === null) {
        try {
          state.returnedSkillPaths = selected.paths.map((path: string) => canonicalPathInRoots(path, [policy.run_root], "read"));
          state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_correct");
          event("retrieval_succeeded", { task_sha256: digest(parsed.task), task_mismatch: false, hint_sha256: parsed.hints.map(digest), selected_rank: selected.rank ?? null, selected_name: selected.name });
        } catch (error) {
          state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_wrong");
          state.returnedSkillPaths = [];
          unsafe("retrieval_failed", { failure_type: "returned_path_escape", reason: String(error), task_mismatch: false });
        }
      } else {
        state.stage = retrievalStageAfter(policy.arm, state.stage, "retrieval_wrong");
        event("retrieval_failed", {
          failure_type: retrievalFailure,
          task_sha256: digest(parsed.task),
          task_mismatch: taskMismatch,
          hint_sha256: parsed.hints.map(digest),
          selected_rank: selected?.rank ?? null,
          selected_name: selected?.name ?? null,
        });
      }
      return { content: [{ type: "text", text: result.stdout }], details: { exit_code: result.code } };
    },
  });

  pi.registerTool({
    name: "harness_command",
    label: "Harness command",
    description: "Run one manifest-approved post-load command with structured arguments. Arbitrary shell is unavailable.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string" },
        args: { type: "array", items: { type: "string" } },
      },
      required: ["name", "args"],
      additionalProperties: false,
    },
    async execute(_id: string, params: { name: string; args: string[] }, signal: AbortSignal) {
      if (state.stage !== "task_loaded") {
        protocolDenial("command_blocked", { failure_type: "route_order", contract_violation: true, reason: "target_skill_not_loaded" });
        event("command_failed", commandFailureDetail("<unverified>", "route_order", "argument_validation", null));
        throw new Error("post-load commands require the selected Skill to be loaded");
      }
      const command = (policy.post_load?.commands ?? []).find((candidate) => candidate.name === params.name);
      if (!command) {
        unsafe("command_blocked", { reason: "not_allowlisted", name_sha256: digest(params.name) });
        throw new Error("command is not allowlisted by the task manifest");
      }
      if (!isAbsolute(command.executable)) {
        unsafe("command_blocked", { reason: "non_absolute_executable", name: command.name });
        throw new Error("allowlisted executable must be absolute");
      }
      const activeRoots = roots();
      let args: string[];
      try {
        args = validateCommandArguments(params.args, command.arguments ?? [], activeRoots.read, activeRoots.write, policy.cwd, policy.post_load?.write_paths ?? []);
      } catch (error) {
        const classification = commandArgumentFailureClassification(error);
        const record = classification === "safety_violation" ? unsafe : protocolDenial;
        record("command_blocked", { failure_type: classification === "safety_violation" ? "path_escape" : "invalid_arguments", reason: String(error), name: command.name });
        event("command_failed", commandFailureDetail(command.name, classification === "safety_violation" ? "path_escape" : "invalid_arguments", "argument_validation", null));
        throw error;
      }
      let sourceDigest: string | null = null;
      if (policy.command_chain && ["validate", "deliver"].includes(command.name)) {
        const expectedSource = canonicalCandidate(policy.command_chain.source_path); const expectedArtifact = canonicalCandidate(policy.command_chain.artifact_path);
        if (args[1] !== expectedSource || (command.name === "deliver" && args[2] !== expectedArtifact)) {
          protocolDenial("command_blocked", { failure_type: "command_chain_path_mismatch", contract_violation: true, name: command.name });
          event("command_failed", commandFailureDetail(command.name, "command_chain_path_mismatch", "argument_validation", args));
          throw new Error("command path does not match the frozen oracle chain");
        }
        sourceDigest = digest(readFileSync(expectedSource));
        if (command.name === "deliver" && (!state.validatedCommand || state.validatedCommand.source_path !== expectedSource || state.validatedCommand.source_sha256 !== sourceDigest)) {
          protocolDenial("command_blocked", { failure_type: "validation_receipt_missing", contract_violation: true, name: command.name });
          event("command_failed", commandFailureDetail(command.name, "validation_receipt_missing", "argument_validation", args));
          throw new Error("deliver requires a successful validation of the same current source");
        }
      }
      let result: { code: number; stdout: string; stderr: string };
      try { result = await runAllowlistedProcess(policy, command.executable, [...(command.fixed_argv ?? []), ...args], signal); }
      catch (error: any) { event("command_failed", commandFailureDetail(command.name, processFailureType(error), "execution", args)); throw error; }
      if (result.code !== 0) {
        event("command_failed", { ...commandFailureDetail(command.name, "exit_nonzero", "execution", args), exit_code: result.code });
        throw new Error(`allowlisted command exited ${result.code}\n${boundedCommandDiagnostics(result.stdout, result.stderr)}`);
      }
      const commandDetail: Record<string, unknown> = { name: command.name, args_sha256: args.map(digest), exit_code: result.code };
      if (policy.command_chain && command.name === "validate" && sourceDigest) {
        const sourcePath = canonicalCandidate(policy.command_chain.source_path); let evidence: ReturnType<typeof validatedArchitectureEvidence>;
        try { evidence = validatedArchitectureEvidence(result.stdout, sourcePath); }
        catch (error) { event("command_failed", commandFailureDetail(command.name, "validator_receipt_invalid", "receipt_validation", args)); throw error; }
        const chainSha = digest(`validate\0${sourcePath}\0${sourceDigest}\0${evidence.validator_receipt_sha256}`);
        state.validatedCommand = { source_path: sourcePath, source_sha256: sourceDigest, chain_sha256: chainSha, ...evidence };
        Object.assign(commandDetail, { source_path_sha256: digest(sourcePath), source_sha256: sourceDigest, receipt_chain_sha256: chainSha, ...evidence });
      } else if (policy.command_chain && command.name === "deliver" && sourceDigest && state.validatedCommand) {
        const artifactPath = canonicalCandidate(policy.command_chain.artifact_path);
        if (!existsSync(artifactPath) || !lstatSync(artifactPath).isFile()) {
          event("command_failed", commandFailureDetail(command.name, "artifact_missing", "execution", args));
          throw new Error("deliver did not create the frozen oracle artifact");
        }
        const artifactDigest = digest(readFileSync(artifactPath)); const chainSha = digest(`${state.validatedCommand.chain_sha256}\0deliver\0${artifactPath}\0${artifactDigest}`);
        Object.assign(commandDetail, { source_path_sha256: digest(state.validatedCommand.source_path), source_sha256: sourceDigest, artifact_path_sha256: digest(artifactPath), artifact_sha256: artifactDigest, validation_receipt_sha256: state.validatedCommand.chain_sha256, receipt_chain_sha256: chainSha });
      }
      event("command", commandDetail);
      return { content: [{ type: "text", text: result.stdout }], details: { exit_code: result.code } };
    },
  });

  pi.on("tool_call", async (call: any) => {
    if (!["read", "write", "edit"].includes(call.toolName)) return;
    const path = call.input?.path;
    if (typeof path !== "string") {
      const record = call.toolName === "read" ? denial : unsafe;
      record("file_tool_blocked", { tool: call.toolName, reason: "file_tool_path_required" });
      return { block: true, reason: "file tool path is required" };
    }
    const candidate = isAbsolute(path) ? path : resolve(policy.cwd, path);
    try {
      const activeRoots = roots();
      const mode: RootMode = call.toolName === "read" ? "read" : "write";
      const canonical = canonicalPathInRoots(candidate, mode === "read" ? activeRoots.read : activeRoots.write, mode);
      if (mode === "write" && (policy.post_load?.write_paths?.length ?? 0) > 0 && !(policy.post_load?.write_paths ?? []).some((path) => canonicalCandidate(path) === canonical)) {
        const decision = classifyContainedWriteDenial(call.toolName, candidate, canonical, policy.post_load?.contained_write_roots, policy.immutable_paths, policy.protected_roots);
        if (decision.classification === "policy_denial") {
          denial("file_tool_blocked", { tool: call.toolName, failure_type: "output_path_denied", contained: true, reason: "write target is not explicitly allowlisted" });
          return { block: true, reason: "Harness gate: output path is not explicitly allowlisted" };
        }
        throw new Error("write target is not explicitly allowlisted");
      }
      if (call.toolName === "read" && state.returnedSkillPaths.includes(canonical)) {
        state.stage = "task_loaded";
        event("target_skill_loaded", { path_sha256: digest(canonical) });
      } else {
        event("file_tool", { tool: call.toolName, path_sha256: digest(canonical) });
      }
    } catch (error) {
      if (call.toolName === "write" && isRouteOrderViolation(policy.arm, state.stage) && isSafePreRouteWriteDenial(candidate, policy.post_load?.write_roots, policy.post_load?.write_paths, policy.post_load?.contained_write_roots, policy.immutable_paths, policy.protected_roots)) {
        protocolDenial("file_tool_blocked", { tool: call.toolName, failure_type: "route_order", contract_violation: true, reason: "target_skill_not_loaded" });
        return { block: true, reason: "Harness gate: target Skill must be loaded before writing task outputs" };
      }
      if (call.toolName === "read" && isRouteOrderViolation(policy.arm, state.stage) && isExactInjectedBootstrapPath(candidate, policy.bootstrap_path)) {
        protocolDenial("file_tool_blocked", { tool: call.toolName, failure_type: "route_order", contract_violation: true, reason: "trusted_bootstrap_must_not_be_read_before_find" });
        return { block: true, reason: "Harness gate: Find must precede reading the injected Bootstrap file" };
      }
      const futureReadRoots = [...(policy.post_load?.read_roots ?? []), ...state.returnedSkillPaths];
      const classification = deniedFileAttemptClassification(call.toolName, candidate, policy.run_root, futureReadRoots, policy.protected_roots ?? []);
      if (classification === "policy_denial" && isRouteOrderViolation(policy.arm, state.stage)) {
        protocolDenial("file_tool_blocked", { tool: call.toolName, failure_type: "route_order", contract_violation: true, reason: String(error) });
      } else {
        const record = classification === "policy_denial" ? denial : unsafe;
        record("file_tool_blocked", { tool: call.toolName, reason: String(error) });
      }
      return { block: true, reason: `Harness gate: ${String(error)}` };
    }
  });

  event("gate_ready", {
    arm: policy.arm,
    trusted_tcb: ["pi_runtime", "frozen_gate", "frozen_skillroster", "manifest_allowlisted_executables"],
  });
}
