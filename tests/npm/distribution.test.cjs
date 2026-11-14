'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdtempSync, readFileSync, writeFileSync, readdirSync, rmSync, mkdirSync } = require('node:fs');
const { join, resolve } = require('node:path');
const { tmpdir } = require('node:os');
const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');

const root = resolve(__dirname, '../..');
const platforms = require('../../npm/platforms.json');
const manifest = require('../../npm/package.json');
const version = manifest.version;
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const python = process.platform === 'win32' ? 'python' : 'python3';
const fixtureScript = `
import hashlib, io, json, pathlib, sys, tarfile, zipfile
root, artifacts, version = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
artifacts.mkdir()
binary = b'#!/usr/bin/env node\\nif(process.argv[2]==="mcp")process.stdin.pipe(process.stdout);else{console.log(JSON.stringify(process.argv.slice(2)));process.exitCode=process.argv[2]==="fail"?7:0;}\\n'
for platform, target in json.loads((root/'npm/platforms.json').read_text()).items():
    name = f'skillroster-{version}-{target}'
    windows = platform.startswith('win32-')
    member = name + ('/skillroster.exe' if windows else '/skillroster')
    archive = artifacts / (name + ('.zip' if windows else '.tar.gz'))
    if windows:
        with zipfile.ZipFile(archive, 'w') as package: package.writestr(member, binary)
    else:
        with tarfile.open(archive, 'w:gz') as package:
            info = tarfile.TarInfo(member); info.size = len(binary); info.mode = 0o755
            package.addfile(info, io.BytesIO(binary))
    (artifacts/(archive.name+'.sha256')).write_text(hashlib.sha256(archive.read_bytes()).hexdigest()+'  '+archive.name+'\\n')
`;

function fixture(t) {
  const temporary = mkdtempSync(join(tmpdir(), 'skillroster-npm-test-'));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const artifacts = join(temporary, 'artifacts');
  const result = spawnSync(python, ['-c', fixtureScript, root, artifacts, version], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return { temporary, artifacts, output: join(temporary, 'packages') };
}

function build(fixture) {
  return spawnSync(python, [join(root, 'scripts/release/build-npm.py'), '--version', version, '--artifacts', fixture.artifacts, '--output', fixture.output], { encoding: 'utf8' });
}

test('npm templates match Cargo, exact optional dependencies and the four release targets', () => {
  assert.equal(version, readFileSync(join(root, 'Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)[1]);
  assert.equal(manifest.bin.skillroster, 'bin/skillroster.cjs');
  assert.equal(manifest.scripts, undefined);
  assert.equal(Object.keys(platforms).length, 4);
  for (const platform of Object.keys(platforms)) {
    const native = require(`../../npm/platforms/${platform}/package.json`);
    assert.equal(native.name, `@tt-a1i/skillroster-${platform}`);
    assert.equal(native.version, version);
    assert.equal(manifest.optionalDependencies[native.name], version);
    assert.equal(native.scripts, undefined);
    assert.equal(`${native.os[0]}-${native.cpu[0]}`, platform);
  }
});

test('staging verifies all archives, writes licenses and provenance, and refuses overwrite or bad hashes', t => {
  const files = fixture(t);
  const wrongVersion = spawnSync(python, [join(root, 'scripts/release/build-npm.py'), '--version', '0.0.0', '--artifacts', files.artifacts, '--output', files.output], { encoding: 'utf8' });
  assert.notEqual(wrongVersion.status, 0);
  assert.match(wrongVersion.stderr, /must match source candidate/);
  const result = build(files);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readdirSync(files.output).sort(), [...Object.keys(platforms), 'skillroster'].sort());
  for (const [platform, target] of Object.entries(platforms)) {
    const packageDir = join(files.output, platform);
    const binaryName = platform.startsWith('win32-') ? 'skillroster.exe' : 'skillroster';
    const binary = join(packageDir, 'bin', binaryName);
    assert.equal(readFileSync(`${binary}.sha256`, 'utf8').trim(), sha256(readFileSync(binary)));
    assert.equal(JSON.parse(readFileSync(join(packageDir, 'provenance.json'))).target, target);
    assert.equal(readFileSync(join(packageDir, 'LICENSE'), 'utf8'), readFileSync(join(root, 'LICENSE'), 'utf8'));
  }
  assert.notEqual(build(files).status, 0);
  assert.ok(readFileSync(join(files.output, 'skillroster/package.json')));
  const corrupt = fixture(t);
  const archive = readdirSync(corrupt.artifacts).find(name => name.endsWith('.tar.gz'));
  writeFileSync(join(corrupt.artifacts, archive), 'corrupt');
  const blocked = build(corrupt);
  assert.notEqual(blocked.status, 0);
  assert.match(blocked.stderr, /SHA-256 mismatch/);
  assert.ok(!readdirSync(corrupt.temporary).includes('packages'));
  assert.deepEqual(readdirSync(corrupt.temporary), ['artifacts']);
});

test('offline npm packs install the native optional package and preserve argv, MCP stdio, exit code and hash refusal', { skip: process.platform === 'win32' }, t => {
  const files = fixture(t);
  assert.equal(build(files).status, 0);
  const cache = join(files.temporary, 'npm-cache');
  const tarballs = [];
  for (const packageName of ['skillroster', ...Object.keys(platforms)]) {
    const pack = spawnSync('npm', ['pack', '--ignore-scripts', '--json', '--cache', cache, '--pack-destination', files.temporary], { cwd: join(files.output, packageName), encoding: 'utf8' });
    assert.equal(pack.status, 0, pack.stderr);
    const packed = JSON.parse(pack.stdout)[0];
    assert.ok(packed.files.some(file => file.path === 'LICENSE'));
    if (packageName !== 'skillroster') assert.ok(packed.files.some(file => file.path.endsWith('.sha256')));
    tarballs.push({ package: packageName, path: join(files.temporary, packed.filename) });
  }
  const platform = `${process.platform}-${process.arch}`;
  assert.ok(platforms[platform], `unsupported test host ${platform}`);
  const install = join(files.temporary, 'install');
  mkdirSync(install);
  const selected = tarballs.filter(file => ['skillroster', platform].includes(file.package));
  const installed = spawnSync('npm', ['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', '--cache', cache, '--prefix', install, ...selected.map(file => file.path)], { encoding: 'utf8' });
  assert.equal(installed.status, 0, installed.stderr);
  const wrapper = join(install, 'node_modules/skillroster/bin/skillroster.cjs');
  const args = ['check', '--json', '--', '$(literal text); with spaces'];
  const probe = spawnSync(process.execPath, [wrapper, ...args], { encoding: 'utf8' });
  assert.equal(probe.status, 0, probe.stderr);
  assert.deepEqual(JSON.parse(probe.stdout), args);
  const frame = '{"jsonrpc":"2.0","id":1,"method":"ping"}\n';
  const mcp = spawnSync(process.execPath, [wrapper, 'mcp'], { input: frame, encoding: 'utf8' });
  assert.equal(mcp.status, 0, mcp.stderr);
  assert.equal(mcp.stdout, frame);
  assert.equal(spawnSync(process.execPath, [wrapper, 'fail']).status, 7);
  const binary = join(install, `node_modules/@tt-a1i/skillroster-${platform}/bin/skillroster`);
  writeFileSync(binary, 'changed');
  const blocked = spawnSync(process.execPath, [wrapper, 'check'], { encoding: 'utf8' });
  assert.notEqual(blocked.status, 0);
  assert.equal(blocked.stdout, '');
  assert.match(blocked.stderr, /checksum mismatch/);
  rmSync(join(install, `node_modules/@tt-a1i/skillroster-${platform}`), { recursive: true });
  assert.match(spawnSync(process.execPath, [wrapper], { encoding: 'utf8' }).stderr, /optional dependencies/);
});
