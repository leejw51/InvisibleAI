import {openConversationOptions, openWorkspace} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';

isolatedBackend(test, 3167);

// The chat AI selector is the one place to pick a provider. It is saved to the
// account, survives reloads, and follows the account's processing policy: an
// On-device or Hybrid policy pins chat to the local model, and the API refuses
// a cloud provider named by the browser.
test('chat provider choice is saved to the account and obeys local and Hybrid policies', async ({page, request}) => {
  const policy = {mode:'cloudai', local_provider:'mlx', cloud_provider:'grok'};
  expect((await request.patch('/api/settings', {data:{
    provider:'grok', default_ai:policy,
    local_ai:{local_only:false, apple_enabled:false, mlx_enabled:true},
  }})).ok()).toBeTruthy();
  expect((await request.patch('/api/ai/preferences', {data:policy})).ok()).toBeTruthy();
  const open = async () => {
    await page.goto('/');
    await openConversationOptions(page);
    await expect(page.getByRole('combobox', {name:'Chat room', exact:true})).not.toHaveValue('');
    await page.locator('.chat-options > summary').click();
  };
  await open();
  await expect(page.getByRole('combobox', {name:'AI provider', exact:true})).toHaveCount(0);
  const provider = page.getByTestId('chat-ai-select');
  await expect(provider).toBeEnabled();
  await expect(provider).toHaveValue('grok');
  await provider.selectOption('openai');
  await expect(provider).toHaveValue('openai');
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).user_ai).toEqual({...policy, cloud_provider:'openai'});
  await open();
  await expect(provider).toHaveValue('openai');
  const tools = page.getByRole('checkbox', {name:'Allow file creation & compilation'});
  await tools.check();
  await provider.selectOption('mlx');
  await expect(tools).toBeDisabled();
  await expect(tools).not.toBeChecked();
  await provider.selectOption('openai');
  await expect(tools).toBeEnabled();
  await expect(tools).toBeChecked();
  const settings = await (await request.get('/api/settings')).json();
  expect(settings.provider).toBe('grok');
  for (const mode of ['ondevice', 'hybrid']) {
    expect((await request.patch('/api/ai/preferences', {data:{...policy, mode}})).ok()).toBeTruthy();
    await open();
    await expect(provider).toHaveValue('mlx');
    await expect(page.getByRole('checkbox', {name:'Allow file creation & compilation'})).toBeDisabled();
    await expect(page.getByTestId('chat-tools-unavailable')).toContainText('Run skills from Skills');
    await expect(page.getByTestId('chat-skills-available')).toHaveCount(0);
    if (mode === 'hybrid') await expect(page.getByTestId('hybrid-scope')).toContainText('Hybrid keeps Chat and remote requests on-device');
    expect((await request.post('/api/think', {data:{provider:'openai', prompt:'Keep private'}})).status()).toBe(400);
  }
  expect((await request.patch('/api/ai/preferences', {data:policy})).ok()).toBeTruthy();
  await open();
  await expect(provider).toBeEnabled();
  await expect(provider).toHaveValue('grok');
  await expect(page.getByRole('checkbox', {name:'Allow file creation & compilation'})).toBeEnabled();
  await expect(page.getByTestId('chat-tools-unavailable')).toHaveCount(0);
});

test('TextEdit permission refreshes before any session exists without starting Auto', async ({page,request}) => {
  await request.patch('/api/settings', {data:{provider:'mlx', local_ai:{local_only:false,mlx_enabled:true},default_ai:{mode:'ondevice',local_provider:'mlx',cloud_provider:'grok'}}});
  await request.patch('/api/ai/preferences', {data:{mode:'ondevice',local_provider:'mlx',cloud_provider:'grok'}});
  expect((await request.patch('/api/assistance/preferences', {data:{textedit_enabled:false}})).ok()).toBeTruthy();
  const ws = new WebSocket('ws://127.0.0.1:3167/api/client/ws');
  const commands=[];
  try {
    await new Promise((resolve,reject) => {
      ws.onerror=reject;
      ws.onopen=() => ws.send(JSON.stringify({type:'hello',id:'policy-mac',name:'Policy test Mac',platform:'macos',textedit_notes_v1:true}));
      ws.onmessage=event => { const v=JSON.parse(event.data); if(v.type==='ready')resolve(); if(v.type==='notes_command')commands.push(v); };
    });
    await page.goto('/');
    await openWorkspace(page);
    await page.getByRole('combobox', {name:'Active capture device', exact:true}).selectOption('policy-mac');
    const auto=page.getByTestId('textedit-auto-toggle');
    const disabled=page.getByText('TextEdit assistance is disabled. Enable it in assistance settings to use Auto.');
    await expect(auto).toBeDisabled();
    await expect(disabled).toBeVisible();
    // The chatroom opened by the workspace is listed, but nothing is running or bound.
    const sessions = await (await request.get('/api/invisible/sessions')).json();
    expect(sessions.every(s => !s.status.active && !s.capture.active && s.client_id === '')).toBe(true);
    expect(commands).toHaveLength(0);
    expect((await request.patch('/api/assistance/preferences', {data:{textedit_enabled:true}})).ok()).toBeTruthy();
    await expect(auto).toBeEnabled();
    await expect(disabled).toHaveCount(0);
    await expect(auto).toHaveAttribute('aria-pressed','false');
    expect(commands).toHaveLength(0);
    expect((await request.patch('/api/assistance/preferences', {data:{textedit_enabled:false}})).ok()).toBeTruthy();
    await expect(auto).toBeDisabled();
    await expect(disabled).toBeVisible();
    expect(commands).toHaveLength(0);
  } finally {
    ws.close();
    await request.patch('/api/assistance/preferences', {data:{textedit_enabled:true}});
  }
});
