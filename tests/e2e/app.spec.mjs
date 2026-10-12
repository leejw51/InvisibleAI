import {openConversationOptions} from './workspace-controls.mjs';
import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/overview');
  await expect(page.getByRole('heading', { name: 'A little clarity. A lot less effort.' })).toBeVisible();
});

// The Library sits behind the "More tools" fold of the menu.
async function openLibrary(page) {
  const item = page.locator('[data-nav="library"]');
  if (!(await item.isVisible())) await page.locator('.more-tools > summary').click();
  await item.click();
}

test('polished home, asset, modes, and responsive layout', async ({ page }) => {
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await expect(page.getByRole('img', { name: /miniature Hong Kong/ })).toBeVisible();
  expect(await page.locator('.hero img').evaluate(img => img.complete && img.naturalWidth > 100)).toBeTruthy();
  await page.getByRole('button', { name: 'Image mode', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Image mode', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByText('Send the screenshot directly to AI')).toBeVisible();
  await page.reload();
  await expect(page.getByRole('button', { name: 'Image mode', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Text mode', exact: true }).click();
  await expect(page.getByText('Extract text with Apple Vision, then ask AI')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Text mode', exact: true })).toHaveClass('selected');
  await expect(page.getByRole('button', { name: 'Image mode', exact: true })).not.toHaveClass('selected');
  await page.screenshot({ path: 'test-results/home-desktop.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole('button', { name: 'Capture', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await expect(page.getByRole('button', { name: 'Wiki', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close navigation' }).click();
  await page.screenshot({ path: 'test-results/home-mobile.png', fullPage: true });
  expect(errors).toEqual([]);
});

test('Think streams an OCR fixture, saves it, and exports Markdown', async ({ page }) => {
  // Exercise the same clipboard fallback used on an HTTP VPN hostname.
  await page.evaluate(() => Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true }));
  await page.getByRole('button', { name: 'Text mode', exact: true }).click();
  await page.getByRole('button', { name: 'Capture', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText('Change - to +');
  await expect(page.getByRole('button', { name: 'Copy answer' })).toBeVisible();
  await expect(page.locator('.save-status')).toHaveText('Saved to your library');
  await page.getByRole('button', { name: 'Copy answer' }).click();
  await expect(page.getByRole('status').filter({hasText:'Answer copied'})).toHaveText('Answer copied');
  await page.locator('.source-details > summary').click();
  await expect(page.locator('.source-details pre')).toContainText('fn add');
  const downloadEvent = page.waitForEvent('download');
  await page.getByRole('link', { name: 'Download memo', exact: true }).click();
  expect((await downloadEvent).suggestedFilename()).toBe('causewaybay-memo.md');
  await page.getByRole('button', { name: 'Pin memo', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Unpin memo', exact: true })).toBeVisible();
  await page.screenshot({ path: 'test-results/answer.png', fullPage: true });
});

test('question and draft persistence use local storage and JSON WebSockets', async ({ page }) => {
  const question = `How can I focus better? #focus-${Date.now()}`;
  await page.getByRole('textbox', { name: 'Ask a question' }).fill(question);
  await expect.poll(async () => page.evaluate(() => JSON.parse(localStorage.getItem('causewaybay.settings.v1') || '{}').draft)).toBe(question);
  await page.reload();
  await expect(page.getByRole('textbox', { name: 'Ask a question' })).toHaveValue(question);
  await page.getByRole('button', { name: 'Send question', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText(question);
  await expect(page.getByRole('button', { name: 'Copy answer' })).toBeVisible();
  await openLibrary(page);
  await page.getByRole('textbox', { name: 'Search memos' }).fill(question.split(' ').at(-1));
  await expect(page.locator('.memo-card')).toHaveCount(1);
});

test('create, search by hashtag and BM25, reopen, pin, and delete memo', async ({ page }) => {
  const tag = `test${Date.now()}`;
  await openLibrary(page);
  await page.getByRole('button', { name: 'New memo', exact: true }).click();
  await page.getByRole('textbox', { name: 'Memo title' }).fill('Compiler notes');
  await page.getByRole('textbox', { name: 'Memo content' }).fill(`Ownership and borrowing make Rust reliable. #${tag}`);
  await page.getByRole('button', { name: 'Save memo', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText('Ownership and borrowing');
  await openLibrary(page);
  await page.getByRole('textbox', { name: 'Search memos' }).fill(`#${tag} borrowing`);
  await expect(page.locator('.memo-card')).toHaveCount(1);
  await page.getByRole('heading', { name: 'Compiler notes', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText('Ownership');
  await page.getByRole('button', { name: 'Delete memo', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm delete', exact: true }).click();
  await openLibrary(page);
  await page.getByRole('textbox', { name: 'Search memos' }).fill(`#${tag}`);
  await expect(page.getByRole('heading', { name: 'No thoughts found' })).toBeVisible();
});

test('browser stores preferences, removes legacy keys, and has no API-key setup', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('causewaybay.settings.v1', JSON.stringify({ keys: { openai: 'legacy-browser-test-key' }, draft: 'My retained draft' })));
  const sent = [];
  page.on('websocket', ws => ws.on('framesent', frame => sent.push(String(frame.payload))));
  await page.reload();
  await expect(page.getByRole('textbox', { name: 'Ask a question' })).toHaveValue('My retained draft');
  const migrated = await page.evaluate(() => JSON.parse(localStorage.getItem('causewaybay.settings.v1')));
  expect(migrated).not.toHaveProperty('keys');
  expect(migrated.draft).toBe('My retained draft');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: /OpenAI/ }).click();
  await expect(page.getByLabel('API key', { exact: true })).toHaveCount(0);
  await expect(page.getByText('AI is configured on your backend. No API key is needed here.')).toBeVisible();
  await page.getByRole('textbox', { name: 'Model', exact: true }).fill('gpt-4.1-mini');
  await page.getByRole('button', { name: 'Save backend settings', exact: true }).click();
  await expect(page.getByRole('status').filter({hasText:'Backend settings saved'})).toHaveText('Backend settings saved');
  const response = await page.request.get('/api/settings');
  expect(await response.text()).not.toContain('legacy-browser-test-key');
  expect(await page.evaluate(() => localStorage.getItem('causewaybay.settings.v1'))).not.toContain('legacy-browser-test-key');
  await page.screenshot({ path: 'test-results/settings.png', fullPage: true });
  await page.getByRole('button', { name: 'Close dialog' }).click();
  await page.reload();
  // The composer shows the backend provider; the only selector lives in chat's AI & tools.
  await expect(page.locator('.composer-provider')).toContainText('OpenAI');
  await expect(page.getByRole('combobox', { name: 'AI provider' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Send question', exact: true }).click();
  await expect(page.locator('.save-status')).toHaveText('Saved to your library');
  const thinkFrame = sent.map(v => JSON.parse(v)).find(v => v.path === '/api/think');
  expect(thinkFrame).toBeTruthy();
  expect(thinkFrame.body).not.toHaveProperty('api_key');
  expect(sent.join('')).not.toContain('legacy-browser-test-key');
});

test('upload, preview, compile, and download a real Rust file', async ({ page }) => {
  await page.locator('input[type=file]').setInputFiles({ name: 'hello.rs', mimeType: 'text/plain', buffer: Buffer.from('pub fn greet() -> &\'static str { "hello" }') });
  await expect(page.locator('.attachment')).toContainText('hello.rs');
  if (!(await page.locator('[data-nav="files"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.getByRole('button', { name: /^Files/ }).click();
  await page.getByRole('heading', { name: 'hello.rs', exact: true }).click();
  await expect(page.locator('.file-code')).toContainText('pub fn greet');
  await page.getByRole('button', { name: 'Compile Rust' }).click();
  await expect(page.locator('.compile-output')).toContainText('Compilation succeeded');
  await page.getByRole('button', { name: 'Close dialog' }).click();
  await page.getByRole('heading', { name: 'hello.rs', exact: true }).click();
  await expect(page.locator('.compile-output')).toContainText('Compilation succeeded');
  const download = page.waitForEvent('download');
  await page.getByRole('dialog').getByRole('link', { name: 'Download', exact: true }).click();
  expect((await download).suggestedFilename()).toBe('hello.rs');
});

test('coding agent saves a file, invokes compiler, and shows tool results', async ({ page }) => {
  // The agent is a chat setting: allow file creation & compilation, then ask.
  await page.goto('/');
  await expect(page.getByRole('combobox', { name: 'Chat room', exact: true })).not.toHaveValue('');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await page.getByRole('checkbox', { name: 'Allow file creation & compilation', exact: true }).check();
  await page.getByRole('textbox', { name: 'Chat message', exact: true }).fill('Write and compile a Rust addition function');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const reply = page.locator('.from-ai').last();
  await expect(reply).toContainText('Your Rust file is ready', { timeout: 30000 });
  await reply.getByRole('link', { name: 'Workspace activity', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText('Your Rust file is ready');
  await expect(page.getByRole('button', { name: 'Copy answer' })).toBeVisible();
  await expect(page.locator('.tool-activity')).toContainText('write file');
  await expect(page.locator('.tool-activity')).toContainText('compile code');
  await expect(page.locator('.tool-activity')).toContainText('Compilation succeeded');
  await expect(page.getByRole('link', { name: 'Download answer.rs' })).toBeVisible();
  await page.screenshot({ path: 'test-results/agent.png', fullPage: true });
});

test('untrusted memo Markdown cannot inject scripts or remote images', async ({ page }) => {
  const created = await (await page.request.post('/api/memos', { data: { title: 'Safe browser memo', content: '<script>window.xss=true</script>\n<img src="https://example.com/tracker" onerror="window.xss=true">\n[bad](javascript:alert(1))\n**Safe content**' } })).json();
  await openLibrary(page);
  await page.getByRole('textbox', { name: 'Search memos' }).fill('Safe content');
  await expect(page.getByRole('heading', { name: 'Safe browser memo', exact: true })).toBeVisible();
  await page.getByRole('heading', { name: 'Safe browser memo', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText('Safe content');
  await expect(page.locator('.answer-content script, .answer-content img, .answer-content a[href^="javascript:"]')).toHaveCount(0);
  expect(await page.evaluate(() => window.xss)).toBeUndefined();
  await page.request.delete(`/api/memos/${created.id}`);
});
