import { createHash } from "node:crypto";
import {
  constants,
  createReadStream,
  closeSync,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readlinkSync,
  readdirSync,
  realpathSync,
  writeSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, parse, relative, resolve, sep } from "node:path";

export class LedgerError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "LedgerError";
    this.code = code;
  }
}

const fail = (code, message) => { throw new LedgerError(code, message); };

const canonical = (path) => {
  const absolute = resolve(path);
  try { return realpathSync.native(absolute); } catch { return absolute; }
};

const isWithin = (child, parent) => {
  const relation = relative(parent, child);
  return relation === "" || (relation !== ".." && !relation.startsWith(`..${sep}`) && !isAbsolute(relation));
};

const requireAbsolute = (value, label) => {
  if (typeof value !== "string" || value.trim() === "") fail("empty_scope", `${label} must be a non-empty path`);
  if (!isAbsolute(value)) fail("relative_scope", `${label} must be absolute`);
  if (value.split(/[\\/]+/u).some((segment) => segment === "." || segment === "..")) fail("path_traversal", `${label} contains traversal`);
  const absolute = resolve(value);
  if (parse(absolute).root === absolute) fail("unsafe_scope", `${label} must not be a filesystem root`);
  return absolute;
};

const uniquePaths = (paths, label) => {
  const inputs = paths.map((path) => requireAbsolute(path, label));
  const normalized = inputs.map((path) => canonical(path));
  for (let index = 0; index < normalized.length; index += 1) {
    for (let other = index + 1; other < normalized.length; other += 1) {
      if (isWithin(normalized[index], normalized[other]) || isWithin(normalized[other], normalized[index])) {
        fail("conflicting_scope", `${label} entries overlap`);
      }
    }
  }
  return normalized;
};

const overlapsForbidden = (scope, forbidden) => forbidden.some((entry) => isWithin(scope, entry) || isWithin(entry, scope));

const modeKind = (mode) => {
  if (mode.isFile()) return "file";
  if (mode.isDirectory()) return "directory";
  if (mode.isSymbolicLink()) return "symlink";
  return "special";
};

const sameIdentity = (opened, current) => opened.ino === 0 || current.ino === 0 || (opened.ino === current.ino && opened.dev === current.dev);
const openNoFollow = (path, directory = false) => {
  const flags = constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (directory ? (constants.O_DIRECTORY ?? 0) : 0);
  const descriptor = openSync(path, flags);
  const opened = fstatSync(descriptor);
  const current = lstatSync(path);
  if (!sameIdentity(opened, current) || (directory ? !opened.isDirectory() || current.isSymbolicLink() : !opened.isFile() || current.isSymbolicLink())) {
    closeSync(descriptor); fail("file_drift", "path changed while it was being opened");
  }
  return { descriptor, opened };
};

const assertDirectoryStable = (path, opened) => {
  let current;
  try { current = lstatSync(path); } catch { fail("directory_drift", "directory changed while it was being read"); }
  if (current.isSymbolicLink() || !current.isDirectory() || !sameIdentity(opened, current)) fail("directory_drift", "directory changed while it was being read");
};

export const parsePathList = (text, label = "path list") => {
  if (typeof text !== "string") fail("invalid_list", `${label} must be text`);
  return text.split(/\r?\n/u).map((line) => line.trim()).filter((line) => line !== "");
};

export const validateScope = ({ approvedRoots, configPaths = [], evidenceDir, repositoryDir, stateDir, homeDir = homedir() }) => {
  if (!Array.isArray(approvedRoots) || approvedRoots.length === 0) fail("empty_scope", "at least one approved root is required");
  if (!Array.isArray(configPaths)) fail("invalid_scope", "config paths must be an array");
  const rootInputs = approvedRoots.map((path) => requireAbsolute(path, "approved root"));
  const configInputs = configPaths.map((path) => requireAbsolute(path, "config path"));
  for (const [inputs, label] of [[rootInputs, "approved root"], [configInputs, "config path"]]) {
    for (const input of inputs) {
      let mode;
      try { mode = lstatSync(input); } catch { fail("unreadable_scope", `${label} cannot be inspected`); }
      if (mode.isSymbolicLink()) fail("unsafe_scope", `${label} must not be a symlink`);
    }
  }
  const roots = uniquePaths(rootInputs, "approved root");
  const configs = uniquePaths(configInputs, "config path");
  if (!evidenceDir || !repositoryDir || !stateDir || !homeDir) fail("invalid_scope", "evidence, repository, state, and home directories are required");
  const home = canonical(requireAbsolute(homeDir, "home directory"));
  const evidenceInput = requireAbsolute(evidenceDir, "evidence directory");
  const repositoryInput = requireAbsolute(repositoryDir, "repository directory");
  const stateInput = requireAbsolute(stateDir, "state directory");
  for (const [input, label] of [[evidenceInput, "evidence directory"], [repositoryInput, "repository directory"], [stateInput, "state directory"]]) {
    try { if (lstatSync(input).isSymbolicLink()) fail("unsafe_scope", `${label} must not be a symlink`); } catch (error) { if (error instanceof LedgerError) throw error; }
  }
  const evidence = canonical(evidenceInput);
  const repository = canonical(repositoryInput);
  const state = canonical(stateInput);
  if (overlapsForbidden(evidence, [home, repository, state]) || overlapsForbidden(repository, [state])) fail("unsafe_scope", "evidence, repository, and state directories overlap");
  const forbidden = [evidence, repository, state]
    .filter((entry) => entry !== undefined && entry !== null)
    .map((entry) => canonical(entry));
  for (const root of roots) {
    if (parse(root).root === root) fail("unsafe_scope", "filesystem root is not an approved scope");
    if (root === home || isWithin(home, root) || overlapsForbidden(root, forbidden)) fail("unsafe_scope", "approved root overlaps a forbidden scope");
    let mode;
    try { mode = lstatSync(root); } catch { fail("unreadable_scope", "approved root cannot be inspected"); }
    if (!mode.isDirectory() || mode.isSymbolicLink()) fail("unsafe_scope", "approved root must be a real directory");
  }
  for (const config of configs) {
    if (config === home || overlapsForbidden(config, forbidden)) fail("unsafe_scope", "config path overlaps a forbidden scope");
    let mode;
    try { mode = lstatSync(config); } catch { fail("unreadable_scope", "config path cannot be inspected"); }
    if (!mode.isFile() || mode.isSymbolicLink()) fail("unsafe_config", "config path must be a regular file");
  }
  return { roots, configs, forbidden };
};

const hashFile = async (path) => {
  const hash = createHash("sha256");
  let descriptor;
  let size;
  try {
    // O_NOFOLLOW (where provided) and a descriptor-bound read close the
    // lstat-to-read gap: a replacement symlink is never opened and hashed.
    const opened = openNoFollow(path);
    descriptor = opened.descriptor;
    size = opened.opened.size;
    for await (const chunk of createReadStream(null, { fd: descriptor, autoClose: false })) hash.update(chunk);
  } catch (error) {
    if (error instanceof LedgerError) throw error;
    fail("unreadable_file", "regular file cannot be read");
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  return { size, sha256: hash.digest("hex") };
};

const relativeIdentity = (root, entry, rootIndex) => {
  const child = relative(root, entry).split(sep).join("/");
  return `root-${rootIndex}/${child}`;
};

const targetScope = (entry, spelling, roots) => {
  const target = resolve(dirname(entry), spelling);
  const index = roots.findIndex((root) => isWithin(target, root));
  if (index < 0) return "external_target_not_hashed";
  const root = roots[index];
  const components = relative(root, target).split(sep).filter(Boolean);
  let current = root;
  for (const component of components) {
    current = resolve(current, component);
    let mode;
    try { mode = lstatSync(current); } catch { return "external_target_not_hashed"; }
    if (mode.isSymbolicLink() || (!mode.isDirectory() && !mode.isFile())) return "external_target_not_hashed";
  }
  return `approved_root_${index}`;
};

const walkRoot = async (root, rootIndex, roots, records) => {
  records.push({ id: `root-${rootIndex}`, kind: "directory" });
  const visit = async (directory) => {
    let descriptor;
    let opened;
    let entries;
    try {
      opened = openNoFollow(directory, true); descriptor = opened.descriptor;
      try { entries = readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name)); }
      catch { fail("unreadable_directory", "directory cannot be read"); }
      assertDirectoryStable(directory, opened.opened);
      for (const entry of entries) {
        assertDirectoryStable(directory, opened.opened);
        const absolute = resolve(directory, entry.name);
        const id = relativeIdentity(root, absolute, rootIndex);
        let mode;
        try { mode = lstatSync(absolute); } catch { fail("unreadable_entry", "entry cannot be inspected"); }
        const kind = modeKind(mode);
        if (kind === "file") {
          const hashed = await hashFile(absolute);
          records.push({ id, kind, size: hashed.size, sha256: hashed.sha256 });
        } else if (kind === "directory") {
          records.push({ id, kind });
          await visit(absolute);
        } else if (kind === "symlink") {
          let spelling;
          try { spelling = readlinkSync(absolute); } catch { fail("unreadable_symlink", "symlink target spelling cannot be read"); }
          records.push({ id, kind, target_spelling: spelling, target_scope: targetScope(absolute, spelling, roots) });
        } else {
          fail("special_file", "special files are outside the safe ledger format");
        }
      }
      assertDirectoryStable(directory, opened.opened);
    } finally {
      if (descriptor !== undefined) closeSync(descriptor);
    }
  };
  await visit(root);
};

const aggregate = (records) => records.reduce((result, record) => {
  result.record_count += 1;
  result[`${record.kind}_count`] += 1;
  if (record.kind === "file") { result.regular_file_bytes += record.size; }
  if (record.target_scope === "external_target_not_hashed") result.external_target_count += 1;
  return result;
}, { record_count: 0, file_count: 0, directory_count: 0, symlink_count: 0, special_count: 0, regular_file_bytes: 0, external_target_count: 0 });

const exactKeys = (value, keys, label) => {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).sort().join("\0") !== [...keys].sort().join("\0")) fail("invalid_ledger", `${label} has an invalid shape`);
};
const integer = (value, label) => { if (!Number.isSafeInteger(value) || value < 0) fail("invalid_ledger", `${label} must be a non-negative integer`); };
const digest = (value, label) => { if (typeof value !== "string" || !/^[0-9a-f]{64}$/u.test(value)) fail("invalid_ledger", `${label} must be a SHA-256 digest`); };

export const validateLedger = (ledger, label = "ledger") => {
  exactKeys(ledger, ["schema_version", "format", "scope", "aggregate", "records"], label);
  if (ledger.schema_version !== 1 || ledger.format !== "skillroster-byte-ledger") fail("invalid_ledger", `${label} has an unsupported format`);
  exactKeys(ledger.scope, ["approved_root_count", "config_path_count"], `${label} scope`);
  integer(ledger.scope.approved_root_count, `${label} root count`); integer(ledger.scope.config_path_count, `${label} config count`);
  if (!Array.isArray(ledger.records) || ledger.records.length === 0) fail("invalid_ledger", `${label} records are required`);
  const ids = new Set(); let rootCount = 0; let configCount = 0;
  for (const record of ledger.records) {
    if (!record || typeof record.id !== "string" || !(/^(?:root-\d+(?:\/[^/]+)+|root-\d+|config-\d+)$/u.test(record.id)) || record.id.startsWith("/") || record.id.split("/").some((segment) => segment === ".." || segment === ".") || ids.has(record.id)) fail("invalid_ledger", `${label} contains an invalid or duplicate identity`);
    ids.add(record.id);
    if (/^root-\d+$/u.test(record.id)) rootCount += 1;
    if (/^config-\d+$/u.test(record.id)) configCount += 1;
    if (record.kind === "file") {
      exactKeys(record, ["id", "kind", "size", "sha256"], `${label} file`); integer(record.size, `${label} file size`); digest(record.sha256, `${label} file digest`);
    } else if (record.kind === "directory") {
      exactKeys(record, ["id", "kind"], `${label} directory`);
    } else if (record.kind === "symlink") {
      exactKeys(record, ["id", "kind", "target_scope", "target_spelling"], `${label} symlink`);
      if (typeof record.target_spelling !== "string" || record.target_spelling.length === 0 || record.target_spelling.includes("\0")) fail("invalid_ledger", `${label} symlink spelling is invalid`);
      if (record.target_scope !== "external_target_not_hashed" && (!/^approved_root_\d+$/u.test(record.target_scope) || Number(record.target_scope.slice("approved_root_".length)) >= ledger.scope.approved_root_count)) fail("invalid_ledger", `${label} symlink scope is invalid`);
    } else fail("invalid_ledger", `${label} contains an unknown record kind`);
  }
  if (rootCount !== ledger.scope.approved_root_count || configCount !== ledger.scope.config_path_count) fail("invalid_ledger", `${label} scope counts do not match records`);
  exactKeys(ledger.aggregate, ["record_count", "file_count", "directory_count", "symlink_count", "special_count", "regular_file_bytes", "external_target_count"], `${label} aggregate`);
  const expected = aggregate(ledger.records);
  if (JSON.stringify(expected) !== JSON.stringify(ledger.aggregate)) fail("invalid_ledger", `${label} aggregate does not match records`);
  return ledger;
};

export const collectLedger = async (scope) => {
  const validated = validateScope(scope);
  const records = [];
  for (const [index, root] of validated.roots.entries()) await walkRoot(root, index, validated.roots, records);
  for (const [index, config] of validated.configs.entries()) {
    let mode;
    try { mode = lstatSync(config); } catch { fail("unreadable_file", "config path cannot be read"); }
    if (!mode.isFile() || mode.isSymbolicLink()) fail("unsafe_config", "config path changed to a non-regular file");
    const hashed = await hashFile(config);
    records.push({ id: `config-${index}`, kind: "file", size: hashed.size, sha256: hashed.sha256 });
  }
  records.sort((left, right) => left.id.localeCompare(right.id));
  return {
    schema_version: 1,
    format: "skillroster-byte-ledger",
    scope: { approved_root_count: validated.roots.length, config_path_count: validated.configs.length },
    aggregate: aggregate(records),
    records,
  };
};

const stableJson = (value) => JSON.stringify(value);
export const ledgerDigest = (ledger) => { validateLedger(ledger); return createHash("sha256").update(stableJson(ledger)).digest("hex"); };

export const compareLedgers = (before, after) => {
  validateLedger(before, "before ledger"); validateLedger(after, "after ledger");
  if (JSON.stringify(before.scope) !== JSON.stringify(after.scope)) fail("scope_mismatch", "ledger scopes do not match");
  const beforeById = new Map(before.records.map((record) => [record.id, stableJson(record)]));
  const afterById = new Map(after.records.map((record) => [record.id, stableJson(record)]));
  let added = 0; let removed = 0; let changed = 0;
  for (const id of beforeById.keys()) {
    if (!afterById.has(id)) removed += 1;
    else if (beforeById.get(id) !== afterById.get(id)) changed += 1;
  }
  for (const id of afterById.keys()) if (!beforeById.has(id)) added += 1;
  return { equal: added === 0 && removed === 0 && changed === 0, added, removed, changed, before_digest: ledgerDigest(before), after_digest: ledgerDigest(after) };
};

export const redactedComparison = (before, after) => {
  const comparison = compareLedgers(before, after);
  return {
    schema_version: 1,
    format: "skillroster-byte-ledger-redacted-comparison",
    before: { digest: comparison.before_digest, aggregate: before.aggregate },
    after: { digest: comparison.after_digest, aggregate: after.aggregate },
    comparison: { equal: comparison.equal, added_records: comparison.added, removed_records: comparison.removed, changed_records: comparison.changed },
    external_target_bytes: "out_of_scope_and_not_hashed",
    privacy: { absolute_paths: false, file_names: false, file_contents: false },
  };
};

export const writeRawLedger = (ledger, outputPath, evidenceDir) => {
  validateLedger(ledger);
  const output = requireAbsolute(outputPath, "ledger output");
  const lexicalEvidence = requireAbsolute(evidenceDir, "evidence directory");
  if (!isWithin(output, lexicalEvidence) || output === lexicalEvidence) fail("unsafe_output", "raw ledger must be written inside evidence directory");
  try { mkdirSync(lexicalEvidence, { recursive: true }); } catch { fail("write_failed", "evidence directory could not be created"); }
  const evidence = canonical(lexicalEvidence);
  const safeOutput = resolve(evidence, relative(lexicalEvidence, output));
  if (dirname(safeOutput) !== evidence || !isWithin(canonical(dirname(safeOutput)), evidence)) fail("unsafe_output", "raw ledger must be directly inside evidence directory");
  let descriptor;
  try { descriptor = openSync(safeOutput, "wx", 0o600); writeSync(descriptor, `${JSON.stringify(ledger, null, 2)}\n`); }
  catch { fail("write_failed", "raw ledger could not be written exclusively"); }
  finally { if (descriptor !== undefined) closeSync(descriptor); }
  return safeOutput;
};

const readListArg = (path, label) => {
  const absolute = requireAbsolute(path, `${label} file`);
  let opened; let text;
  try {
    opened = openNoFollow(absolute);
    text = readFileSync(opened.descriptor, "utf8");
    const current = lstatSync(absolute);
    if (!sameIdentity(opened.opened, current) || current.isSymbolicLink()) fail("input_drift", `${label} file changed while it was read`);
  } catch (error) { if (error instanceof LedgerError) throw error; fail("unreadable_input", `${label} file cannot be read`); }
  finally { if (opened !== undefined) closeSync(opened.descriptor); }
  return parsePathList(text, label);
};
const argValue = (args, flag) => {
  const index = args.indexOf(flag);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) fail("invalid_args", `${flag} requires a value`);
  return value;
};

const cli = async (args) => {
  const mode = args[0];
  if (mode === "capture") {
    const rootsFile = argValue(args, "--roots-file"); const configFile = argValue(args, "--config-file"); const evidenceDir = argValue(args, "--evidence-dir"); const output = argValue(args, "--output");
    const repositoryDir = argValue(args, "--repository-dir"); const stateDir = argValue(args, "--state-dir"); const homeDir = argValue(args, "--home-dir");
    if (!rootsFile || !evidenceDir || !output || !repositoryDir || !stateDir || !homeDir) fail("invalid_args", "capture requires explicit roots, evidence, output, repository, state, and home paths");
    const ledger = await collectLedger({ approvedRoots: readListArg(rootsFile, "approved root list"), configPaths: configFile ? readListArg(configFile, "config path list") : [], evidenceDir, repositoryDir, stateDir, homeDir });
    writeRawLedger(ledger, output, evidenceDir);
    process.stdout.write(`${JSON.stringify({ format: ledger.format, aggregate: ledger.aggregate, ledger_sha256: ledgerDigest(ledger) })}\n`);
  } else if (mode === "compare") {
    const before = JSON.parse(readFileSync(requireAbsolute(argValue(args, "--before"), "before ledger"), "utf8"));
    const after = JSON.parse(readFileSync(requireAbsolute(argValue(args, "--after"), "after ledger"), "utf8"));
    process.stdout.write(`${JSON.stringify(redactedComparison(before, after))}\n`);
  } else fail("invalid_args", "command must be capture or compare");
};

export { cli as runCli };

if (process.argv[1] === new URL(import.meta.url).pathname) {
  cli(process.argv.slice(2)).catch((error) => { process.stderr.write(`${error.code ?? "error"}: ${error.message}\n`); process.exitCode = 1; });
}
