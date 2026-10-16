import {expectProcessingMode, openConversationOptions, openProcessingSettings} from './workspace-controls.mjs';
import {isolatedBackend} from './invisible-fixture.mjs';
import { test, expect } from '@playwright/test';

isolatedBackend(test,3155);
const defaults = {
  local_only: false, apple_enabled: false, mlx_enabled: false,
  mlx_url: 'http://127.0.0.1:8080/v1', speech_provider: 'auto',
  speech_locale: 'en-US', mlx_speech_model: '',
};
const defaultAi = {mode:'cloudai', local_provider:'apple', cloud_provider:'grok'};

test.beforeEach(async ({ request }) => {
  const r = await request.patch('/api/settings', { data: { provider: 'grok', model: 'grok-4', local_ai: defaults, default_ai:defaultAi } });
  expect(r.ok()).toBeTruthy();
  await request.patch('/api/ai/preferences',{data:{mode:'cloudai',local_provider:'apple',cloud_provider:'grok'}});
});
test.afterEach(async ({ request }) => {
  await request.patch('/api/settings', { data: { provider: 'grok', local_ai: defaults, default_ai:defaultAi } });
});

async function setup(page) {
  await page.goto('/overview');
  const settingsButton = page.getByRole('button', { name: 'Open settings', exact: true });
  if (await settingsButton.isVisible()) await settingsButton.click();
  else { await page.getByRole('button', {name:'Open navigation',exact:true}).click(); await page.keyboard.press('Escape'); await page.getByRole('button', {name:'Open settings',exact:true}).click(); }
  await expect(page.getByRole('region', { name: 'Local AI setup' })).toBeVisible();
}

test('admin enables MLX local, saves a model ID, and keeps settings after reload', async ({ page, request }) => {
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  await setup(page);
  await page.getByRole('checkbox', { name: 'Enable MLX local', exact: true }).check();
  await page.getByRole('button', { name: /MLX local/ }).click();
  await page.getByRole('textbox', { name: 'Model', exact: true }).fill('mlx-community/Qwen3.5-9B-4bit');
  await page.getByRole('checkbox', { name: 'Local-only mode', exact: true }).check();
  await page.getByLabel('Speech-to-text provider').selectOption('apple');
  await page.getByLabel('Speech language', { exact: true }).fill('ko-KR');
  await page.getByRole('button', { name: 'Save backend settings', exact: true }).click();
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).local_ai.local_only).toBe(true);
  await page.getByRole('button', {name:'Close dialog'}).click();
  const settings = await (await request.get('/api/settings')).json();
  expect(settings.provider).toBe('mlx');
  expect(settings.local_ai).toMatchObject({ local_only: true, mlx_enabled: true, speech_provider: 'apple', speech_locale: 'ko-KR' });
  await page.reload();
  const settingsButton = page.getByRole('button', { name: 'Open settings', exact: true });
  if (await settingsButton.isVisible()) await settingsButton.click();
  else { await page.getByRole('button', {name:'Open navigation',exact:true}).click(); await page.keyboard.press('Escape'); await page.getByRole('button', {name:'Open settings',exact:true}).click(); }
  await expect(page.getByRole('checkbox', { name: 'Local-only mode', exact: true })).toBeChecked();
  await expect(page.getByLabel('Speech language', { exact: true })).toHaveValue('ko-KR');
  await page.getByRole('button', { name: /MLX local/ }).click();
  await expect(page.getByRole('textbox', { name: 'Model', exact: true })).toHaveValue('mlx-community/Qwen3.5-9B-4bit');
  expect(errors).toEqual([]);
  await page.screenshot({ path: `test-results/local-ai-${test.info().project.name}.png`, fullPage: true });
});

test('remote MLX endpoint is rejected and not persisted', async ({ page, request }) => {
  await setup(page);
  // The URL field belongs to the service engine; the built-in worker needs none.
  await expect(page.getByLabel('MLX server URL', { exact: true })).toHaveCount(0);
  await page.getByLabel('MLX engine', { exact: true }).selectOption('service');
  await page.getByLabel('MLX server URL', { exact: true }).fill('https://external.example/v1');
  await page.getByRole('button', { name: 'Save backend settings', exact: true }).click();
  await expect(page.getByRole('dialog').getByRole('alert')).toContainText('MLX must use');
  expect((await (await request.get('/api/settings')).json()).local_ai.mlx_url).toBe(defaults.mlx_url);
});

test('Apple local can answer a demo question through the normal capture frontend', async ({ page, request }) => {
  await request.patch('/api/settings', { data: { provider: 'apple', model: 'system', default_ai:{...defaultAi, mode:'ondevice'}, local_ai: { ...defaults, apple_enabled: true, local_only: true } } });
  await request.patch('/api/ai/preferences',{data:{mode:'ondevice',local_provider:'apple',cloud_provider:'grok'}});
  await page.goto('/overview');
  await expect(page.locator('.composer-provider .provider-symbol')).toHaveClass(/apple/);
  await page.getByRole('textbox', { name: 'Ask a question', exact: true }).fill('Explain the text in my private note.');
  await page.getByRole('button', { name: 'Send question', exact: true }).click();
  await expect(page.locator('.answer-content')).toContainText('private note');
  const memos = await (await request.get('/api/memos')).json();
  expect(JSON.stringify(memos)).toContain('apple');
  // Even an explicitly forged cloud request must fail on the server.
  const denied = await request.post('/api/think', { data: { provider: 'grok', prompt: 'Never send this outside' } });
  expect(denied.status()).toBe(400);
  expect((await denied.json()).error).toMatch(/local processing|Local-only/);
});

test('local settings fit the screen and expose no credential entry', async ({ page }) => {
  await setup(page);
  await expect(page.getByLabel('API key', { exact: true })).toHaveCount(0);
  await page.getByLabel('Speech-to-text provider').selectOption('mlx');
  await page.getByLabel('MLX speech model directory').fill('/models/whisper');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await expect(page.getByRole('button', { name: 'Save backend settings', exact: true })).toBeVisible();
});

test('large admin mode cards save a workspace default and preserve personal choice', async ({page, request}, info) => {
  await setup(page);
  const group = page.getByRole('group', {name:'Default AI processing mode'});
  const card = name => group.getByRole('button', {name:new RegExp('^'+name)});
  await expect(group.getByRole('button')).toHaveCount(5);
  await expect(card('Cloud AI')).toHaveAttribute('aria-pressed', 'true');
  for (const label of ['On-device', 'Hybrid', 'Cloud AI', 'Codex', 'Claude Code']) {
    const bounds = await card(label).boundingBox();
    expect(bounds.height).toBeGreaterThanOrEqual(88);
    expect(bounds.width).toBeGreaterThanOrEqual(130);
  }
  await card('Hybrid').click();
  await expect(card('Hybrid')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.admin-mode-detail')).toContainText('when the local model asks for help');
  expect((await (await request.get('/api/settings')).json()).default_ai.mode).toBe('cloudai');
  await page.getByLabel('Cloud provider for approved help').selectOption('openai');
  await page.getByRole('button', {name:'Save backend settings', exact:true}).click();
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).default_ai).toEqual({...defaultAi, mode:'hybrid', cloud_provider:'openai'});
  expect((await (await request.get('/api/ai/preferences')).json()).mode).toBe('cloudai');
  await page.getByRole('button', {name:'Close dialog'}).click();
  await page.reload(); await setup(page);
  await expect(card('Hybrid')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByLabel('Cloud provider for approved help')).toHaveValue('openai');
  await card('On-device').focus(); await page.keyboard.press('Space');
  await expect(card('On-device')).toBeFocused();
  await expect(card('On-device')).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', {name:'Save backend settings', exact:true}).click();
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).default_ai.mode).toBe('ondevice');
  await card('Cloud AI').click();
  await page.getByRole('button', {name:'Save backend settings', exact:true}).click();
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).default_ai.mode).toBe('cloudai');
  await page.getByLabel('Local-only mode', {exact:true}).check();
  await expect(card('On-device')).toHaveAttribute('aria-pressed', 'true');
  await expect(card('Hybrid')).toBeDisabled();
  await expect(card('Cloud AI')).toBeDisabled();
  await page.getByLabel('Local-only mode', {exact:true}).uncheck();
  for (const toggle of await page.locator('.runtime-toggle').all()) {
    expect((await toggle.boundingBox()).height).toBeGreaterThanOrEqual(44);
  }
  for (const theme of ['light', 'dark']) {
    if (await page.locator('html').getAttribute('data-theme') !== theme) {
      await page.getByRole('button', {name:'Close dialog'}).click();
      await page.getByTestId('theme-toggle').click(); await setup(page);
    }
    await page.getByRole('dialog').evaluate(el => { el.scrollTop = 0; });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
    await page.screenshot({path:`test-results/admin-modes-${info.project.name}-${theme}.png`});
  }
});

test('editing a cloud provider keeps the selected Hybrid default', async ({page, request}) => {
  await setup(page);
  const modes = page.getByRole('group', {name:'Default AI processing mode'});
  await modes.getByRole('button', {name:/^Hybrid/}).click();
  await page.locator('.provider-grid').getByRole('button', {name:/OpenAI/}).click();
  await expect(modes.getByRole('button', {name:/^Hybrid/})).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('textbox', {name:'Model', exact:true}).fill('hybrid-cloud-model');
  await page.getByRole('button', {name:'Save backend settings', exact:true}).click();
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).default_ai)
    .toEqual({...defaultAi, mode:'hybrid', cloud_provider:'openai'});
  const settings = await (await request.get('/api/settings')).json();
  expect(settings.providers.find(p => p.id === 'openai').model).toBe('hybrid-cloud-model');
  await page.getByRole('button', {name:'Close dialog'}).click();
  await setup(page);
  await expect(modes.getByRole('button', {name:/^Hybrid/})).toHaveAttribute('aria-pressed', 'true');
});

test('local model follows admin updates in an open browser and ignores legacy model caches', async ({page, request}) => {
  const policy = {...defaultAi, mode:'ondevice', local_provider:'mlx'};
  const local = {...defaults, mlx_enabled:true};
  expect((await request.patch('/api/settings', {data:{provider:'mlx', model:'installed-first', default_ai:policy, local_ai:local}})).ok()).toBeTruthy();
  expect((await request.patch('/api/ai/preferences', {data:policy})).ok()).toBeTruthy();
  await page.addInitScript(() => {
    localStorage.setItem('causewaybay.settings.v1', JSON.stringify({provider:'mlx',models:{mlx:'obsolete-main-cache'}}));
    localStorage.setItem('causewaybay.chat.local', JSON.stringify({provider:'mlx',models:{mlx:'obsolete-chat-cache'}}));
  });
  await page.goto('/');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  // An MLX pick offers the backend's model catalogue as a select (no free-text
  // model field): the admin's installed model is the checked entry, and the
  // obsolete browser caches never appear in it.
  const model = page.getByLabel('Chat On-device model', {exact: true});
  await expect(model).toHaveValue('installed-first');
  await expect(page.getByLabel('Chat AI model', {exact: true})).toHaveCount(0);
  await expect(model.locator('option:checked')).toHaveText('installed-first');
  expect((await model.locator('option').allTextContents()).join('\n')).not.toMatch(/obsolete/);
  expect((await request.patch('/api/settings', {data:{provider:'mlx',model:'installed-next'}})).ok()).toBeTruthy();
  // Keep the browser open: its periodic settings refresh must replace the local model.
  await expect(model).toHaveValue('installed-next', {timeout:25000});
  await page.getByRole('textbox', {name:'Chat message', exact:true}).fill('Explain my local note');
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  await expect(page.locator('.from-ai').last()).toContainText('local note');
  // A visible streamed answer can precede the backend's completion event.
  await expect.poll(async () => {
    const {memos} = await (await request.get('/api/memos')).json();
    const memo = memos.find(m => m.input === 'Explain my local note');
    return memo && {provider:memo.provider, model:memo.model, status:memo.status};
  }).toEqual({provider:'mlx', model:'installed-next', status:'complete'});
  await page.reload();
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await expect(model).toHaveValue('installed-next');
  // The admin model editor must also ignore the obsolete overview cache.
  await setup(page);
  await expect(page.getByRole('textbox', {name:'Model', exact:true})).toHaveValue('installed-next');
});

test('Local-only displays the effective on-device policy and restores the saved cloud choice', async ({page, request}) => {
  const personal = {...defaultAi, cloud_provider:'openai'};
  expect((await request.patch('/api/ai/preferences', {data:personal})).ok()).toBeTruthy();
  await page.goto('/');
  await expectProcessingMode(page, 'Cloud AI');
  let modes = await openProcessingSettings(page);
  await expect(modes.getByRole('button', {name:/^Cloud AI/})).toHaveAttribute('aria-pressed', 'true');
  const local = {...defaults, local_only:true, mlx_enabled:true};
  expect((await request.patch('/api/settings', {data:{provider:'mlx',local_ai:local,default_ai:{...defaultAi,mode:'ondevice',local_provider:'mlx'}}})).ok()).toBeTruthy();
  await expect(modes.getByRole('button', {name:/^On-device/})).toHaveAttribute('aria-pressed', 'true', {timeout:25000});
  await expect(modes.getByRole('button', {name:/^Cloud AI/})).toHaveAttribute('aria-pressed', 'false');
  await expect(modes.getByRole('button', {name:/^Cloud AI/})).toBeDisabled();
  await expect(modes.getByRole('button', {name:/^Hybrid/})).toBeDisabled();
  await page.goBack();
  await expectProcessingMode(page, 'On-device');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await expect(page.getByLabel('Chat AI provider')).toHaveValue('mlx');
  expect((await request.post('/api/think', {data:{provider:'openai',prompt:'Private'}})).status()).toBe(400);
  expect((await request.patch('/api/settings', {data:{local_ai:{...local,local_only:false}}})).ok()).toBeTruthy();
  modes = await openProcessingSettings(page);
  await expect(modes.getByRole('button', {name:/^Cloud AI/})).toHaveAttribute('aria-pressed', 'true', {timeout:25000});
  await expect(modes.getByRole('button', {name:/^Cloud AI/})).toBeEnabled();
  expect(await (await request.get('/api/ai/preferences')).json()).toEqual(personal);
  await page.goBack();
  await expectProcessingMode(page, 'Cloud AI');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await expect(page.getByLabel('Chat AI provider')).toHaveValue('openai');
});
