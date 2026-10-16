import { test, expect, chromium } from '@playwright/test';
import { observeEvents } from './events.mjs';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

test.setTimeout(180000);
const base = 'http://127.0.0.1:3116';
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-native-'));
const backendDir = path.join(home, '.causewaybaybot');
const clientDir = path.join(home, '.causewaybaybotclient');
let server, client, displayBrowser, displayPage;
let clientLog = '';
async function stop(child) {
  if (child && child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit'); child.kill('SIGTERM'); await exited;
  }
}
test.beforeAll(async ({ request }) => {
  expect(Boolean(process.env.XAI_API_KEY), 'XAI_API_KEY is required for the native screen-to-AI test').toBeTruthy();
  const env = { ...process.env, HOME: home, BIND_ADDR: '127.0.0.1:3116', SEMANTIC_SEARCH: 'off', DEMO_MODE: '0', APP_TOKEN: '', ANTHROPIC_API_KEY: '', OPENAI_API_KEY: '', GEMINI_API_KEY: '' };
  // Exercise the actual default folders, not --config-dir or fixture-image overrides.
  for (const key of ['DATA_DIR', 'CAUSEWAYBAY_CONFIG_DIR', 'CAUSEWAYBAY_CLIENT_CONFIG_DIR']) delete env[key];
  server = spawn('./target/debug/causewaybay-server', ['serve', '--assets', path.resolve('dist')], { env, stdio: 'ignore' });
  await expect.poll(async () => { try { return (await request.get(`${base}/api/health`)).ok(); } catch { return false; } }).toBeTruthy();
  client = spawn('./target/debug/causewaybay', ['client', '--backend', base, '--name', 'Native screen client'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  client.stdout.on('data', b => { clientLog += b; });
  client.stderr.on('data', b => { clientLog += b; });
  await expect.poll(async () => (await (await request.get(`${base}/api/clients`)).json()).some(c => c.online)).toBeTruthy();
  // Show a known arithmetic question on the primary desktop while driving the app headlessly.
  displayBrowser = await chromium.launch({ headless: false, args: ['--kiosk', '--start-fullscreen', '--window-position=0,0'] });
  displayPage = await displayBrowser.newPage({ viewport: null, deviceScaleFactor: undefined, isMobile: undefined });
  await displayPage.setContent('<html><body style="margin:0;background:#f8f5fc;font:40px system-ui;height:100vh;display:grid;place-content:center;text-align:center"><h1>CAUSEWAYBAY CAPTURE TEST</h1><p style="font-size:80px">What is 7 + 5?</p><button onclick="document.documentElement.requestFullscreen()">Start capture test</button></body></html>');
  await displayPage.getByRole('button', { name: 'Start capture test' }).click();
  await displayPage.bringToFront();
  await displayPage.waitForFunction(() => document.fullscreenElement !== null);
  // macOS switches Spaces asynchronously after the DOM fullscreen event. A
  // browser screenshot can already show this page while screencapture still
  // sees the previous desktop, so let the native compositor finish switching.
  await displayPage.waitForTimeout(1500);
});
test.afterAll(async () => { await displayBrowser?.close(); await stop(client); await stop(server); });

for (const mode of ['Text', 'Image']) {
  test(`real screen → Rust CLI → screenshot preview → ${mode} processing → Grok → disk and SQLite`, async ({ page, request }) => {
    const events = [];
    let memoId, target;
    await observeEvents(page,(event,id)=>{ events.push(event); memoId=id; });
    page.on('websocket', ws => {
      ws.on('framesent', frame => { const v=JSON.parse(frame.payload); if (v.path==='/api/think') target=v.body.client_id; });

    });
    await page.goto(base + '/overview');
    await expect(page.getByRole('combobox', { name: 'Capture client' })).toContainText('Native screen client');
    await page.getByRole('button', { name: `${mode} mode`, exact: true }).click();
    await page.getByRole('textbox', { name: 'Ask a question' }).fill('Answer the arithmetic question on the captured screen.');
    await displayPage.bringToFront();
    await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
    await expect(page.getByTestId('capture-preview').locator('img')).toBeVisible({ timeout: 30000 });
    expect(await page.getByTestId('capture-preview').locator('img').evaluate(img => img.complete && img.naturalWidth > 500)).toBeTruthy();
    if (mode === 'Text') {
      await expect.poll(() => events.find(e => e.kind === 'ocr_text')?.text ?? '', { timeout: 30000 }).toContain('7 + 5');
    }
    await expect(page.locator('.answer-content')).toContainText('12', { timeout: 150000 });
    await expect(page.locator('.save-status')).toHaveText('Saved to your library', { timeout: 150000 });
    expect(target).toBeTruthy();
    expect(events.some(e => e.kind === 'phase' && e.text === (mode === 'Text' ? 'ocr' : 'image'))).toBeTruthy();
    expect(events.some(e => e.kind === 'phase' && e.text === 'ai')).toBeTruthy();
    expect(events.some(e => e.kind === 'ocr')).toBe(mode === 'Text');
    const memo = await (await request.get(`${base}/api/memos/${memoId}`)).json();
    expect(memo.has_image).toBe(true);
    expect(memo.status).toBe('complete');
    const image = await (await request.get(`${base}/api/memos/${memoId}/image`)).body();
    expect(fs.readFileSync(path.join(backendDir, 'data/screenshots', `${memoId}.png`)).equals(image)).toBeTruthy();
    const screenshots=fs.readdirSync(path.join(clientDir,'screenshots'));
    expect(screenshots.every(name => !name.startsWith('.') && name.endsWith('.png'))).toBeTruthy();
    expect(screenshots.some(name => fs.readFileSync(path.join(clientDir,'screenshots',name)).equals(image))).toBeTruthy();
    expect(fs.existsSync(path.join(clientDir, 'client.jsonl'))).toBe(true);
    expect(clientLog).toContain('Screenshot received by backend.');
    expect(clientLog).toContain('SSE callbacks connected.');
    expect(clientLog).not.toContain('No such file or directory');
    await page.screenshot({ path: `test-results/native-${mode.toLowerCase()}-capture.png`, fullPage: true });
  });
}

test('capture failures surface the client error without claiming delivery', async ({ page }) => {
  await page.goto(base + '/overview');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('spinbutton', { name: 'Display number' }).fill('16');
  await page.getByRole('button', { name: 'Save backend settings', exact: true }).click();
  await expect(page.getByRole('status').filter({hasText:'Backend settings saved'})).toHaveText('Backend settings saved');
  const settings=fs.readFileSync(path.join(backendDir,'settings.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
  expect(settings.at(-1).config.display).toBe(16);
  await page.getByRole('button', { name: 'Close dialog' }).click();
  const delivered=(clientLog.match(/Screenshot received by backend/g)||[]).length;
  await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('Display 16 is unavailable on the capture Mac', { timeout:30000 });
  await expect(page.getByTestId('capture-preview')).toHaveCount(0);
  expect((clientLog.match(/Screenshot received by backend/g)||[]).length).toBe(delivered);
  expect(clientLog).toContain('Capture failed:');
});
