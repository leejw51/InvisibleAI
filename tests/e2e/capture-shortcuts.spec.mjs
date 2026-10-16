// Capture in a hurry: device and AI are one tap away on the hero, the device
// choice follows the account, and Capture brings the answer into view by itself.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Hero geometry is asserted at the design's desktop size, not Playwright's 1280×720 default.
test.use({ viewport: { width: 1440, height: 900 } });
const captureButton = page => page.locator('.chat-compose-actions button[aria-label="Capture"]');
async function stop(child) {
  if (child && child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit'); child.kill('SIGTERM'); await exited;
  }
}
// Each run enrolls new ids: a registered id stays bound to its device key.
const run = Date.now().toString(36);
const id = name => `shortcut-${run}-${name}`;
function startClient(dir, name) {
  return spawn('./target/debug/rust-cli', ['client', '--backend', 'http://127.0.0.1:3099', '--name', name, '--client-id', id(name)],
    { env: { ...process.env, APP_TOKEN: '', CAUSEWAYBAY_CLIENT_CONFIG_DIR: path.join(dir, name) }, stdio: 'ignore' });
}

test('device and AI shortcuts sit on the hero and the device choice is kept on the account', async ({ page, browser, request }) => {
  test.setTimeout(90000);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cw-shortcuts-'));
  const children = [startClient(dir, 'Desk'), startClient(dir, 'Laptop')];
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  let fresh;
  try {
    await expect.poll(async () => (await (await request.get('/api/clients')).json()).filter(c => c.id.startsWith(`shortcut-${run}-`) && c.online).length).toBe(2);
    await request.patch('/api/capture/choices', { data: { active: '', default: '', continuous: [] } });
    // The hero geometry below is the fresh-page layout, not the compact one a conversation gets.
    await request.post('/api/rooms/thought', { data: { fresh: true } });
    await page.goto('/');
    const shortcuts = page.getByRole('group', { name: 'Capture shortcuts' });
    const device = shortcuts.getByRole('combobox', { name: 'Active capture device' });
    const ai = shortcuts.getByTestId('quick-ai-select');
    // Both are visible without opening any settings; the device lives in one place only.
    await expect(device).toBeVisible();
    await expect(ai).toBeVisible();
    await expect(page.locator('.everyday-workspace')).not.toHaveAttribute('open', /.*/);
    await expect(page.locator('#active-capture-device')).toHaveCount(1);
    await expect(page.locator('.everyday-workspace #active-capture-device')).toHaveCount(0);
    await expect(device).toHaveValue(id('Desk'));
    // The hero shortcut sits beside the companion, inside the intro card.
    const intro = await page.locator('.chat-page > .page-intro').boundingBox();
    const pick = await device.boundingBox();
    expect(pick.y).toBeGreaterThanOrEqual(intro.y);
    expect(pick.x + pick.width / 2).toBeGreaterThan(intro.x + intro.width / 2);
    expect(intro.height).toBeLessThanOrEqual(300);
    const companion = await page.locator('.page-intro invisible-companion').boundingBox();
    expect(companion.y).toBeGreaterThanOrEqual(pick.y + pick.height);

    await device.selectOption(id('Laptop'));
    await expect.poll(async () => (await (await request.get('/api/capture/choices')).json())?.active).toBe(id('Laptop'));
    expect(await request.patch('/api/capture/choices', { data: { active: 'bad id; drop', default: '', continuous: [] } }).then(r => r.ok())).toBe(false);

    // A new browser with no stored choice starts with the account's device.
    fresh = await browser.newContext();
    const other = await fresh.newPage();
    await other.goto('/');
    await expect(other.getByRole('combobox', { name: 'Active capture device' })).toHaveValue(id('Laptop'));
    expect(await other.evaluate(() => JSON.parse(localStorage.getItem('causewaybay.capture.v1:local')).active)).toBe(id('Laptop'));

    // The AI shortcut saves to the account policy like the full picker does.
    const before = await (await request.get('/api/ai/preferences')).json();
    const current = await ai.inputValue();
    const options = await ai.locator('option:not([disabled])').evaluateAll(o => o.map(e => e.value));
    const target = options.find(v => v !== current && !['apple', 'mlx', 'codex', 'claude_code'].includes(v));
    expect(target).toBeTruthy();
    await ai.selectOption(target);
    await expect.poll(async () => (await (await request.get('/api/ai/preferences')).json()).cloud_provider).toBe(target);
    await expect(ai).toHaveValue(target);
    await request.patch('/api/ai/preferences', { data: before });
    await expect(ai).toHaveValue(current);

    await page.setViewportSize({ width: 390, height: 844 });
    await expect(device).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    await fresh?.close();
    await Promise.all(children.map(stop));
    await request.patch('/api/capture/choices', { data: { active: '', default: '', continuous: [] } });
  }
});

test('one press: Capture is the primary action and Enter on an empty box captures', async ({ page, request }) => {
  await request.patch('/api/capture/choices', { data: { active: '', default: '', continuous: [] } });
  await request.post('/api/rooms/thought', { data: { fresh: true } });
  await page.goto('/');
  const capture = captureButton(page);
  await expect(capture).toHaveClass(/primary-button/);
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).not.toHaveClass(/primary-button/);
  await expect(page.locator('#capture-action-hint')).toContainText('Enter on an empty message captures too.');
  const draft = page.getByRole('textbox', { name: 'Chat message', exact: true });
  await expect(capture).toBeEnabled();
  // Empty box + Enter = Capture: the post is a screen capture, and the answer follows.
  await draft.press('Enter');
  await expect(page.locator('.from-user').last()).toContainText('Screen capture');
  await expect(page.locator('.from-ai').last()).toContainText('returns 5');
  // With text in the box, Enter still sends the text instead.
  await draft.fill('Just a question');
  await draft.press('Enter');
  await expect(page.locator('.from-user').last()).toContainText('Just a question');
  await expect(page.locator('.from-user').last()).not.toContainText('Screen capture');
  // Off the Capture page, Enter on an empty box does nothing and Send stays primary.
  await page.goto('/chat');
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toHaveClass(/primary-button/);
  const count = await page.locator('.from-user').count();
  await page.getByRole('textbox', { name: 'Chat message', exact: true }).press('Enter');
  await page.waitForTimeout(800);
  expect(await page.locator('.from-user').count()).toBe(count);
});

test.describe('capture shine', () => {
  test.use({ reducedMotion: 'no-preference' });
  test('the ready Capture button shines and sparkles on press; disabled or reduced motion stays still', async ({ page, request }) => {
    await request.patch('/api/capture/choices', { data: { active: '', default: '', continuous: [] } });
    await request.post('/api/rooms/thought', { data: { fresh: true } });
    await page.goto('/');
    const capture = captureButton(page);
    await expect(capture).toBeEnabled();
    // The sweep moves the highlight across the button within a few seconds.
    await expect.poll(() => capture.evaluate(el => el.style.getPropertyValue('--shine-x')), { timeout: 6000, intervals: [50] }).not.toMatch(/^(-140%)?$/);
    const sparks = page.locator('.capture-spark');
    await capture.click();
    await expect(page.locator('.capture-ring')).toHaveCount(1);
    expect(await sparks.count()).toBeGreaterThan(4);
    // Particles clean themselves up; nothing is left in the page.
    await expect(sparks).toHaveCount(0, { timeout: 3000 });
    await expect(page.locator('.capture-ring')).toHaveCount(0);
    await expect(page.locator('.from-ai').last()).toContainText('returns 5');
    // The shine never adds children to the button Yew renders.
    expect(await capture.evaluate(el => [...el.children].map(c => c.tagName.toLowerCase()))).toEqual(['svg', 'span']);
  });
  test('reduced motion: no sweep and no sparkles', async ({ page, request }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await request.patch('/api/capture/choices', { data: { active: '', default: '', continuous: [] } });
    await request.post('/api/rooms/thought', { data: { fresh: true } });
    await page.goto('/');
    const capture = captureButton(page);
    await expect(capture).toBeEnabled();
    await page.waitForTimeout(3500);
    expect(await capture.evaluate(el => el.style.getPropertyValue('--shine-x'))).toBe('-140%');
    await capture.click();
    await page.waitForTimeout(100);
    await expect(page.locator('.capture-spark, .capture-ring')).toHaveCount(0);
  });
});

test.describe('auto focus', () => {
  test.use({ reducedMotion: 'no-preference', viewport: { width: 1440, height: 900 } });
  test('Capture eases the screenshot and then the answer into view', async ({ page, request }) => {
    await request.patch('/api/capture/choices', { data: { active: '', default: '', continuous: [] } });
    await request.post('/api/rooms/thought', { data: { fresh: true } });
    await page.goto('/');
    const capture = captureButton(page);
    await expect(capture).toBeEnabled();
    expect(await page.evaluate(() => scrollY)).toBe(0);
    // Demo replies land a moment after the post, so record which messages
    // glowed, in order, rather than hoping to catch the first one.
    await page.evaluate(() => {
      window.__glowed = [];
      new MutationObserver(records => { for (const r of records) { const el = r.target; if (el.classList?.contains('is-focused') && !window.__glowed.includes(el.className)) window.__glowed.push(el.className); } })
        .observe(document.body, { subtree: true, attributes: true, attributeFilter: ['class'] });
    });
    await capture.click();
    const reply = page.locator('.from-ai').last();
    await expect(reply).toContainText('returns 5');
    await expect(reply).toHaveClass(/is-focused/);
    expect(await page.evaluate(() => window.__glowed.map(c => c.includes('from-user') ? 'post' : 'reply'))).toEqual(['post', 'reply']);
    // The reply settles below the header and above the composer, with no help.
    await expect.poll(async () => {
      const r = await reply.boundingBox(); const dock = await page.locator('.chat-dock').boundingBox();
      return r && dock && r.y >= 60 && r.y < dock.y - 120 ? 'in view' : `top ${r?.y}`;
    }, { timeout: 10000 }).toBe('in view');
    const list = page.locator('.chat-messages');
    expect(await list.evaluate(el => el.style.scrollBehavior)).toBe('');
    expect(await page.evaluate(() => scrollY)).toBeGreaterThan(0);
    // The glow fades out on its own.
    await expect(reply).not.toHaveClass(/is-focused/, { timeout: 5000 });
    await expect(page.locator('.is-focused')).toHaveCount(0);
  });
});
