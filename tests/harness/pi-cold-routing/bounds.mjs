import { closeSync, constants, existsSync, fchmodSync, fstatSync, ftruncateSync, lstatSync, mkdirSync, openSync, opendirSync, readSync, realpathSync, writeSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

export const HARNESS_IO_LIMITS = Object.freeze({
  maxEntries: 10_000,
  maxDepth: 32,
  maxSingleFileBytes: 64 * 1024 * 1024,
  maxTotalBytes: 256 * 1024 * 1024,
});
export const GATE_LEDGER_MAX_BYTES = 8 * 1024 * 1024;

function fail(label, reason) {
  throw new Error(`${label} exceeds bounded I/O policy: ${reason}`);
}

function within(candidate, root) {
  const suffix = relative(root, candidate);
  return suffix === "" || (!suffix.startsWith(`..${sep}`) && suffix !== ".." && !isAbsolute(suffix));
}

export function createReadBudget(maxTotalBytes = HARNESS_IO_LIMITS.maxTotalBytes) {
  return { usedBytes: 0, maxTotalBytes };
}

function statIdentity(stat) {
  return { dev: stat.dev, ino: stat.ino, size: stat.size, mode: stat.mode, mtimeMs: stat.mtimeMs, ctimeMs: stat.ctimeMs, birthtimeMs: stat.birthtimeMs, nlink: stat.nlink };
}

function sameObject(left, right) {
  const sameType = (left.mode & constants.S_IFMT) === (right.mode & constants.S_IFMT);
  const strongFileId = left.dev !== 0 || left.ino !== 0 || right.dev !== 0 || right.ino !== 0;
  if (strongFileId) return sameType && left.dev === right.dev && left.ino === right.ino;
  const stableBirthtime = Number.isFinite(left.birthtimeMs) && left.birthtimeMs > 0 && Number.isFinite(right.birthtimeMs) && right.birthtimeMs > 0;
  return sameType && stableBirthtime && left.birthtimeMs === right.birthtimeMs;
}

function sameIdentity(left, right) {
  return sameObject(left, right) && left.size === right.size && left.mtimeMs === right.mtimeMs && left.ctimeMs === right.ctimeMs && left.nlink === right.nlink;
}

function assertIdentity(actual, expected, label, complete = true) {
  if (expected && !(complete ? sameIdentity(actual, expected) : sameObject(actual, expected))) fail(label, "filesystem identity drifted");
}

function reserveBudget(budget, bytes, label) {
  if (!budget) return () => {};
  const next = budget.usedBytes + bytes;
  if (next > budget.maxTotalBytes) fail(label, `total bytes ${next} > ${budget.maxTotalBytes}`);
  budget.usedBytes = next;
  let active = true;
  return () => { if (active) { budget.usedBytes -= bytes; active = false; } };
}

function noFollowFlag() {
  return process.platform === "win32" ? 0 : (constants.O_NOFOLLOW ?? 0);
}

function writeAll(descriptor, bytes) {
  let offset = 0;
  while (offset < bytes.length) offset += writeSync(descriptor, bytes, offset, bytes.length - offset, null);
}

export function boundedReadFile(path, options = {}) {
  const label = options.label ?? "file";
  const maxSingleFileBytes = options.maxSingleFileBytes ?? HARNESS_IO_LIMITS.maxSingleFileBytes;
  const lexicalStat = lstatSync(path); const lexicalIdentity = statIdentity(lexicalStat);
  if (!lexicalStat.isFile()) fail(label, lexicalStat.isSymbolicLink() ? "symbolic link" : "special file");
  assertIdentity(lexicalIdentity, options.expectedIdentity, label);
  if (lexicalStat.size > maxSingleFileBytes) fail(label, `single file ${lexicalStat.size} > ${maxSingleFileBytes} bytes`);
  const descriptor = openSync(path, constants.O_RDONLY | noFollowFlag());
  let rollback = () => {};
  try {
    const stat = fstatSync(descriptor); const openedIdentity = statIdentity(stat);
    if (!stat.isFile()) fail(label, "opened object is not a regular file");
    assertIdentity(openedIdentity, lexicalIdentity, label);
    if (stat.size > maxSingleFileBytes) fail(label, `single file ${stat.size} > ${maxSingleFileBytes} bytes`);
    rollback = reserveBudget(options.budget, stat.size, label);
    const buffer = Buffer.allocUnsafe(stat.size);
    let offset = 0;
    while (offset < buffer.length) {
      const count = readSync(descriptor, buffer, offset, buffer.length - offset, offset);
      if (count === 0) break;
      offset += count;
    }
    const probe = Buffer.allocUnsafe(1);
    if (readSync(descriptor, probe, 0, 1, offset) !== 0) fail(label, "file grew during bounded read");
    const content = offset === buffer.length ? buffer : buffer.subarray(0, offset);
    assertIdentity(statIdentity(fstatSync(descriptor)), openedIdentity, label);
    assertIdentity(statIdentity(lstatSync(path)), openedIdentity, label);
    rollback = () => {};
    return options.encoding ? content.toString(options.encoding) : content;
  } catch (error) {
    rollback();
    throw error;
  } finally {
    closeSync(descriptor);
  }
}

export function boundedFileSha256(path, options = {}) {
  return createHash("sha256").update(boundedReadFile(path, options)).digest("hex");
}

export function copyBoundedFile(source, destination, options = {}) {
  const sourceStat = lstatSync(source); const content = boundedReadFile(source, { ...options, expectedIdentity: options.expectedIdentity ?? statIdentity(sourceStat) });
  boundedWriteFile(destination, content, { label: options.label, flag: "wx", mode: sourceStat.mode & 0o777 });
}

export function boundedAppendFile(path, value, options = {}) {
  const label = options.label ?? "append-only file";
  const bytes = Buffer.from(value, options.encoding ?? "utf8");
  const existing = existsSync(path) ? lstatSync(path) : null; const existingIdentity = existing ? statIdentity(existing) : null;
  if (existing && !existing.isFile()) fail(label, existing.isSymbolicLink() ? "symbolic link" : "special file");
  const maxBytes = options.maxBytes ?? HARNESS_IO_LIMITS.maxSingleFileBytes;
  const flags = constants.O_WRONLY | constants.O_APPEND | noFollowFlag() | (existing ? 0 : constants.O_CREAT | constants.O_EXCL);
  const descriptor = openSync(path, flags, options.mode ?? 0o600);
  try {
    const opened = fstatSync(descriptor); const openedIdentity = statIdentity(opened);
    if (!opened.isFile()) fail(label, "opened object is not a regular file");
    assertIdentity(openedIdentity, existingIdentity, label);
    assertIdentity(openedIdentity, options.expectedIdentity, label);
    const next = opened.size + bytes.length;
    if (next > maxBytes) fail(label, `total bytes ${next} > ${maxBytes}`);
    writeAll(descriptor, bytes);
    const after = statIdentity(fstatSync(descriptor));
    if (!sameObject(after, openedIdentity) || after.size !== next) fail(label, "append identity drifted");
    assertIdentity(statIdentity(lstatSync(path)), after, label);
  } finally { closeSync(descriptor); }
}

export function boundedWriteFile(path, value, options = {}) {
  const label = options.label ?? "output file";
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(value, options.encoding ?? "utf8");
  const maxBytes = options.maxBytes ?? HARNESS_IO_LIMITS.maxSingleFileBytes;
  if (bytes.length > maxBytes) fail(label, `single file ${bytes.length} > ${maxBytes} bytes`);
  const existing = existsSync(path) ? lstatSync(path) : null; const existingIdentity = existing ? statIdentity(existing) : null;
  if (existing && !existing.isFile()) fail(label, existing.isSymbolicLink() ? "symbolic link" : "special file");
  if (options.flag === "wx" && existing) fail(label, "destination already exists");
  assertIdentity(existingIdentity, options.expectedIdentity, label);
  const rollback = reserveBudget(options.budget, bytes.length, label);
  const flags = constants.O_WRONLY | noFollowFlag() | (existing ? 0 : constants.O_CREAT | constants.O_EXCL);
  let descriptor;
  try {
    descriptor = openSync(path, flags, options.mode ?? 0o666);
    const opened = fstatSync(descriptor); const openedIdentity = statIdentity(opened);
    if (!opened.isFile()) fail(label, "opened object is not a regular file");
    assertIdentity(openedIdentity, existingIdentity, label);
    if (options.mode !== undefined) fchmodSync(descriptor, options.mode);
    ftruncateSync(descriptor, 0);
    writeAll(descriptor, bytes);
    const after = statIdentity(fstatSync(descriptor));
    if (!sameObject(after, openedIdentity) || after.size !== bytes.length) fail(label, "write identity drifted");
    assertIdentity(statIdentity(lstatSync(path)), after, label);
  } catch (error) {
    rollback();
    throw error;
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

export function walkBoundedTree(root, options = {}) {
  const limits = { ...HARNESS_IO_LIMITS, ...(options.limits ?? {}) };
  const label = options.label ?? "tree";
  const lexicalRoot = resolve(root);
  const rootStat = lstatSync(lexicalRoot);
  if (!rootStat.isDirectory()) fail(label, rootStat.isSymbolicLink() ? "root symbolic link" : "root is not a directory");
  const rootIdentity = statIdentity(rootStat);
  const canonicalRoot = realpathSync(lexicalRoot);
  const stack = [{ path: lexicalRoot, relativePath: "", depth: 0, identity: rootIdentity }];
  const files = [];
  const directories = [];
  const specials = [];
  let entries = 0;
  let totalBytes = 0;
  while (stack.length) {
    const current = stack.pop();
    if (current.depth > limits.maxDepth) fail(label, `depth ${current.depth} > ${limits.maxDepth}`);
    assertIdentity(statIdentity(lstatSync(current.path)), current.identity, label);
    const children = []; const directory = opendirSync(current.path);
    try {
      while (true) {
        const entry = directory.readSync();
        if (!entry) break;
        entries += 1;
        options.onEntryRead?.({ directory: current.path, name: entry.name, entries });
        if (entries > limits.maxEntries) fail(label, `entries ${entries} > ${limits.maxEntries}`);
        children.push(entry);
      }
    } finally {
      directory.closeSync();
    }
    children.sort((a, b) => a.name.localeCompare(b.name));
    assertIdentity(statIdentity(lstatSync(current.path)), current.identity, label);
    for (let index = children.length - 1; index >= 0; index -= 1) {
      const entry = children[index];
      const absolutePath = join(current.path, entry.name);
      const relativePath = current.relativePath ? `${current.relativePath}/${entry.name}` : entry.name;
      const stat = lstatSync(absolutePath);
      if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) {
        const special = { absolutePath, relativePath, kind: stat.isSymbolicLink() ? "symlink" : "special" };
        if (options.allowSpecial === true) { specials.push(special); continue; }
        fail(label, `${special.kind} at ${relativePath}`);
      }
      const canonical = realpathSync(absolutePath);
      if (!within(canonical, canonicalRoot)) fail(label, `member escapes root at ${relativePath}`);
      if (stat.isDirectory()) {
        const depth = current.depth + 1;
        if (depth > limits.maxDepth) fail(label, `depth ${depth} > ${limits.maxDepth}`);
        const identity = statIdentity(stat);
        directories.push({ absolutePath, relativePath, depth, identity });
        stack.push({ path: absolutePath, relativePath, depth, identity });
      } else {
        if (stat.size > limits.maxSingleFileBytes) fail(label, `single file ${relativePath} is ${stat.size} > ${limits.maxSingleFileBytes} bytes`);
        totalBytes += stat.size;
        if (totalBytes > limits.maxTotalBytes) fail(label, `total bytes ${totalBytes} > ${limits.maxTotalBytes}`);
        files.push({ absolutePath, relativePath, size: stat.size, identity: statIdentity(stat) });
      }
    }
  }
  files.sort((a, b) => a.relativePath.localeCompare(b.relativePath));
  directories.sort((a, b) => a.relativePath.localeCompare(b.relativePath));
  specials.sort((a, b) => a.relativePath.localeCompare(b.relativePath));
  assertIdentity(statIdentity(lstatSync(lexicalRoot)), rootIdentity, label);
  return { files, directories, specials, entries, totalBytes, rootIdentity };
}

export function copyBoundedTree(source, destination, options = {}) {
  const canonicalSource = realpathSync(source);
  if (existsSync(destination)) fail(options.label ?? "copied tree", "destination already exists");
  let ancestor = resolve(destination); const remainder = [];
  while (!existsSync(ancestor)) { const parent = dirname(ancestor); if (parent === ancestor) fail(options.label ?? "copied tree", "destination has no existing ancestor"); remainder.unshift(relative(parent, ancestor)); ancestor = parent; }
  const canonicalDestination = resolve(realpathSync(ancestor), ...remainder);
  if (within(canonicalDestination, canonicalSource)) fail(options.label ?? "copied tree", "destination resolves inside source");
  const walked = walkBoundedTree(source, { ...options, allowSpecial: false });
  mkdirSync(dirname(destination), { recursive: true });
  mkdirSync(destination);
  const budget = createReadBudget(options.limits?.maxTotalBytes ?? HARNESS_IO_LIMITS.maxTotalBytes);
  for (const directory of walked.directories) mkdirSync(join(destination, directory.relativePath), { recursive: true });
  for (const file of walked.files) {
    const target = join(destination, file.relativePath);
    mkdirSync(dirname(target), { recursive: true });
    copyBoundedFile(file.absolutePath, target, { label: options.label ?? "copied tree", budget, maxSingleFileBytes: options.limits?.maxSingleFileBytes, expectedIdentity: file.identity });
  }
  return walked;
}
