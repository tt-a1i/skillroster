#!/usr/bin/env node
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const [inputArg, outputArg] = process.argv.slice(2);
if (!inputArg || !outputArg) process.exit(64);
const inputPath = resolve(inputArg);
const outputDir = resolve(outputArg);
const input = JSON.parse(readFileSync(inputPath, "utf8"));
if (typeof input.title !== "string" || !Array.isArray(input.items)) process.exit(65);
const items = input.items.map((item, index) => ({
  position: index + 1,
  name: String(item.name),
  status: String(item.status),
}));
mkdirSync(outputDir, { recursive: true });
writeFileSync(join(outputDir, "report.json"), `${JSON.stringify({ title: input.title, item_count: items.length, items }, null, 2)}\n`);
writeFileSync(join(outputDir, "README.md"), `# ${input.title}\n\nItems: ${items.length}\n\n${items.map((item) => `${item.position}. ${item.name} — ${item.status}`).join("\n")}\n`);
