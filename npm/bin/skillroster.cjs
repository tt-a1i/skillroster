#!/usr/bin/env node
'use strict';

const { createHash } = require('node:crypto');
const { readFileSync } = require('node:fs');
const { dirname, join } = require('node:path');
const { spawn } = require('node:child_process');
const platforms = require('../platforms.json');
const manifest = require('../package.json');

try {
  const platform = `${process.platform}-${process.arch}`;
  if (!platforms[platform]) throw new Error(`Unsupported platform ${platform}; see the GitHub Release installation guide.`);
  const name = `@tt-a1i/skillroster-${platform}`;
  let packagePath;
  try {
    packagePath = require.resolve(`${name}/package.json`);
  } catch {
    throw new Error(`Missing ${name}. Reinstall skillroster with optional dependencies enabled (omit --omit=optional). No fallback download is performed.`);
  }
  const installed = JSON.parse(readFileSync(packagePath, 'utf8'));
  if (installed.version !== manifest.version) throw new Error(`Binary package version mismatch: expected ${manifest.version}, found ${installed.version}.`);
  const binary = join(dirname(packagePath), 'bin', process.platform === 'win32' ? 'skillroster.exe' : 'skillroster');
  const expected = readFileSync(`${binary}.sha256`, 'utf8').trim();
  const actual = createHash('sha256').update(readFileSync(binary)).digest('hex');
  if (!/^[a-f0-9]{64}$/.test(expected) || actual !== expected) throw new Error(`Binary checksum mismatch for ${name}; reinstall the package.`);
  const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
  child.on('error', error => { process.stderr.write(`skillroster: ${error.message}\n`); process.exitCode = 1; });
  child.on('exit', (code, signal) => {
    if (signal) { process.removeAllListeners(signal); process.kill(process.pid, signal); }
    else process.exitCode = code ?? 1;
  });
} catch (error) {
  process.stderr.write(`skillroster: ${error.message}\n`);
  process.exitCode = 1;
}
