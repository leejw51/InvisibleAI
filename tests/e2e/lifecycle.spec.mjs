import { test, expect } from '@playwright/test';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const exec = promisify(execFile);
test('make start2 and stop manage both processes and print connection URLs', async ({ request, page }) => {
  test.setTimeout(120000);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-make-'));
  const args = [`CONFIG_DIR=${dir}`, `CLIENT_CONFIG_DIR=${path.join(dir, 'client')}`, 'BIND_ADDR=0.0.0.0:3113', 'BACKEND_URL=http://127.0.0.1:3113'];
  const env = { ...process.env, SEMANTIC_SEARCH: 'off', APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' };
  try {
    const { stdout } = await exec('make', ['start2', ...args], { env });
    expect(stdout).toContain('Listening on    0.0.0.0:3113');
    expect(stdout).toContain('Web URL       http://127.0.0.1:3113');
    expect(stdout).toContain('API URL       http://127.0.0.1:3113/api');
    expect(stdout).toContain('API health');
    expect(stdout).toContain('/status');
    expect(stdout).toContain('Tailwind CSS');
    expect(stdout).toContain('Tailscale');
    expect((await request.get('http://127.0.0.1:3113/assets/app.css')).ok()).toBeTruthy();
    const networkHost = Object.values(os.networkInterfaces()).flat().find(a => a && a.family === 'IPv4' && !a.internal && !a.address.startsWith('169.254.'));
    if (networkHost) {
      const base = `http://${networkHost.address}:3113`;
      expect(stdout).toContain(`Web URL       ${base}`);
      expect(stdout).toContain(`API URL       ${base}/api`);
      expect((await request.get(`${base}/api/health`)).ok()).toBeTruthy();
      expect((await request.get(`${base}/`)).ok()).toBeTruthy();
    }
    await expect.poll(async () => (await (await request.get('http://127.0.0.1:3113/api/clients')).json()).some(c => c.online)).toBeTruthy();
    await page.goto('http://127.0.0.1:3113/devices');
    await expect(page.getByTestId('rust-connected')).toHaveText('1');
    await expect(page.getByTestId('web-connected')).toHaveText('1');
    expect(fs.readFileSync(path.join(dir, 'logs/backend.log'), 'utf8')).toContain('Rust clients: 1 · Web clients: 1');
    for (const name of ['backend', 'client']) {
      const records = fs.readFileSync(path.join(name === 'client' ? path.join(dir, 'client') : dir, `${name}.jsonl`), 'utf8').trim().split('\n').map(JSON.parse);
      expect(records.at(-1).version).toBe(1);
    }
  } finally { await exec('make', ['stop', ...args], { env }); }
  const { stdout } = await exec('make', ['status', ...args], { env });
  expect(stdout).toContain('server stopped');
  expect(stdout).toContain('client stopped');
});

test('make start, start3, and stop3 independently manage only the requested process', async ({ request }) => {
  test.setTimeout(120000);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-start-targets-'));
  const clientDir = path.join(dir, 'client');
  const args = [`CONFIG_DIR=${dir}`, `CLIENT_CONFIG_DIR=${clientDir}`, 'BIND_ADDR=127.0.0.1:3115', 'BACKEND_URL=http://127.0.0.1:3115'];
  const env = { ...process.env, SEMANTIC_SEARCH: 'off', APP_TOKEN: '', CWBBOT_ADMIN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' };
  const make = target => exec('make', [target, ...args], { env });
  const serverPid = path.join(dir, 'run/server.pid');
  const clientPid = path.join(clientDir, 'run/client.pid');
  try {
    const backend = await make('start');
    expect(backend.stdout).toContain('manage.sh start-server');
    expect(backend.stdout).not.toContain('cargo build -p causewaybay-cli');
    expect((await request.get('http://127.0.0.1:3115/api/health')).ok()).toBe(true);
    expect(fs.existsSync(serverPid)).toBe(true);
    expect(fs.existsSync(clientPid)).toBe(false);
    expect((await make('status')).stdout).toContain('client stopped');
    const originalServer = fs.readFileSync(serverPid, 'utf8');
    await make('start-server');
    expect(fs.readFileSync(serverPid, 'utf8')).toBe(originalServer);
    await make('stop');

    const client = await make('start3');
    expect(client.stdout).toContain('manage.sh start-client');
    expect(client.stdout).not.toContain('cargo build -p causewaybay-server');
    expect(client.stdout).not.toContain('trunk build');
    expect(fs.existsSync(serverPid)).toBe(false);
    expect(fs.existsSync(clientPid)).toBe(true);
    const status = (await make('status')).stdout;
    expect(status).toContain('server stopped');
    expect(status).toContain('client running');
    const originalClient = fs.readFileSync(clientPid, 'utf8');
    await make('start-client');
    expect(fs.readFileSync(clientPid, 'utf8')).toBe(originalClient);

    // The client can start before its remote backend and connect when it appears.
    await make('start');
    await expect.poll(async () => (await (await request.get('http://127.0.0.1:3115/api/clients')).json()).filter(c => c.online).length).toBe(1);
    expect(fs.readFileSync(clientPid, 'utf8')).toBe(originalClient);
    const runningServer = fs.readFileSync(serverPid, 'utf8');
    await make('start2');
    expect(fs.readFileSync(serverPid, 'utf8')).toBe(runningServer);
    expect(fs.readFileSync(clientPid, 'utf8')).toBe(originalClient);

    const stopped = await make('stop3');
    expect(stopped.stdout).toContain('client stopped.');
    expect(fs.existsSync(clientPid)).toBe(false);
    expect(fs.readFileSync(serverPid, 'utf8')).toBe(runningServer);
    expect((await request.get('http://127.0.0.1:3115/api/health')).ok()).toBe(true);
    await expect.poll(async () => (await (await request.get('http://127.0.0.1:3115/api/clients')).json()).filter(c => c.online).length).toBe(0);
    const afterStop = (await make('status')).stdout;
    expect(afterStop).toContain('server running');
    expect(afterStop).toContain('client stopped');
    expect((await make('stop3')).stdout).toContain('client is not running.');
    expect(fs.readFileSync(serverPid, 'utf8')).toBe(runningServer);

    await make('start3');
    await expect.poll(async () => (await (await request.get('http://127.0.0.1:3115/api/clients')).json()).filter(c => c.online).length).toBe(1);
    expect(fs.readFileSync(serverPid, 'utf8')).toBe(runningServer);
  } finally {
    await make('stop');
  }
  expect(fs.existsSync(serverPid)).toBe(false);
  expect(fs.existsSync(clientPid)).toBe(false);
});
