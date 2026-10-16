import {openConversationOptions} from './workspace-controls.mjs';
import { test, expect } from '@playwright/test';
import { spawn as spawnChild } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// A test that times out skips its finally block, so every process this file
// starts is also stopped after each test; a stray server would hold the port
// for the next run.
const children = new Set();
function spawn(...args) { const child = spawnChild(...args); children.add(child); return child; }
test.afterEach(async () => {
  for (const child of children) {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit');
      child.kill('SIGTERM');
      await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 3000))]);
      if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await exited; }
    }
  }
  children.clear();
});


const binary = path.resolve('target/debug/causewaybay-server');
const assets = path.resolve('dist');
const base = 'http://127.0.0.1:3115';
function workspace() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-env-'));
  const config = path.join(root, 'config');
  fs.mkdirSync(config);
  return { root, config };
}
function start({ root, config }, overrides = {}) {
  const env = { ...process.env, APP_TOKEN: '', DATA_DIR: path.join(config, 'data'), DEMO_MODE: '0', SEMANTIC_SEARCH: 'off', ANTHROPIC_API_KEY: '', OPENAI_API_KEY: '', GEMINI_API_KEY: '', ...overrides };
  if (!Object.hasOwn(overrides, 'XAI_API_KEY')) delete env.XAI_API_KEY;
  const child = spawn(binary, ['serve', '--bind', '127.0.0.1:3115', '--config-dir', config, '--assets', assets], { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', b => { output += b; });
  child.stderr.on('data', b => { output += b; });
  return { child, output: () => output };
}
async function stop(child) {
  if (child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit');
    child.kill('SIGTERM');
    await exited;
  }
}
async function ready(request) {
  await expect.poll(async () => {
    try { return (await request.get(`${base}/api/health`)).ok(); } catch { return false; }
  }).toBeTruthy();
  return (await request.get(`${base}/api/settings`)).json();
}

test('backend loads working-directory .env; browser needs no key setup', async ({ page, request }) => {
  const dir = workspace();
  fs.writeFileSync(path.join(dir.root, '.env'), 'XAI_API_KEY="working-directory-test-key" # quoted value\n', { mode: 0o600 });
  const server = start(dir);
  try {
    const settings = await ready(request);
    expect(settings.provider).toBe('grok');
    expect(settings.providers.find(p => p.id === 'grok').configured).toBe(true);
    expect(JSON.stringify(settings)).not.toContain('working-directory-test-key');
    // Old browser provider preferences cannot override the backend's available provider.
    await page.addInitScript(() => {
      localStorage.setItem('causewaybay.settings.v1', JSON.stringify({ provider: 'anthropic' }));
      localStorage.setItem('causewaybay.chat.local', JSON.stringify({ provider: 'anthropic' }));
    });
    await page.goto(base);
    await openConversationOptions(page); await page.locator('.chat-options > summary').click();
    await expect(page.getByRole('combobox', { name: 'Chat AI provider', exact: true })).toHaveValue('grok');
    await expect(page.getByLabel('Capture setup')).toContainText('Connect a computer');
    await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeDisabled();
    await page.getByRole('button', {name:'Connect a device', exact:true}).click();
    await expect(page.getByLabel('Capture client command')).toHaveText(`./bin/client ${base}`);
    await page.getByRole('link', {name:'Return to New thought'}).click();
    await expect(page.getByRole('heading', {name:'Hey, what’s on your mind?', exact:true})).toBeVisible();
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    await expect(page.getByRole('button', { name: /Grok Available/ })).toBeEnabled();
    await expect(page.getByRole('button', { name: /Anthropic Not configured/ })).toBeDisabled();
    await expect(page.getByLabel('API key', { exact: true })).toHaveCount(0);
    await page.screenshot({ path: 'test-results/backend-provider-settings.png', fullPage: true });
    expect(fs.readFileSync(path.join(dir.config, 'backend.jsonl'), 'utf8')).not.toContain('working-directory-test-key');
    expect(server.output()).not.toContain('working-directory-test-key');
  } finally { await stop(server.child); }
});

test('backend loads config-directory .env and exported variables take precedence', async ({ request }) => {
  const dir = workspace();
  fs.writeFileSync(path.join(dir.config, '.env'), 'XAI_API_KEY=config-directory-test-key\n', { mode: 0o600 });
  let server = start(dir);
  try {
    expect((await ready(request)).provider).toBe('grok');
  } finally { await stop(server.child); }
  // An explicitly empty exported value disables a key even if a file sets it.
  server = start(dir, { XAI_API_KEY: '', OPENAI_API_KEY: 'exported-test-key' });
  try {
    const settings = await ready(request);
    expect(settings.provider).toBe('openai');
    expect(settings.providers.find(p => p.id === 'grok').configured).toBe(false);
    expect(settings.providers.find(p => p.id === 'openai').configured).toBe(true);
    expect(JSON.stringify(settings)).not.toContain('exported-test-key');
    expect(server.output()).not.toContain('exported-test-key');
  } finally { await stop(server.child); }
});

test('invalid .env fails clearly without printing the secret-bearing line', async () => {
  const dir = workspace();
  fs.writeFileSync(path.join(dir.root, '.env'), 'XAI_API_KEY="malformed-private-test-key\n', { mode: 0o600 });
  const server = start(dir);
  try {
    const [code] = await once(server.child, 'exit');
    expect(code).not.toBe(0);
    expect(server.output()).toContain('Check permissions and dotenv syntax');
    expect(server.output()).not.toContain('malformed-private-test-key');
  } finally { await stop(server.child); }
});
