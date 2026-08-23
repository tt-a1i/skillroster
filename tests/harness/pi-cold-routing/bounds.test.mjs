import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, symlinkSync, truncateSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { boundedAppendFile, boundedReadFile, boundedWriteFile, copyBoundedFile, copyBoundedTree, createReadBudget, GATE_LEDGER_MAX_BYTES, walkBoundedTree } from "./bounds.mjs";

test("bounded tree walker rejects entry, depth, single-file, and total-byte excess before copy", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-bounds-"));
  mkdirSync(join(root, "a", "b"), { recursive: true });
  writeFileSync(join(root, "one.txt"), "1234");
  writeFileSync(join(root, "a", "two.txt"), "5678");
  writeFileSync(join(root, "a", "b", "three.txt"), "90");
  assert.throws(() => walkBoundedTree(root, { limits: { maxEntries: 2 } }), /entries/u);
  assert.throws(() => walkBoundedTree(root, { limits: { maxDepth: 1 } }), /depth/u);
  assert.throws(() => walkBoundedTree(root, { limits: { maxSingleFileBytes: 3 } }), /single file/u);
  assert.throws(() => walkBoundedTree(root, { limits: { maxTotalBytes: 9 } }), /total bytes/u);
  const destination = `${root}-copy-must-not-exist`;
  assert.throws(() => copyBoundedTree(root, destination, { limits: { maxEntries: 2 } }), /entries/u);
  assert.throws(() => walkBoundedTree(destination), /ENOENT/u);
});

test("tree walker stops streaming after global limit plus one and closes the directory", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-stream-limit-"));
  for (const name of ["a", "b", "c", "d", "e"]) writeFileSync(join(root, name), name);
  let reads = 0;
  assert.throws(() => walkBoundedTree(root, { limits: { maxEntries: 2 }, onEntryRead() { reads += 1; } }), /entries/u);
  assert.equal(reads, 3);
  assert.doesNotThrow(() => renameSync(root, `${root}-renamed`));
});

test("bounded tree walker rejects symlinks without following loops", (context) => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-symlink-bounds-"));
  mkdirSync(join(root, "directory"));
  const link = join(root, "directory", "loop");
  try { symlinkSync(root, link, process.platform === "win32" ? "junction" : "dir"); }
  catch (error) { context.skip(`symlink unavailable: ${error.code ?? error}`); return; }
  assert.throws(() => walkBoundedTree(root), /symlink/u);
});

test("bounded full-file reads enforce per-file and cumulative budgets", () => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-read-bounds-"));
  const first = join(root, "first.txt"); const second = join(root, "second.txt");
  writeFileSync(first, "12345"); writeFileSync(second, "67890");
  assert.throws(() => boundedReadFile(first, { maxSingleFileBytes: 4 }), /single file/u);
  const budget = createReadBudget(9);
  assert.equal(boundedReadFile(first, { encoding: "utf8", budget }), "12345");
  assert.throws(() => boundedReadFile(second, { budget }), /total bytes/u);
  assert.equal(budget.usedBytes, 5);
  const sparse = join(root, "oversized.bin"); writeFileSync(sparse, ""); truncateSync(sparse, 65 * 1024 * 1024);
  assert.throws(() => boundedReadFile(sparse), /single file/u);
  const writeBudget = createReadBudget(5);
  boundedWriteFile(join(root, "written.txt"), "12345", { budget: writeBudget });
  assert.throws(() => boundedWriteFile(join(root, "too-much.txt"), "x", { budget: writeBudget }), /total bytes/u);
});

test("tree copy canonicalizes a symlinked destination parent before mutation", (context) => {
  const parent = mkdtempSync(join(tmpdir(), "skillroster-copy-alias-")); const source = join(parent, "source"); const alias = join(parent, "alias");
  mkdirSync(source); const input = join(source, "input.txt"); writeFileSync(input, "immutable");
  try { symlinkSync(source, alias, process.platform === "win32" ? "junction" : "dir"); }
  catch (error) { context.skip(`symlink unavailable: ${error.code ?? error}`); return; }
  assert.throws(() => copyBoundedTree(source, join(alias, "nested-copy")), /destination resolves inside source/u);
  assert.equal(readFileSync(input, "utf8"), "immutable");
  assert.equal(existsSync(join(source, "nested-copy")), false);
});

test("tree copy rejects an existing destination containing an outside symlink", (context) => {
  const parent = mkdtempSync(join(tmpdir(), "skillroster-existing-copy-")); const source = join(parent, "source"); const destination = join(parent, "destination"); const outside = join(parent, "outside");
  mkdirSync(source); mkdirSync(destination); mkdirSync(outside); writeFileSync(join(source, "input.txt"), "source"); writeFileSync(join(outside, "sentinel.txt"), "outside");
  try { symlinkSync(outside, join(destination, "escape"), process.platform === "win32" ? "junction" : "dir"); }
  catch (error) { context.skip(`symlink unavailable: ${error.code ?? error}`); return; }
  assert.throws(() => copyBoundedTree(source, destination), /destination already exists/u);
  assert.equal(readFileSync(join(source, "input.txt"), "utf8"), "source");
  assert.equal(readFileSync(join(outside, "sentinel.txt"), "utf8"), "outside");
});

test("recorded identities, no-follow opens, and budget reservation fail closed", (context) => {
  const root = mkdtempSync(join(tmpdir(), "skillroster-identity-")); const source = join(root, "source.txt"); const replacement = join(root, "replacement.txt");
  writeFileSync(source, "first"); const recorded = walkBoundedTree(root).files.find((file) => file.relativePath === "source.txt").identity;
  writeFileSync(replacement, "other"); renameSync(replacement, source);
  const budget = createReadBudget(64);
  assert.throws(() => boundedReadFile(source, { expectedIdentity: recorded, budget }), /identity drifted/u);
  assert.equal(budget.usedBytes, 0);
  assert.throws(() => copyBoundedFile(source, join(root, "copy.txt"), { expectedIdentity: recorded, budget }), /identity drifted/u);
  assert.equal(existsSync(join(root, "copy.txt")), false);
  assert.equal(budget.usedBytes, 0);

  const appendIdentity = walkBoundedTree(root).files.find((file) => file.relativePath === "source.txt").identity;
  writeFileSync(replacement, "third"); renameSync(replacement, source);
  assert.throws(() => boundedAppendFile(source, "x", { expectedIdentity: appendIdentity }), /identity drifted/u);
  assert.throws(() => boundedWriteFile(source, "x", { expectedIdentity: appendIdentity }), /identity drifted/u);
  assert.equal(readFileSync(source, "utf8"), "third");

  const alias = join(root, "alias.txt");
  try { symlinkSync(source, alias, "file"); }
  catch (error) { context.skip(`symlink unavailable: ${error.code ?? error}`); return; }
  assert.throws(() => boundedReadFile(alias), /symbolic link/u);
  assert.throws(() => boundedAppendFile(alias, "x"), /symbolic link/u);
  assert.throws(() => boundedWriteFile(alias, "x"), /symbolic link/u);
});

test("gate event ledger uses a dedicated eight MiB hard cap", () => {
  assert.equal(GATE_LEDGER_MAX_BYTES, 8 * 1024 * 1024);
  const root = mkdtempSync(join(tmpdir(), "skillroster-ledger-cap-")); const ledger = join(root, "events.jsonl");
  boundedAppendFile(ledger, Buffer.alloc(GATE_LEDGER_MAX_BYTES), { maxBytes: GATE_LEDGER_MAX_BYTES });
  assert.throws(() => boundedAppendFile(ledger, "x", { maxBytes: GATE_LEDGER_MAX_BYTES }), /total bytes/u);
});
