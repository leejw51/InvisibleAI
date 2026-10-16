import {openConversationOptions, openNav} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

async function openNavigation(page) {
  const menu = page.getByRole('button', {name: 'Open navigation', exact: true});
  if (await menu.isVisible() && await menu.getAttribute('aria-expanded') !== 'true') await menu.click();
}
async function navigate(page, destination) {
  // openNav opens the drawer and whichever fold holds the item, without toggling an open fold shut.
  await openNav(page, destination);
  await expect(page.locator('.sidebar-backdrop')).toHaveCount(0);
}
async function fits(page) {
  const width = page.viewportSize().width;
  const layout = await page.evaluate(() => ({
    content: document.documentElement.scrollWidth,
    wide: [...document.querySelectorAll('body *')].map(el => ({tag: el.tagName, class: el.className, right: el.getBoundingClientRect().right, width: el.getBoundingClientRect().width, scroll: el.scrollWidth, client: el.clientWidth})).filter(el => el.right > innerWidth + 1 || el.scroll > el.client + 20).slice(0, 12),
  }));
  expect(layout.content, JSON.stringify(layout)).toBeLessThanOrEqual(width + 1);
}
async function theme(page, mode) {
  if (await page.locator('html').getAttribute('data-theme') !== mode) await page.getByTestId('theme-toggle').click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', mode);
  await expect(page.getByTestId('theme-toggle')).toHaveText(mode === 'dark' ? 'Dark' : 'Light');
  expect(await page.locator('html').evaluate(el => getComputedStyle(el).colorScheme)).toBe(mode);
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute('content', mode === 'dark' ? '#18161e' : '#f8f7f4');
}
async function screenshot(page, info, name) {
  const file = `test-results/responsive-${info.project.name}-${name}.png`;
  await page.screenshot({path: file, fullPage: true});
  await info.attach(name, {path: file, contentType: 'image/png'});
}

test('light/dark toggle and main pages fit desktop, tablet, and phone', async ({page}, info) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByRole('textbox', {name: 'Chat message', exact: true})).toBeVisible();
  const toggle = page.getByTestId('theme-toggle');
  await expect(toggle).toBeInViewport();
  const bounds = await toggle.boundingBox();
  expect(bounds.width).toBeGreaterThanOrEqual(44);
  expect(bounds.height).toBeGreaterThanOrEqual(44);
  await openConversationOptions(page);
  await page.locator('.chat-options > summary').click();
  await expect(page.getByRole('button', {name: 'Check AI connection', exact: true})).toBeVisible();
  await openConversationOptions(page);
  await page.locator('.chat-options > summary').click();
  for (const mode of ['light', 'dark']) {
    await page.goto('/');
    await theme(page, mode);
    await fits(page);
    await screenshot(page, info, `chat-${mode}`);
    if (info.project.use.hasTouch) {
      expect(await page.getByRole('textbox', {name: 'Chat message', exact: true}).evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(16);
    }
    await navigate(page, 'dashboard');
    await expect(page.getByTestId('status-files')).toBeVisible();
    await expect(page.locator('cwb-status-diagram')).toHaveAttribute('data-renderer', /three|unavailable/);
    await fits(page);
    await screenshot(page, info, `dashboard-${mode}`);
    for (const destination of ['files', 'library', 'devices']) {
      await navigate(page, destination);
      await fits(page);
      await expect(toggle).toBeInViewport();
      await expect(page.locator('html')).toHaveAttribute('data-theme', mode);
    }
    await page.evaluate(() => scrollTo(0, document.body.scrollHeight));
    await expect(toggle).toBeInViewport();
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', mode);
  }
  expect(errors).toEqual([]);
});

test('rotation, narrow screens, and a shorter viewport keep chat and dialogs usable', async ({page}, info) => {
  await page.goto('/');
  await openConversationOptions(page);
  const room = page.getByRole('combobox', {name: 'Chat room', exact: true});
  await expect(room).not.toHaveValue('');
  const previous = await room.inputValue();
  await page.getByRole('button', {name: 'New chat', exact: true}).click();
  await openConversationOptions(page);
  await expect(room).not.toHaveValue(previous);
  const draft = page.getByRole('textbox', {name: 'Chat message', exact: true});
  await draft.fill('Responsive layout fixture');
  await page.getByRole('button', {name: 'Send message', exact: true}).click();
  const answer = page.locator('.from-ai').last();
  await expect(answer.getByRole('button', {name: 'Edit message', exact: true})).toBeVisible();
  await expect(page.getByRole('button', {name: 'Stop answer', exact: true})).toHaveCount(0);
  await answer.getByRole('button', {name: 'Edit message', exact: true}).click();
  await answer.getByRole('textbox', {name: 'Edit message', exact: true}).fill(`A long URL: https://example.test/${'abcdefghij'.repeat(60)}\n\n\`\`\`text\n${'wide-code-'.repeat(90)}\n\`\`\``);
  await answer.getByRole('button', {name: 'Save message', exact: true}).click();
  const initial = page.viewportSize();
  const sizes = !info.project.use.isMobile ? [initial, {width: 1024, height: 768}]
    : initial.width > 600 ? [initial, {width: 1024, height: 768}, {width: 768, height: 480}]
    : [initial, {width: 320, height: 568}, {width: 844, height: 390}, {width: 390, height: 420}];
  for (const size of sizes) {
    await page.setViewportSize(size);
    await theme(page, 'dark');
    await fits(page);
    // A populated conversation must have enough space to read, even with a short viewport.
    const messages = await page.locator('.chat-messages').boundingBox();
    expect(messages.height).toBeGreaterThanOrEqual(200);
    const composer = await page.locator('.chat-compose').boundingBox();
    expect(composer.y).toBeGreaterThanOrEqual(messages.y + messages.height - 1);
    await draft.fill('Keep my draft while changing orientation and theme');
    await theme(page, 'light');
    await expect(draft).toHaveValue('Keep my draft while changing orientation and theme');
    await expect(page.getByRole('button', {name: 'Send message', exact: true})).toBeEnabled();
    const toolbar = page.getByRole('group', {name:'Message toolbar', exact:true});
    const controls = toolbar.locator(':scope > *');
    await expect(controls).toHaveCount(3);
    await expect(controls.nth(0)).toHaveText('Capture');
    await expect(controls.nth(1)).toHaveAttribute('aria-label', 'Chat message');
    await expect(controls.nth(2)).toHaveText('Send message');
    for (const scroll of [0, 10000]) {
      await page.evaluate(y => window.scrollTo(0, y), scroll);
      for (const control of await controls.all()) {
        await expect(control).toBeInViewport({ratio:1});
        const box = await control.boundingBox();
        expect(box.height).toBeGreaterThanOrEqual(44);
        expect(box.width).toBeGreaterThanOrEqual(44);
      }
      const dock = await page.locator('.chat-dock').boundingBox();
      expect(Math.abs(dock.y + dock.height - size.height)).toBeLessThanOrEqual(1);
    }
    await openNavigation(page);
    if (await page.getByRole('button', {name: 'Close navigation', exact: true}).isVisible()) {
      await page.getByRole('button', {name: 'Close navigation', exact: true}).click();
      await expect(page.locator('.sidebar-backdrop')).toHaveCount(0);
    }
    await page.getByRole('button', {name: 'Open settings', exact: true}).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await fits(page);
    const bounds = await dialog.boundingBox();
    expect(bounds.height).toBeLessThanOrEqual(size.height);
    await dialog.getByRole('button', {name: 'Save backend settings', exact: true}).scrollIntoViewIfNeeded();
    await expect(dialog.getByRole('button', {name: 'Save backend settings', exact: true})).toBeInViewport();
    await dialog.getByRole('button', {name: 'Close dialog', exact: true}).click();
    await expect(draft).toHaveValue('Keep my draft while changing orientation and theme');
    await screenshot(page, info, `chat-${size.width}x${size.height}`);
  }
  if (initial.width < 600) {
    await page.setViewportSize({width: 320, height: 568});
    for (const language of ['en', 'ko', 'yue-HK', 'zh-CN', 'ja', 'es', 'cs']) {
      await page.getByTestId('language-select').selectOption(language);
      await fits(page);
      const current = await page.locator('html').getAttribute('data-theme');
      await page.getByTestId('theme-toggle').click();
      await expect(page.locator('html')).toHaveAttribute('data-theme', current === 'dark' ? 'light' : 'dark');
      await fits(page);
    }
  }
});

test('appearance toggle works before login and persists through login and logout', async ({page, request}, info) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-responsive-login-'));
  const base = 'http://127.0.0.1:3138';
  const server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: {...process.env, CWBBOT_ADMIN: '0x9858effd232b4033e47d90003d41ec34ecaeda94', BIND_ADDR: '127.0.0.1:3138',
      DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '',
      XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: ''}, stdio: 'ignore',
  });
  try {
    await expect.poll(async () => (await request.get(`${base}/api/health`).catch(() => null))?.status()).toBe(200);
    await page.goto(base);
    await expect(page.getByLabel('Recovery phrase')).toBeVisible();
    for (const mode of ['light', 'dark']) {
      await theme(page, mode);
      await fits(page);
      await expect(page.getByTestId('theme-toggle')).toBeInViewport();
      await screenshot(page, info, `login-${mode}`);
    }
    await page.getByLabel('Recovery phrase').fill('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about');
    await page.getByRole('button', {name: 'Sign in', exact: true}).click();
    await expect(page.getByRole('heading', {name: 'Hey, what’s on your mind?', exact: true})).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await expect(page.getByTestId('theme-toggle')).toBeInViewport();
    // Sign out sits in the main navigation, not under Settings.
    await openNavigation(page);
    await page.getByRole('navigation', {name: 'Main navigation'}).getByRole('button', {name: 'Sign out', exact: true}).click();
    // Signing out asks once more; Cancel keeps the session, Confirm ends it.
    const ask = page.getByRole('dialog', {name: 'Sign out?'});
    await ask.getByRole('button', {name: 'Cancel'}).click();
    await expect(ask).toHaveCount(0);
    await expect(page.getByLabel('Recovery phrase')).toHaveCount(0);
    await openNavigation(page);
    await page.getByRole('navigation', {name: 'Main navigation'}).getByRole('button', {name: 'Sign out', exact: true}).click();
    await page.getByRole('dialog', {name: 'Sign out?'}).getByRole('button', {name: 'Confirm sign out'}).click();
    await expect(page.getByLabel('Recovery phrase')).toBeVisible();
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await theme(page, 'light');
    if (info.project.use.hasTouch) expect(await page.getByLabel('Recovery phrase').evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(16);
  } finally {
    if (server.exitCode === null && server.signalCode === null) {
      const exited = once(server, 'exit'); server.kill('SIGTERM'); await exited;
    }
    fs.rmSync(dir, {recursive: true, force: true});
  }
});
