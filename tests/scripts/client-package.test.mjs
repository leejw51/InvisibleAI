import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync, execFileSync} from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wallet = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const backend = 'http://100.64.0.20:4567';

// An isolated build tree with a harmless executable: no real account, screen, or server.
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-launcher-test-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  for (const directory of ['scripts', 'bin', 'packaging', 'build stage']) fs.mkdirSync(path.join(root, directory));
  for (const file of ['client-go.sh', 'package-client.mjs']) fs.copyFileSync(path.resolve('scripts', file), path.join(root, 'scripts', file));
  for (const file of ['client.env.example', 'QUICK.md', 'README.md']) fs.copyFileSync(path.resolve('packaging', file), path.join(root, 'packaging', file));
  fs.writeFileSync(path.join(root, 'bin/client'), '#!/bin/sh\nprintf "%s\\n" "$PWD" "${APP_TOKEN-}" "$@" > "$LAUNCH_RECORD"\nexit "${LAUNCH_EXIT_CODE:-0}"\n', {mode: 0o755});
  const stage = path.join(root, 'build stage'), zip = path.join(root, 'client.zip');
  const env = {...process.env, OWNER_WALLET: wallet, PACKAGE_BACKEND_URL: backend, APP_TOKEN: ''};
  function build(overrides = {}) {
    return spawnSync(process.execPath, ['scripts/package-client.mjs', stage, zip], {cwd: root, env: {...env, ...overrides}, encoding: 'utf8'});
  }
  function unpack() {
    const destination = path.join(root, 'unpacked client');
    fs.mkdirSync(destination);
    execFileSync('unzip', ['-q', zip, '-d', destination]);
    return path.join(destination, 'build stage');
  }
  function launch(destination, args = [], overrides = {}, input = '') {
    const record = path.join(root, 'launch-record');
    const result = spawnSync(path.join(destination, 'go.sh'), args, {cwd: root, env: {...env, LAUNCH_RECORD: record, ...overrides}, encoding: 'utf8', input});
    return {...result, args: fs.existsSync(record) ? fs.readFileSync(record, 'utf8').trimEnd().split('\n') : null};
  }
  return {root, stage, zip, build, unpack, launch};
}

test('rebuilding removes stale ZIP entries and restores executable launcher permissions', t => {
  const f = fixture(t);
  fs.writeFileSync(path.join(f.stage, '.env'), 'SYNTHETIC_STALE_SECRET=must-not-ship');
  fs.writeFileSync(path.join(f.stage, 'go.sh'), 'obsolete launcher', {mode: 0o600});
  // Simulate an interrupted previous build leaving a partially assembled ZIP behind.
  execFileSync('zip', ['-q', 'client.zip', '.env', 'go.sh'], {cwd: f.stage});
  fs.writeFileSync(f.zip, 'old published archive');
  const built = f.build();
  assert.equal(built.status, 0, built.stderr);
  assert.deepEqual(execFileSync('unzip', ['-Z1', f.zip], {encoding: 'utf8'}).trim().split('\n').sort(),
    ['.env.example', 'QUICK.md', 'README.md', 'client', 'go.sh'].map(file => `build stage/${file}`));
  const unpacked = f.unpack();
  for (const file of ['client', 'go.sh']) fs.accessSync(path.join(unpacked, file), fs.constants.X_OK);
  assert.equal(fs.readFileSync(path.join(unpacked, '.env.example'), 'utf8'), fs.readFileSync('packaging/client.env.example', 'utf8'));
  assert.equal(fs.existsSync(path.join(unpacked, '.env')), false);
  assert.equal(fs.readFileSync(path.join(unpacked, 'client'), 'utf8'), fs.readFileSync(path.join(f.root, 'bin/client'), 'utf8'));
});

test('invalid wallet and non-Tailscale URLs are rejected without replacing an existing archive', t => {
  const f = fixture(t);
  const previous = Buffer.from('previous usable archive');
  fs.writeFileSync(f.zip, previous);
  const invalid = [
    ...['http://127.0.0.1:3000', 'http://192.168.1.2:3000', 'http://example.com',
      'http://100.63.255.255', 'http://100.128.0.1', 'http://[::1]',
      'ftp://100.64.0.20', 'http://name:password@100.64.0.20', 'http://100.64.0.20/private',
      'http://100.64.0.20?token=secret', 'http://100.64.0.20#secret',
    ].map(PACKAGE_BACKEND_URL => ({PACKAGE_BACKEND_URL})),
    {OWNER_WALLET: 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'},
    {OWNER_WALLET: "0x123'$(touch unsafe)"},
  ];
  for (const input of invalid) {
    const built = f.build(input);
    assert.notEqual(built.status, 0, JSON.stringify(input));
    assert.deepEqual(fs.readFileSync(f.zip), previous);
    assert.equal(fs.existsSync(path.join(f.root, 'unsafe')), false);
  }
});

for (const url of ['http://100.64.0.1:4567', 'https://100.127.255.254:8443', 'http://[fd7a:115c:a1e0::1234]:4567']) {
  test(`launcher preserves Tailscale URL ${url}, wallet, spaces, and exit status`, t => {
    const f = fixture(t);
    const built = f.build({PACKAGE_BACKEND_URL: url, OWNER_WALLET: `  ${wallet.toUpperCase()}  `});
    assert.equal(built.status, 0, built.stderr);
    const unpacked = f.unpack();
    const launched = f.launch(unpacked, ['--name', 'My capture computer', '--retry-seconds', '2'], {LAUNCH_EXIT_CODE: '23', APP_TOKEN: 'synthetic-runtime-token'});
    assert.equal(launched.status, 23, launched.stderr);
    assert.deepEqual(launched.args, [fs.realpathSync(f.root), 'synthetic-runtime-token', url, wallet, '--name', 'My capture computer', '--retry-seconds', '2']);
  });
}

test('generic package prompts for a missing wallet and node token without embedding build secrets', t => {
  const f = fixture(t);
  const built = f.build({OWNER_WALLET: '', APP_TOKEN: 'synthetic-build-token-do-not-embed'});
  assert.equal(built.status, 0, built.stderr);
  const unpacked = f.unpack();
  assert.ok(!fs.readFileSync(path.join(unpacked, 'go.sh'), 'utf8').includes('synthetic-build-token-do-not-embed'));
  const token = 'synthetic-runtime-token';
  const launched = f.launch(unpacked, [], {}, `${wallet}\n${token}\n`);
  assert.equal(launched.status, 0, launched.stderr);
  assert.deepEqual(launched.args, [fs.realpathSync(f.root), token, backend, wallet]);
  assert.match(launched.stdout, /Owner wallet address/);
  assert.match(launched.stdout, /Backend node token/);
  assert.ok(!launched.stdout.includes(token));
});

test('prefilled package uses an environment token without prompting; missing token fails before launching', t => {
  const f = fixture(t);
  const built = f.build({APP_TOKEN: 'synthetic-build-token'});
  assert.equal(built.status, 0, built.stderr);
  const unpacked = f.unpack();
  const missing = f.launch(unpacked);
  assert.notEqual(missing.status, 0);
  assert.equal(missing.args, null);
  const launched = f.launch(unpacked, [], {APP_TOKEN: 'synthetic-environment-token'});
  assert.equal(launched.status, 0, launched.stderr);
  assert.equal(launched.stdout, '');
  assert.deepEqual(launched.args, [fs.realpathSync(f.root), 'synthetic-environment-token', backend, wallet]);
});

test('launcher fills missing values from .env without executing it; baked values and the environment win', t => {
  const f = fixture(t);
  // The backend URL is baked in; the .env URL must not replace it.
  const built = f.build({OWNER_WALLET: ''});
  assert.equal(built.status, 0, built.stderr);
  const unpacked = f.unpack();
  fs.writeFileSync(path.join(unpacked, '.env'), [
    '# synthetic client settings', 'BACKEND_URL=http://100.64.0.99:9999', `OWNER_WALLET=${wallet}`,
    'APP_TOKEN=synthetic-dotenv-token', 'UNRELATED=$(touch unsafe)', 'PATH=/nowhere',
  ].join('\n'));
  const fromFile = f.launch(unpacked);
  assert.equal(fromFile.status, 0, fromFile.stderr);
  assert.equal(fromFile.stdout, '');
  assert.deepEqual(fromFile.args, [fs.realpathSync(f.root), 'synthetic-dotenv-token', backend, wallet]);
  assert.equal(fs.existsSync(path.join(f.root, 'unsafe')), false);
  const fromEnvironment = f.launch(unpacked, [], {APP_TOKEN: 'synthetic-environment-token'});
  assert.deepEqual(fromEnvironment.args, [fs.realpathSync(f.root), 'synthetic-environment-token', backend, wallet]);
});
