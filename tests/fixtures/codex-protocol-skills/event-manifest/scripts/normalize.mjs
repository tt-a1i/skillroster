#!/usr/bin/env node

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  process.stderr.write("usage: normalize.mjs INPUT.psv OUTPUT.json\n");
  process.exit(64);
}

const records = readFileSync(resolve(input), "utf8").trim().split("\n").map((line, index) => {
  const parts = line.split("|");
  if (parts.length !== 4 || parts.some((part) => !part.trim())) throw new Error(`invalid row ${index + 1}`);
  const [id, occurredAt, owner, state] = parts.map((part) => part.trim());
  if (!/^evt-[0-9]{2}$/u.test(id) || !["open", "closed"].includes(state) || Number.isNaN(Date.parse(occurredAt))) throw new Error(`invalid row ${index + 1}`);
  return { id, occurred_at: occurredAt, owner, state };
}).sort((left, right) => left.id.localeCompare(right.id));

mkdirSync(dirname(resolve(output)), { recursive: true });
writeFileSync(resolve(output), `${JSON.stringify({ schema_version: 1, records }, null, 2)}\n`);
