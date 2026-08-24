import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { collectLedger, compareLedgers, LedgerError, redactedComparison, validateLedger, validateScope, writeRawLedger } from "./ledger.mjs";

const tempRoot = () => mkdtempSync(join(resolve(tmpdir()), "skillroster-byte-ledger-"));
const scope = (root, extras = {}) => ({ approvedRoots: [root], configPaths: extras.configPaths ?? [], evidenceDir: extras.evidenceDir ?? join(dirname(root), "evidence"), repositoryDir: extras.repositoryDir ?? join(dirname(root), "repo"), stateDir: extras.stateDir ?? join(dirname(root), "state"), homeDir: extras.homeDir ?? join(dirname(root), "home") });

test("same-size content changes are detected without relying on mtime", async () => {
  const root = tempRoot(); const file = join(root, "skill", "SKILL.md"); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, "AAAA");
  const before = await collectLedger(scope(root)); const originalMtime = (await import("node:fs")).statSync(file).mtime;
  writeFileSync(file, "BBBB"); utimesSync(file, originalMtime, originalMtime);
  const after = await collectLedger(scope(root)); const comparison = compareLedgers(before, after);
  assert.equal(comparison.equal, false); assert.equal(comparison.changed, 1); assert.notEqual(before.records.find((record) => record.kind === "file").sha256, after.records.find((record) => record.kind === "file").sha256); rmSync(root, { recursive: true, force: true });
});

test("symlink retargeting changes identity but never hashes its target", { skip: process.platform === "win32" }, async () => {
  const root = tempRoot(); const inside = join(root, "inside.txt"); const outside = join(dirname(root), "outside-secret.txt"); writeFileSync(inside, "inside"); writeFileSync(outside, "secret"); symlinkSync("inside.txt", join(root, "link"));
  const before = await collectLedger(scope(root)); rmSync(join(root, "link")); symlinkSync(outside, join(root, "link")); const after = await collectLedger(scope(root));
  const linkBefore = before.records.find((record) => record.kind === "symlink"); const linkAfter = after.records.find((record) => record.kind === "symlink");
  const outsideDigest = createHash("sha256").update("secret").digest("hex"); assert.equal(linkBefore.target_scope, "approved_root_0"); assert.equal(linkAfter.target_scope, "external_target_not_hashed"); assert.equal(compareLedgers(before, after).changed, 1); assert.equal(after.records.some((record) => record.sha256 === outsideDigest), false); rmSync(outside, { force: true }); rmSync(root, { recursive: true, force: true });
});

test("symlink target classification is conservative for indirect external links", { skip: process.platform === "win32" }, async () => {
  const root = tempRoot(); const outside = `${root}-outside`; mkdirSync(outside); writeFileSync(join(outside, "secret"), "secret"); symlinkSync(outside, join(root, "alias"), "dir"); symlinkSync("alias/secret", join(root, "indirect")); const ledger = await collectLedger(scope(root));
  assert.equal(ledger.records.find((record) => record.id === "root-0/indirect").target_scope, "external_target_not_hashed"); rmSync(root, { recursive: true, force: true }); rmSync(outside, { recursive: true, force: true });
});

test("external symlink targets are explicitly out of scope even when unreadable", { skip: process.platform === "win32" }, async () => {
  const root = tempRoot(); const target = join(root, "link"); symlinkSync("/path/that/does/not/exist", target); const ledger = await collectLedger(scope(root)); const link = ledger.records.find((record) => record.kind === "symlink");
  assert.equal(link.target_scope, "external_target_not_hashed"); assert.equal(ledger.aggregate.external_target_count, 1); rmSync(root, { recursive: true, force: true });
});

test("unsafe scopes fail closed", () => {
  const root = tempRoot(); const common = scope(root); mkdirSync(join(root, "subhome")); assert.throws(() => validateScope({ ...common, approvedRoots: [] }), (error) => error instanceof LedgerError && error.code === "empty_scope"); assert.throws(() => validateScope({ ...common, approvedRoots: ["/"] }), (error) => error instanceof LedgerError && error.code === "unsafe_scope"); assert.throws(() => validateScope({ ...common, approvedRoots: [root, root] }), (error) => error instanceof LedgerError && error.code === "conflicting_scope"); assert.throws(() => validateScope({ ...common, approvedRoots: [root], evidenceDir: root }), (error) => error instanceof LedgerError && error.code === "unsafe_scope"); assert.throws(() => validateScope({ ...common, approvedRoots: [root], repositoryDir: root }), (error) => error instanceof LedgerError && error.code === "unsafe_scope"); assert.throws(() => validateScope({ ...common, approvedRoots: [root], homeDir: join(root, "subhome") }), (error) => error instanceof LedgerError && error.code === "unsafe_scope"); assert.throws(() => validateScope({ ...common, approvedRoots: [root], evidenceDir: join(dirname(root), "evidence"), homeDir: dirname(root) }), (error) => error instanceof LedgerError && error.code === "unsafe_scope"); rmSync(root, { recursive: true, force: true });
});

test("special files fail closed", { skip: process.platform === "win32" }, async () => {
  const root = tempRoot(); const special = join(root, "pipe"); execFileSync("mkfifo", [special]); await assert.rejects(() => collectLedger(scope(root)), (error) => error instanceof LedgerError && error.code === "special_file"); rmSync(root, { recursive: true, force: true });
});

test("config files are explicit and privacy-safe redaction contains no path, name, or content", async () => {
  const root = tempRoot(); const config = join(dirname(root), "agent-config.json"); writeFileSync(config, '{"private":"value"}'); const before = await collectLedger(scope(root, { configPaths: [config] })); const after = await collectLedger(scope(root, { configPaths: [config] })); const redacted = redactedComparison(before, after);
  const serialized = JSON.stringify(redacted); assert.equal(redacted.comparison.equal, true); assert.equal(serialized.includes(config), false); assert.equal(serialized.includes("agent-config.json"), false); assert.equal(serialized.includes("private"), false); assert.equal(redacted.privacy.absolute_paths, false); rmSync(config, { force: true }); rmSync(root, { recursive: true, force: true });
});

test("raw output is exclusive and confined to the isolated evidence directory", async () => {
  const root = tempRoot(); const evidence = `${root}-evidence`; const ledger = await collectLedger(scope(root, { evidenceDir: evidence })); const output = join(evidence, "before.json"); const written = writeRawLedger(ledger, output, evidence); assert.equal(JSON.parse(readFileSync(written)).format, "skillroster-byte-ledger"); assert.throws(() => writeRawLedger(ledger, output, evidence), (error) => error instanceof LedgerError && error.code === "write_failed"); assert.throws(() => writeRawLedger(ledger, join(root, "escape.json"), evidence), (error) => error instanceof LedgerError && error.code === "unsafe_output"); rmSync(root, { recursive: true, force: true }); rmSync(evidence, { recursive: true, force: true });
});

test("config identity and symlink inputs are validated before canonicalization", { skip: process.platform === "win32" }, async () => {
  const root = tempRoot(); const config = join(`${root}-config`, "settings.json"); mkdirSync(dirname(config), { recursive: true }); writeFileSync(config, "{}"); const common = scope(root, { configPaths: [config] });
  assert.throws(() => validateScope({ ...common, configPaths: [config, config] }), (error) => error instanceof LedgerError && error.code === "conflicting_scope");
  const rootAlias = `${root}-alias`; symlinkSync(root, rootAlias, "dir"); assert.throws(() => validateScope({ ...common, approvedRoots: [rootAlias] }), (error) => error instanceof LedgerError && error.code === "unsafe_scope");
  const configAlias = `${config}-alias`; symlinkSync(config, configAlias); assert.throws(() => validateScope({ ...common, configPaths: [configAlias] }), (error) => error instanceof LedgerError && error.code === "unsafe_scope");
  rmSync(root, { recursive: true, force: true }); rmSync(rootAlias, { force: true }); rmSync(dirname(config), { recursive: true, force: true });
});

test("CLI fails closed for unsafe list inputs and missing flag values", () => {
  const script = fileURLToPath(new URL("../../../scripts/byte-ledger.mjs", import.meta.url));
  const missing = spawnSync(process.execPath, [script, "capture", "--roots-file"], { encoding: "utf8" });
  assert.notEqual(missing.status, 0); assert.match(missing.stderr, /requires a value/u);
  const unreadable = spawnSync(process.execPath, [script, "capture", "--roots-file", "/path/does/not/exist", "--evidence-dir", "/tmp/evidence", "--output", "/tmp/evidence/x.json", "--repository-dir", "/tmp/repo", "--state-dir", "/tmp/state", "--home-dir", "/tmp/home"], { encoding: "utf8" });
  assert.notEqual(unreadable.status, 0); assert.match(unreadable.stderr, /unreadable_input/u);
  const incomplete = spawnSync(process.execPath, [script, "capture", "--roots-file", "/tmp/roots.txt", "--evidence-dir", "/tmp/evidence", "--output", "/tmp/evidence/x.json"], { encoding: "utf8" });
  assert.notEqual(incomplete.status, 0); assert.match(incomplete.stderr, /explicit roots/u);
});

test("compare validates schema, identities, scope, and aggregates before redaction", async () => {
  const root = tempRoot(); const ledger = await collectLedger(scope(root));
  assert.throws(() => validateLedger({ ...ledger, aggregate: { ...ledger.aggregate, file_count: ledger.aggregate.file_count + 1 } }), (error) => error instanceof LedgerError && error.code === "invalid_ledger");
  assert.throws(() => validateLedger({ ...ledger, records: [{ ...ledger.records[0], id: "../private" }, ...ledger.records.slice(1)] }), (error) => error instanceof LedgerError && error.code === "invalid_ledger");
  assert.throws(() => compareLedgers(ledger, { ...ledger, scope: { ...ledger.scope, config_path_count: 1 } }), (error) => error instanceof LedgerError && error.code === "invalid_ledger");
  const redacted = redactedComparison(ledger, ledger); assert.deepEqual(Object.keys(redacted).sort(), ["after", "before", "comparison", "external_target_bytes", "format", "privacy", "schema_version"]); assert.equal(JSON.stringify(redacted).includes(root), false);
  rmSync(root, { recursive: true, force: true });
});
