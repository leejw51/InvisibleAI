import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// The fixture server listens on PW_PORT (see playwright.config.mjs); the
// spawned Rust client must join the same backend.
const base = `http://127.0.0.1:${process.env.PW_PORT || '3099'}`;

// Playwright skips a test's `finally` after a timeout, so children and extra
// pages are released from afterEach instead.
const cleanup = [];
test.afterEach(async () => {
  while (cleanup.length) { try { await cleanup.pop()(); } catch {} }
});
function stopChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  const exited = once(child, 'exit');
  child.kill('SIGTERM');
  const hammer = setTimeout(() => child.kill('SIGKILL'), 5000);
  return exited.finally(() => clearTimeout(hammer));
}

test('dashboard shows Rust and web clients joining and leaving', async ({ page, context, request }) => {
  const config = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-dashboard-'));
  const cli = spawn('./target/debug/causewaybay', ['client', '--backend', base, '--name', 'Dashboard MacBook', '--config-dir', config], { env: { ...process.env, APP_TOKEN: '' }, stdio: 'ignore' });
  cleanup.push(() => stopChild(cli));
  let second;
  cleanup.push(async () => { if (second) await second.close(); });
  await page.goto('/devices');
  await expect(page.getByRole('heading', { name: 'Your connected devices.' })).toBeVisible();
  await expect(page.getByTestId('backend-status')).toHaveText('Connected');
  await expect(page.getByTestId('rust-connected')).toHaveText('1');
  await expect(page.getByTestId('web-connected')).toHaveText('1');
  const rust = page.getByRole('region', { name: 'Rust capture clients' });
  await expect(rust.locator('.device-row').filter({ hasText: 'Dashboard MacBook' })).toContainText('Connected');
  await expect(page.getByText('This tab', { exact: true })).toBeVisible();
  second = await context.newPage();
  await second.goto('/');
  await expect(second.getByRole('button', { name: 'Backend connection' })).toHaveText('Connected');
  await expect(page.getByTestId('web-connected')).toHaveText('2');
  const data = await (await request.get('/api/dashboard')).json();
  expect(data.rust_clients.find(c => c.name === 'Dashboard MacBook').online).toBeTruthy();
  expect(data.web_clients.filter(c => c.online)).toHaveLength(2);
  await page.screenshot({ path: 'test-results/dashboard-desktop.png', fullPage: true });
  await second.close(); second = null;
  await expect(page.getByTestId('web-connected')).toHaveText('1');
  await stopChild(cli);
  await expect(page.getByTestId('rust-connected')).toHaveText('0');
  await expect(rust.locator('.device-row').filter({ hasText: 'Dashboard MacBook' })).toContainText('Offline');
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({ path: 'test-results/dashboard-mobile.png', fullPage: true });
});

test('web client sleeps between retries and restores its dashboard without reload', async ({ page, request }) => {
  const attempts = [];
  let available = true;
  let connection;
  await page.routeWebSocket('**/api/ws', ws => {
    attempts.push(Date.now());
    if (!available) { void ws.close(); return; }
    const server = ws.connectToServer();
    connection = { ws, server };
  });
  await page.goto('/devices');
  await expect(page.getByRole('button', { name: 'Backend connection' })).toHaveText('Connected');
  await expect(page.getByTestId('web-connected')).toHaveText('1');
  const before = (await (await request.get('/api/dashboard')).json()).web_clients.find(c => c.online).id;
  available = false;
  const disconnectedAt = Date.now();
  await connection.ws.close(); await connection.server.close();
  await expect(page.getByRole('button', { name: 'Backend connection' })).toHaveText('Reconnecting');
  await expect(page.getByText(/Waiting 2 seconds between retries/)).toBeVisible();
  await expect(page.getByTestId('web-connected')).toHaveText('—');
  await expect.poll(() => attempts.length, { timeout: 12000 }).toBeGreaterThanOrEqual(3);
  expect(attempts[1] - disconnectedAt).toBeGreaterThanOrEqual(1700);
  expect(attempts[2] - attempts[1]).toBeGreaterThanOrEqual(1700);
  available = true;
  await expect(page.getByRole('button', { name: 'Backend connection' })).toHaveText('Connected');
  await expect(page.getByTestId('web-connected')).toHaveText('1');
  const after = (await (await request.get('/api/dashboard')).json()).web_clients.filter(c => c.online);
  expect(after).toHaveLength(1);
  expect(after[0].id).toBe(before);
});
