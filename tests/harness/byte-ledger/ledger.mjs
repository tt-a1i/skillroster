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
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";

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
  return resolve(value);
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
  const home = canonical(requireAbsolute(homeDir, "home directory"));
  const forbidden = [evidenceDir, repositoryDir, stateDir]
    .filter((entry) => entry !== undefined && entry !== null)
    .map((entry) => canonical(requireAbsolute(entry, "forbidden scope")));
  for (const root of roots) {
    if (root === resolve(sep)) fail("unsafe_scope", "filesystem root is not an approved scope");
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
  try {
    // O_NOFOLLOW (where provided) and a descriptor-bound read close the
    // lstat-to-read gap: a replacement symlink is never opened and hashed.
    const noFollow = constants.O_NOFOLLOW ?? 0;
    descriptor = openSync(path, constants.O_RDONLY | noFollow);
    const opened = fstatSync(descriptor);
    const current = lstatSync(path);
    if (!opened.isFile() || current.isSymbolicLink() || (opened.ino !== 0 && current.ino !== 0 && (opened.ino !== current.ino || opened.dev !== current.dev))) fail("file_drift", "regular file changed while it was being opened");
    for await (const chunk of createReadStream(null, { fd: descriptor, autoClose: false })) hash.update(chunk);
  } catch (error) {
    if (error instanceof LedgerError) throw error;
    fail("unreadable_file", "regular file cannot be read");
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  return hash.digest("hex");
};

const relativeIdentity = (root, entry, rootIndex) => {
  const child = relative(root, entry).split(sep).join("/");
  return `root-${rootIndex}/${child}`;
};

const targetScope = (entry, spelling, roots) => {
  const target = resolve(dirname(entry), spelling);
  const index = roots.findIndex((root) => isWithin(target, root));
  return index < 0 ? "external_target_not_hashed" : `approved_root_${index}`;
};

const walkRoot = async (root, rootIndex, roots, records) => {
  records.push({ id: `root-${rootIndex}`, kind: "directory" });
  const visit = async (directory) => {
    let entries;
    try { entries = readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name)); }
    catch { fail("unreadable_directory", "directory cannot be read"); }
    for (const entry of entries) {
      const absolute = resolve(directory, entry.name);
      const id = relativeIdentity(root, absolute, rootIndex);
      let mode;
      try { mode = lstatSync(absolute); } catch { fail("unreadable_entry", "entry cannot be inspected"); }
      const kind = modeKind(mode);
      if (kind === "file") {
        records.push({ id, kind, size: mode.size, sha256: await hashFile(absolute) });
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

export const collectLedger = async (scope) => {
  const validated = validateScope(scope);
  const records = [];
  for (const [index, root] of validated.roots.entries()) await walkRoot(root, index, validated.roots, records);
  for (const [index, config] of validated.configs.entries()) {
    let mode;
    try { mode = lstatSync(config); } catch { fail("unreadable_file", "config path cannot be read"); }
    if (!mode.isFile() || mode.isSymbolicLink()) fail("unsafe_config", "config path changed to a non-regular file");
    records.push({ id: `config-${index}`, kind: "file", size: mode.size, sha256: await hashFile(config) });
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
export const ledgerDigest = (ledger) => createHash("sha256").update(stableJson(ledger)).digest("hex");

export const compareLedgers = (before, after) => {
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
  const output = requireAbsolute(outputPath, "ledger output");
  const lexicalEvidence = requireAbsolute(evidenceDir, "evidence directory");
  if (!isWithin(output, lexicalEvidence) || output === lexicalEvidence) fail("unsafe_output", "raw ledger must be written inside evidence directory");
  try { mkdirSync(lexicalEvidence, { recursive: true }); } catch { fail("write_failed", "evidence directory could not be created"); }
  const evidence = canonical(lexicalEvidence);
  const safeOutput = resolve(evidence, relative(lexicalEvidence, output));
  if (!isWithin(canonical(dirname(safeOutput)), evidence)) fail("unsafe_output", "raw ledger parent escapes evidence directory");
  let descriptor;
  try { descriptor = openSync(safeOutput, "wx", 0o600); writeSync(descriptor, `${JSON.stringify(ledger, null, 2)}\n`); }
  catch { fail("write_failed", "raw ledger could not be written exclusively"); }
  finally { if (descriptor !== undefined) closeSync(descriptor); }
  return safeOutput;
};

const readListArg = (path, label) => {
  const absolute = requireAbsolute(path, `${label} file`);
  let mode; let text;
  try { mode = lstatSync(absolute); if (!mode.isFile() || mode.isSymbolicLink()) fail("unsafe_input", `${label} file must be a regular file`); text = readFileSync(absolute, "utf8"); }
  catch (error) { if (error instanceof LedgerError) throw error; fail("unreadable_input", `${label} file cannot be read`); }
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
    if (!rootsFile || !evidenceDir || !output) fail("invalid_args", "capture requires --roots-file, --evidence-dir, and --output");
    const ledger = await collectLedger({ approvedRoots: readListArg(rootsFile, "approved root list"), configPaths: configFile ? readListArg(configFile, "config path list") : [], evidenceDir, repositoryDir: argValue(args, "--repository-dir"), stateDir: argValue(args, "--state-dir"), homeDir: argValue(args, "--home-dir") ?? homedir() });
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
