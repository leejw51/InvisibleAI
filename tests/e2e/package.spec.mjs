import { test, expect } from '@playwright/test';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { gunzipSync } from 'node:zlib';

test('packaged release contains the backend, capture client and local AI workers and excludes stale files', () => {
  const name = `causewaybay-${os.platform()}-${os.arch()}`;
  const root = path.resolve('package', name);
  const stale = ['bin/causewaybay', 'bin/causewaybay-server', 'bin/rust-cli', 'bin/rust_cli', '.env.privacy-regression', 'dist/stale-privacy-fixture.js', 'docs/stale-privacy-fixture.md'];
  try {
    for (const file of stale) fs.writeFileSync(path.join(root, file), 'synthetic stale-file fixture');
    execFileSync('bash', ['scripts/package.sh'], { encoding: 'utf8' });
    const entries = execFileSync('tar', ['-tzf', path.resolve('package', `${name}.tar.gz`)], { encoding: 'utf8' }).trim().split('\n');
    for (const file of stale) {
      expect(fs.existsSync(path.join(root, file))).toBe(false);
      expect(entries).not.toContain(`${name}/${file}`);
    }
    expect(entries).toContain(`${name}/.env.example`);
    expect(fs.readdirSync(path.join(root, 'bin')).sort()).toEqual(['client', 'invisible-apple-ai', 'invisible-mlx', 'mlx.metallib', 'server']);
    for (const [output, cargoName] of [['server', 'causewaybay-server'], ['client', 'rust-cli']]) {
      const binary = fs.readFileSync(path.resolve('bin', output));
      expect(binary.equals(fs.readFileSync(path.resolve('target/release', cargoName)))).toBe(true);
      expect(binary.equals(fs.readFileSync(path.join(root, 'bin', output)))).toBe(true);
      fs.accessSync(path.resolve('bin', output), fs.constants.X_OK);
    }
    expect(entries.filter(entry => entry.startsWith(`${name}/bin/`) && !entry.endsWith('/')).sort()).toEqual([
      `${name}/bin/client`, `${name}/bin/invisible-apple-ai`, `${name}/bin/invisible-mlx`, `${name}/bin/mlx.metallib`, `${name}/bin/server`,
    ]);
    // macOS tar hides AppleDouble entries in its listing, so also inspect the raw archive.
    const archiveBytes = gunzipSync(fs.readFileSync(path.resolve('package', `${name}.tar.gz`)));
    expect(archiveBytes.includes(Buffer.from(`${name}/bin/._`))).toBe(false);
  } finally {
    for (const file of stale) fs.rmSync(path.join(root, file), { force: true });
  }
});

test('packaged release binary serves its WASM interface and Tailwind assets', async ({ page, request }) => {
  const root = path.resolve(`package/causewaybay-${os.platform()}-${os.arch()}`);
  expect(fs.readFileSync(path.join(root, '.env.example'), 'utf8')).toContain('XAI_API_KEY=');
  expect(fs.existsSync(path.join(root, '.env'))).toBe(false);
  expect(execFileSync(path.join(root,'bin/client'),['--help'],{encoding:'utf8'})).toContain('rust-cli http://backend.example:3000');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-package-'));
  const moved = path.join(dir, 'Moved Invisible AI');
  fs.cpSync(root, moved, {recursive:true});
  expect(JSON.parse(execFileSync(path.join(moved,'bin/invisible-mlx'),['--self-test'],{cwd:dir,encoding:'utf8'})).ok).toBe(true);
  const child = spawn(path.join(moved, 'bin/server'), ['serve', '--bind', '127.0.0.1:3114', '--no-semantic', '--config-dir', path.join(dir,'config')], { cwd: dir, env: { ...process.env, DATA_DIR: path.join(dir, 'data'), APP_TOKEN: '', XAI_API_KEY: '', ANTHROPIC_API_KEY: '', OPENAI_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore' });
  try {
    await expect.poll(async () => {
      try { return (await request.get('http://127.0.0.1:3114/api/health')).ok(); } catch { return false; }
    }).toBeTruthy();
    await page.goto('http://127.0.0.1:3114');
    await expect(page.getByRole('heading', { name: 'Hey, what’s on your mind?', exact:true })).toBeVisible();
    expect((await request.get('http://127.0.0.1:3114/assets/app.css')).ok()).toBeTruthy();
    const artwork = await request.get('http://127.0.0.1:3114/assets/everyday-companion.png');
    expect(artwork.ok()).toBeTruthy();
    expect(artwork.headers()['content-type']).toContain('image/png');
    await page.screenshot({ path: 'test-results/release-desktop.png', fullPage: true });
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit');
      child.kill('SIGTERM');
      // Open browser WebSockets may keep graceful shutdown waiting. Bound test cleanup.
      const force = setTimeout(() => child.kill('SIGKILL'), 2000);
      try { await exited; } finally { clearTimeout(force); }
    }
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('client.zip has executable client and go.sh; launching from a path with spaces preserves URL and wallet',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'cwb-client-package-'));
  const stage=path.join(dir,'build stage'), unpack=path.join(dir,'unpacked client');
  const zip=path.join(dir,'client.zip'), wallet='0x9858effd232b4033e47d90003d41ec34ecaeda94';
  try {
    expect(execFileSync('unzip',['-Z1','package/client.zip'],{encoding:'utf8'}).trim().split('\n').sort()).toEqual(['client','go.sh']);
    execFileSync('node',['scripts/package-client.mjs',stage,zip],{env:{...process.env,PACKAGE_BACKEND_URL:'http://100.64.0.20:4567',OWNER_WALLET:wallet,APP_TOKEN:''}});
    fs.mkdirSync(unpack);execFileSync('unzip',['-q',zip,'-d',unpack]);
    expect(fs.readFileSync(path.join(unpack,'client')).equals(fs.readFileSync('bin/client'))).toBe(true);
    for(const file of ['client','go.sh'])fs.accessSync(path.join(unpack,file),fs.constants.X_OK);
    const script=fs.readFileSync(path.join(unpack,'go.sh'),'utf8');
    expect(script).toContain(`owner_wallet='${wallet}'`);
    expect(script).toContain("backend_url='http://100.64.0.20:4567'");
    expect(script).not.toContain('@@');
    // Replace only the extracted test executable, to validate launcher arguments without a network node.
    fs.writeFileSync(path.join(unpack,'client'),'#!/bin/sh\nprintf "%s\\n" "$@"\n',{mode:0o755});
    const args=execFileSync(path.join(unpack,'go.sh'),['--retry-seconds','2'],{cwd:dir,encoding:'utf8',env:{...process.env,APP_TOKEN:'synthetic-test-token'}}).trim().split('\n');
    expect(args).toEqual(['http://100.64.0.20:4567',wallet,'--retry-seconds','2']);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
