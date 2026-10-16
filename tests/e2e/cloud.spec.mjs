import {expectProcessingMode, revealNav} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';
import {randomUUID} from 'node:crypto';
import {isolatedBackend} from './invisible-fixture.mjs';

const baseURL = isolatedBackend(test, 3168);
const local = {local_only:false, apple_enabled:true, mlx_enabled:false, mlx_url:'http://127.0.0.1:8080/v1', speech_provider:'auto', speech_locale:'en-US', mlx_speech_model:'', auto_interval_seconds:2, local_interval_seconds:2, cloud_min_interval_seconds:2};
const policy = {mode:'ondevice', local_provider:'apple', cloud_provider:'grok'};
test.beforeEach(async ({request}) => {
  for (const session of await (await request.get('/api/invisible/sessions')).json()) {
    await request.post(`/api/invisible/sessions/${session.id}/stop`);
    await request.delete(`/api/rooms/${session.id}`);
  }
  expect((await request.patch('/api/settings', {data:{provider:'apple',model:'system',local_ai:local}})).ok()).toBeTruthy();
  expect((await request.patch('/api/ai/preferences', {data:policy})).ok()).toBeTruthy();
  // Approval tests below opt into confirmation; automatic cloud help is the default.
  expect((await request.patch('/api/assistance/preferences', {data:{textedit_enabled:true,hybrid_confirm:true}})).ok()).toBeTruthy();
});
// Playwright skips a test's `finally` after a timeout: close fake Macs and stop
// sessions from a hook so nothing outlives the test.
const devices = [];
test.afterEach(async ({request}) => {
  for (const device of devices.splice(0)) device.close();
  await stopAll(request).catch(() => {});
});

async function mac() {
  const id = `cloud-${randomUUID()}`, commands = [];
  let text = 'Explain what Bitcoin is.';
  let errorCode = null;
  const socket = new WebSocket('ws://127.0.0.1:3168/api/client/ws');
  await new Promise((resolve,reject) => {
    socket.onopen = () => socket.send(JSON.stringify({type:'hello',id,name:'Studio Mac',platform:'macos',textedit_notes_v1:true,mac_actions_v1:true}));
    socket.onerror = reject;
    socket.onmessage = event => {
      const message = JSON.parse(event.data);
      if (message.type === 'ready') resolve();
      if (message.type !== 'notes_command') return;
      commands.push(message);
      if (errorCode && message.operation !== 'stop') {
        socket.send(JSON.stringify({type:'notes_result',request_id:message.request_id,ok:false,error_code:errorCode,error:'Legacy generic failure'}));
        return;
      }
      if (message.operation === 'open_url') {
        socket.send(JSON.stringify({type:'notes_result',request_id:message.request_id,ok:true,opened:message.url}));
        return;
      }
      const ok = message.operation !== 'append' || message.expected === text;
      if (ok && message.operation === 'append') text += message.suffix;
      socket.send(JSON.stringify({type:'notes_result',request_id:message.request_id,ok,snapshot:{document:message.app==='notes'?'notes:x-coredata://Studio':'Studio.txt',text}}));
    };
  });
  const heartbeat = setInterval(() => {if (socket.readyState === 1) socket.send(JSON.stringify({type:'heartbeat'}));},10000);
  const device = {id,commands,get text(){return text;},set text(value){text=value;},set errorCode(value){errorCode=value;},close(){clearInterval(heartbeat);socket.close();}};
  devices.push(device);
  return device;
}
// The AI mode cards, the save status and the Hybrid confirmation checkbox sit
// inside the "AI settings & usage" fold of the Autonomous page, closed by
// default (and again after every reload); role queries skip folded content.
async function openAiSettings(page) {
  const details = page.locator('.simple-ai-settings');
  const summary = details.locator(':scope > summary');
  await expect(summary).toBeVisible();
  if ((await details.getAttribute('open')) === null) await summary.click();
  return page.getByRole('group',{name:/AI processing mode|AI 처리 위치/});
}
async function create(page,device,title='Studio notes') {
  await page.getByRole('button',{name:'Add a helper',exact:true}).click();
  await page.getByRole('textbox',{name:'Name your helper'}).fill(title);
  await page.getByRole('combobox',{name:'Which Mac?'}).selectOption(device.id);
  await page.getByRole('button',{name:'Save for later',exact:true}).click();
  const card = page.getByRole('article',{name:title,exact:true});
  await expect(card).toBeVisible();
  return card;
}
async function stopAll(request) {
  for (const s of await (await request.get('/api/invisible/sessions')).json()) await request.post(`/api/invisible/sessions/${s.id}/stop`);
}

test('Cloud navigation is localized, empty state is safe, and direct links survive reload',async ({page,request}) => {
  await page.goto('/');
  await page.getByTestId('language-select').selectOption('ko');
  const open = page.getByRole('button',{name:'메뉴 열기'});
  if (await open.isVisible()) await open.click();
  // Autonomous AI sits inside the sidebar's "More tools" fold.
  await revealNav(page,'autonomous');
  const nav = page.locator('[data-nav="autonomous"]');
  await expect(nav).toHaveText('자율 AI');
  await nav.click();
  await expect(page).toHaveURL(/\/autonomous$/);
  await expect(page.getByRole('heading',{name:'하던 일은 그대로, 도움은 곁에서.'})).toBeVisible();
  // No helpers yet: the empty state invites the first one instead of listing chats.
  await expect(page.getByRole('heading',{name:'첫 도우미를 만들어 볼까요?'})).toBeVisible();
  await page.getByRole('button',{name:'도우미 만들기',exact:true}).click();
  await page.getByRole('textbox',{name:'어떤 이름으로 부를까요?'}).fill('내 Mac');
  await expect(page.getByRole('button',{name:'나중에 시작하기'})).toBeEnabled();
  await page.reload();
  await expect(page.getByRole('heading',{name:'하던 일은 그대로, 도움은 곁에서.'})).toBeVisible();
  expect((await (await request.get('/api/ai/preferences')).json()).mode).toBe('ondevice');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
});

test('TextEdit permission errors stay visible in Korean and the same session recovers',async ({page,request}) => {
  const device = await mac();
  try {
    device.errorCode = 'permission_denied';
    await page.goto('/autonomous');
    const card = await create(page,device,'Permission recovery');
    const id = await card.getAttribute('data-session-id');
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect(card.locator('.notes-error')).toContainText('Privacy & Security → Automation');
    await expect(card.locator('.cloud-session-state')).toHaveText('Stopped');
    await page.getByTestId('language-select').selectOption('ko');
    await expect(card.locator('.notes-error')).toContainText('개인정보 보호 및 보안 → 자동화');
    await page.reload();
    await expect(card.locator('.notes-error')).toBeVisible();
    await expect(card.locator('.notes-error')).toContainText('TextEdit 제어를 허용');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
    device.errorCode = null;
    await card.getByRole('button',{name:'도움 시작'}).click();
    await expect.poll(() => device.text).toContain('[End Invisible AI]');
    await expect(card.locator('.notes-error')).toHaveCount(0);
    expect((await (await request.get(`/api/invisible/sessions/${id}`)).json()).id).toBe(id);
    await card.getByRole('button',{name:'도움 멈추기'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('중지됨');
  } finally {await stopAll(request);device.close();}
});

test('create, start, continue without UI, then stop the same backend session from another browser',async ({page,browser,request},info) => {
  const device = await mac(); let secondContext;
  try {
    await page.goto('/autonomous');
    const card = await create(page,device);
    await expect(card.locator('.cloud-session-state')).toHaveText('Stopped');
    expect(device.commands).toHaveLength(0);
    const id = await card.getAttribute('data-session-id');
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('Running');
    await expect.poll(() => device.text).toContain('[End Invisible AI]');
    await page.getByTestId('language-select').selectOption('ko');
    await expect(card.getByRole('button',{name:'도움 멈추기'})).toBeVisible();
    await expect(page.getByTestId('connection-activity')).toHaveCount(0);
    await page.evaluate(() => window.scrollTo(0,0));
    await page.screenshot({path:`test-results/cloud-${info.project.name}.png`,fullPage:true});
    await page.getByRole('button',{name:'다크 모드로 전환'}).click();
    await page.screenshot({path:`test-results/cloud-dark-${info.project.name}.png`,fullPage:true});
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
    await page.close();
    device.text += '\nHow does mining work?';
    await expect.poll(() => device.commands.filter(c=>c.operation==='append').length).toBe(2);
    secondContext = await browser.newContext(); const second = await secondContext.newPage();
    await second.goto(baseURL+'/autonomous');
    const restored = second.locator(`[data-session-id="${id}"]`);
    await expect(restored.locator('.cloud-session-state')).toHaveText('Running');
    await restored.getByRole('button',{name:'Stop helping'}).click();
    await expect(restored.locator('.cloud-session-state')).toHaveText('Stopped');
    expect((await (await request.get(`/api/invisible/sessions/${id}`)).json()).status.active).toBe(false);
    expect(device.commands.some(c=>c.operation==='stop')).toBeTruthy();
    await second.reload();
    await expect(restored.locator('.cloud-session-state')).toHaveText('Stopped');
    expect((await (await request.get('/api/ai/preferences')).json()).mode).toBe('ondevice');
  } finally {await stopAll(request); await secondContext?.close(); device.close();}
});

test('Hybrid shows exact pending content, keeps rejected notes local, and approves only once',async ({page,request}) => {
  const device = await mac();
  try {
    await request.patch('/api/ai/preferences',{data:{...policy,mode:'hybrid'}});
    await page.goto('/autonomous');
    const card = await create(page,device,'Hybrid notes');
    await card.getByRole('button',{name:'Start helping'}).click();
    const approval = card.getByRole('region',{name:'Cloud approval'});
    await expect(approval.locator('pre')).toHaveText(device.text);
    await expect(card.locator('.cloud-session-state')).toHaveText('Awaiting approval');
    expect(device.commands.filter(c=>c.operation==='append')).toHaveLength(0);
    await approval.getByRole('button',{name:'Keep it local'}).click();
    await expect(approval).toHaveCount(0);
    expect(device.commands.filter(c=>c.operation==='append')).toHaveLength(0);
    device.text += '\nExplain the next idea.';
    await expect(approval.locator('pre')).toHaveText(device.text);
    await approval.getByRole('button',{name:'Approve once'}).click();
    await expect.poll(()=>device.text).toContain('[Invisible AI · cloud approved]');
    await expect(approval).toHaveCount(0);
    await card.getByRole('button',{name:'Stop helping'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('Stopped');
    expect(device.commands.filter(c=>c.operation==='append')).toHaveLength(1);
  } finally {await stopAll(request);device.close();}
});

test('disabled TextEdit and offline Mac prevent starts while other sessions remain manageable',async ({page,request}) => {
  const device = await mac();
  try {
    await page.goto('/autonomous');
    const first = await create(page,device,'First notes');
    const second = await create(page,device,'Second notes');
    await request.patch('/api/assistance/preferences',{data:{textedit_enabled:false}});
    await expect(first.getByRole('button',{name:'Start helping'})).toBeDisabled();
    await expect(second.getByRole('button',{name:'Start helping'})).toBeDisabled();
    await expect(first.locator('.cloud-start-hint')).toContainText('TextEdit assistance is disabled');
    expect(device.commands).toHaveLength(0);
    await request.patch('/api/assistance/preferences',{data:{textedit_enabled:true}});
    await expect(first.getByRole('button',{name:'Start helping'})).toBeEnabled();
    await first.getByRole('button',{name:'Start helping'}).click();
    await expect(first.locator('.cloud-session-state')).toHaveText('Running');
    await expect(second.locator('.cloud-session-state')).toHaveText('Stopped');
    await expect(second.getByRole('button',{name:'Start helping'})).toBeDisabled();
    await expect(second.locator('.cloud-start-hint')).toHaveText('Another session is running on this Mac. Stop it first.');
    await first.getByRole('button',{name:'Stop helping'}).click();
    await expect(first.locator('.cloud-session-state')).toHaveText('Stopped');
    await expect(second.getByRole('button',{name:'Start helping'})).toBeEnabled();
    await second.getByRole('button',{name:'Start helping'}).click();
    await expect(second.locator('.cloud-session-state')).toHaveText('Running');
    await expect(first.getByRole('button',{name:'Start helping'})).toBeDisabled();
    device.close();
    await expect(first.locator('.cloud-session-state')).toHaveText('Stopped');
    await expect(first.getByRole('button',{name:'Start helping'})).toBeDisabled();
    await expect(second.locator('.cloud-session-state')).toHaveText('Stopped');
    await expect(second.getByRole('button',{name:'Start helping'})).toBeDisabled();
    await expect(second.locator('.cloud-start-hint')).toHaveText('Reconnect this capture Mac to start the session.');
  } finally {await stopAll(request);device.close();}
});

test('session loading and failed actions recover without losing persisted sessions',async ({page,request}) => {
  const device = await mac();
  let failLoad = true, failStart = false;
  await page.routeWebSocket('**/api/ws', socket => {
    const server = socket.connectToServer();
    socket.onMessage(raw => {
      const message = JSON.parse(String(raw));
      const error = failLoad && message.path === '/api/invisible/sessions' && message.method === 'GET'
        ? 'Session service unavailable'
        : failStart && message.path?.endsWith('/start') ? 'Mac temporarily unavailable' : '';
      if (error) socket.send(JSON.stringify({type:'response',id:message.id,status:503,data:{error}}));
      else server.send(raw);
    });
  });
  try {
    await page.goto('/autonomous');
    await expect(page.locator('.cloud-page [role="alert"]')).toContainText('Session service unavailable');
    await expect(page.locator('.cloud-ai-save')).toHaveText('Waiting for backend');
    failLoad = false;
    await expect(page.locator('.cloud-page [role="alert"]')).toHaveCount(0);
    const card = await create(page,device,'Recovered notes');
    failStart = true;
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect(page.locator('.cloud-page [role="alert"]')).toContainText('Mac temporarily unavailable');
    await expect(card.locator('.cloud-session-state')).toHaveText('Stopped');
    failStart = false;
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('Running');
    await expect(page.locator('.cloud-page [role="alert"]')).toHaveCount(0);
    failLoad = true;
    await expect(page.locator('.cloud-page [role="alert"]')).toContainText('Session service unavailable');
    await expect(page.locator('.cloud-ai-save')).toHaveText('Waiting for backend');
    await expect(card.locator('.cloud-session-state')).toHaveText('Running');
    failLoad = false;
    await expect(page.locator('.cloud-page [role="alert"]')).toHaveCount(0);
  } finally {await stopAll(request);device.close();}
});

test('Cloud mode cards save account policy, preserve providers, and sync across browsers',async ({page,browser,request}) => {
  let otherContext;
  const selected = {mode:'ondevice',local_provider:'apple',cloud_provider:'openai'};
  await request.patch('/api/ai/preferences',{data:selected});
  try {
    await page.goto('/autonomous');
    await page.getByTestId('language-select').selectOption('ko');
    await openAiSettings(page);
    const group = page.getByRole('group',{name:'AI 처리 위치'});
    await expect(group.getByRole('button',{name:/온디바이스/})).toHaveAttribute('aria-pressed','true');
    otherContext = await browser.newContext(); const other = await otherContext.newPage();
    await other.goto(baseURL+'/autonomous');
    await openAiSettings(other);
    const otherGroup = other.getByRole('group',{name:'AI processing mode'});
    for (const [mode,label,en] of [['hybrid','하이브리드','Hybrid'],['cloudai','클라우드 AI','Cloud AI'],['ondevice','온디바이스','On-device']]) {
      await group.getByRole('button',{name:new RegExp(label)}).click();
      await expect(group.getByRole('button',{name:new RegExp(label)})).toHaveAttribute('aria-pressed','true');
      expect(await(await request.get('/api/ai/preferences')).json()).toEqual({...selected,mode});
      await expect(otherGroup.getByRole('button',{name:new RegExp(en)})).toHaveAttribute('aria-pressed','true');
      await page.reload();
      await openAiSettings(page);
      await expect(group.getByRole('button',{name:new RegExp(label)})).toHaveAttribute('aria-pressed','true');
    }
    expect(await(await request.get('/api/invisible/sessions')).json()).toEqual([]);
    await group.getByRole('button',{name:/하이브리드/}).click();
    await expect(group.getByRole('button',{name:/하이브리드/})).toHaveAttribute('aria-pressed','true');
    await page.goto('/');
    await expectProcessingMode(page,'하이브리드');
  } finally {await otherContext?.close();}
});

test('Cloud mode cards respect the admin Local-only restriction and restore personal policy',async ({page,request}) => {
  await request.patch('/api/ai/preferences',{data:{...policy,mode:'cloudai'}});
  const saved = await request.patch('/api/settings',{data:{provider:'apple',local_ai:{...local,local_only:true},default_ai:policy}});
  expect(saved.ok()).toBeTruthy();
  await page.goto('/autonomous');
  await openAiSettings(page);
  const group = page.getByRole('group',{name:'AI processing mode'});
  await expect(group.getByRole('button',{name:/On-device/})).toHaveAttribute('aria-pressed','true');
  await expect(group.getByRole('button',{name:/Hybrid/})).toBeDisabled();
  await expect(group.getByRole('button',{name:/Cloud AI/})).toBeDisabled();
  await expect(page.locator('.cloud-mode-hint')).toContainText('limited this backend to On-device');
  expect((await request.patch('/api/ai/preferences',{data:{...policy,mode:'cloudai'}})).status()).toBe(400);
  expect((await request.patch('/api/settings',{data:{provider:'apple',local_ai:local}})).ok()).toBeTruthy();
  await page.reload();
  await openAiSettings(page);
  await expect(group.getByRole('button',{name:/Cloud AI/})).toHaveAttribute('aria-pressed','true');
  await expect(group.getByRole('button',{name:/Hybrid/})).toBeEnabled();
});

test('failed mode saves keep the old selection and delayed polls cannot undo a successful change',async ({page,request}) => {
  let rejectSave = true, holdNextRead = false, held, release;
  const pending = new Set();
  await page.routeWebSocket('**/api/ws',socket => {
    const server = socket.connectToServer();
    socket.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if(message.method === 'GET' && message.path === '/api/ai/preferences' && holdNextRead) {
        holdNextRead = false; pending.add(message.id);
      }
      if(message.method === 'PATCH' && message.path === '/api/ai/preferences' && rejectSave) {
        socket.send(JSON.stringify({type:'response',id:message.id,status:503,data:{error:'Mode save temporarily unavailable'}}));
      } else server.send(raw);
    });
    server.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if(pending.delete(message.id)) {held=message;release=()=>socket.send(raw);}
      else socket.send(raw);
    });
  });
  await page.goto('/autonomous');
  await openAiSettings(page);
  const group = page.getByRole('group',{name:'AI processing mode'});
  await expect(group.getByRole('button',{name:/On-device/})).toBeEnabled();
  await group.getByRole('button',{name:/Hybrid/}).click();
  await expect(page.locator('.cloud-ai-settings [role="alert"]')).toHaveText('Mode save temporarily unavailable');
  await expect(group.getByRole('button',{name:/On-device/})).toHaveAttribute('aria-pressed','true');
  expect((await(await request.get('/api/ai/preferences')).json()).mode).toBe('ondevice');
  rejectSave = false; holdNextRead = true;
  await expect.poll(()=>held?.data?.mode).toBe('ondevice');
  await group.getByRole('button',{name:/Hybrid/}).click();
  await expect(group.getByRole('button',{name:/Hybrid/})).toHaveAttribute('aria-pressed','true');
  const oldRead = held;
  holdNextRead = true;
  release();
  // Wait until the stale response was consumed and the next poll is held, so
  // a reload or a newer response cannot conceal a reverted selection.
  await expect.poll(()=>held !== oldRead && held?.data?.mode === 'hybrid').toBe(true);
  await expect(group.getByRole('button',{name:/Hybrid/})).toHaveAttribute('aria-pressed','true');
  release();
  await expect(page.locator('.cloud-ai-settings [role="alert"]')).toHaveCount(0);
  await expect.poll(async()=>(await(await request.get('/api/ai/preferences')).json()).mode).toBe('hybrid');
  await page.reload();
  await openAiSettings(page);
  await expect(group.getByRole('button',{name:/Hybrid/})).toHaveAttribute('aria-pressed','true');
});

test('changing Cloud mode invalidates pending Hybrid approval without sending its note',async ({page,request}) => {
  const device = await mac();
  try {
    await request.patch('/api/ai/preferences',{data:{...policy,mode:'hybrid'}});
    await page.goto('/autonomous');
    const card = await create(page,device,'Mode switch notes');
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect(card.getByRole('region',{name:'Cloud approval'})).toBeVisible();
    const id = await card.getAttribute('data-session-id');
    const approval = (await(await request.get(`/api/invisible/sessions/${id}`)).json()).status.approval;
    await (await openAiSettings(page)).getByRole('button',{name:/On-device/}).click();
    await expect(card.getByRole('region',{name:'Cloud approval'})).toHaveCount(0);
    await expect(card.locator('.cloud-session-detail [role="status"]')).toHaveText('AI settings changed. Write a new note to continue.');
    expect((await request.post(`/api/invisible/sessions/${id}/approval`,{data:{id:approval.id,approve:true}})).status()).toBe(400);
    expect(device.commands.filter(c=>c.operation==='append')).toHaveLength(0);
    device.text += '\nA new idea after switching to local.';
    await expect.poll(()=>device.text).toContain('[End Invisible AI]');
    expect(device.text).not.toContain('cloud approved');
    await card.getByRole('button',{name:'Stop helping'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('Stopped');
  } finally {await stopAll(request);device.close();}
});

test('an unconfigured provider explains why a connected session cannot start',async ({page,request}) => {
  const device = await mac();
  try {
    // Settings GET uses HTTP even when session controls use WebSocket RPC.
    await page.route('**/api/settings',async route => {
      const response = await route.fetch();
      const settings = await response.json();
      settings.demo = false;
      for(const provider of settings.providers) provider.configured = false;
      await route.fulfill({response,json:settings});
    });
    await page.goto('/autonomous');
    const card = await create(page,device,'Needs AI setup');
    await expect(card.getByRole('button',{name:'Start helping'})).toBeDisabled();
    await expect(card.locator('.cloud-start-hint')).toHaveText('Enable the selected AI provider in Backend settings before starting.');
    expect(device.commands).toHaveLength(0);
  } finally {await stopAll(request);device.close();}
});

test('remote control sends phone requests to the Mac: Safari, calculator, model answer, and Korean labels',async ({page,request},info) => {
  const device = await mac();
  try {
    await page.goto('/autonomous');
    await page.getByRole('button',{name:'Add a helper',exact:true}).click();
    await page.getByRole('textbox',{name:'Name your helper'}).fill('Phone control');
    await page.getByRole('combobox',{name:'Which Mac?'}).selectOption(device.id);
    await page.getByRole('combobox',{name:'Where should I help?'}).selectOption('notes');
    await page.getByRole('button',{name:'Save for later',exact:true}).click();
    const card = page.getByRole('article',{name:'Phone control',exact:true});
    await expect(card).toBeVisible();
    const id = await card.getAttribute('data-session-id');
    const remote = card.getByTestId('remote-control');
    // Remote control only exists while the helper runs: nothing to send to a stopped one.
    await expect(remote).toHaveCount(0);
    await expect(card.getByRole('combobox',{name:'Watched app'})).toHaveValue('notes');
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('Running');
    await expect(remote).toBeVisible();
    await expect(remote.getByTestId('send-remote-prompt')).toBeDisabled();
    await expect.poll(() => device.text).toContain('[End Invisible AI]');
    expect(device.commands.every(c => c.app === 'notes')).toBeTruthy();
    await expect(card.getByRole('combobox',{name:'Watched app'})).toBeDisabled();
    // Explicit commands run without a model: Safari on the Mac, arithmetic on the backend.
    const box = remote.getByRole('textbox',{name:'Request for the Mac'});
    await remote.getByRole('button',{name:'Open Safari'}).click();
    await expect(box).toHaveValue('open ');
    await box.fill('open https://example.com/docs');
    await remote.getByTestId('send-remote-prompt').click();
    await expect.poll(() => device.commands.filter(c => c.operation === 'open_url').map(c => c.url)).toEqual(['https://example.com/docs']);
    await expect(remote.getByTestId('remote-action')).toHaveText('Opened in Safari: https://example.com/docs');
    await box.fill('calc 12*34');
    await box.press('Enter');
    await expect(remote.getByTestId('remote-action')).toHaveText('12*34 = 408');
    await expect.poll(() => device.text).toContain('[Invisible AI · remote]\ncalc 12*34\n12*34 = 408\n[End Invisible AI]');
    // Free text goes to the (demo) model and the reply shows on the card and in chat.
    await box.fill('Explain what a Merkle tree is');
    await box.press('Enter');
    await expect(remote.getByTestId('remote-reply')).toContainText('Here is help with your request: Explain what a Merkle tree is');
    expect(device.commands.filter(c => c.operation === 'open_url')).toHaveLength(1);
    const stats = (await (await request.get(`/api/invisible/sessions/${id}`)).json());
    expect(stats.activity.inferences.length).toBe(2);
    expect(stats.activity.entries.some(e => e.source === 'remote' && e.stage === 'Request complete. Send another from your device.')).toBeTruthy();
    expect(stats.activity.entries.some(e => e.source === 'notes' && e.stage === 'Connecting to Notes…')).toBeTruthy();
    const messages = (await (await request.get(`/api/rooms/${id}/sync?after=0`)).json()).messages;
    expect(messages.some(m => m.role === 'assistant' && m.provider === 'remote' && m.text === '12*34 = 408')).toBeTruthy();
    await page.getByTestId('language-select').selectOption('ko');
    await expect(remote.getByRole('button',{name:'Mac으로 보내기'})).toBeVisible();
    await expect(remote.getByRole('button',{name:'Safari 열기'})).toBeVisible();
    await expect(card.locator('.cloud-session-detail p[role="status"]')).toContainText('요청 완료');
    await page.setViewportSize({width:390,height:844});
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
    await remote.scrollIntoViewIfNeeded();
    await page.screenshot({path:`test-results/cloud-remote-${info.project.name}.png`,fullPage:true});
    await card.getByRole('button',{name:'도움 멈추기'}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('중지됨');
    await expect(remote).toHaveCount(0);
  } finally {await stopAll(request); device.close();}
});

test('Hybrid sends cloud help automatically unless confirmation is turned on',async ({page,request}) => {
  const device = await mac();
  try {
    await request.patch('/api/assistance/preferences',{data:{hybrid_confirm:false}});
    await request.patch('/api/ai/preferences',{data:{...policy,mode:'hybrid'}});
    await page.goto('/autonomous');
    await openAiSettings(page);
    const confirm = page.getByRole('checkbox',{name:'Ask before each cloud request'});
    await expect(confirm).not.toBeChecked();
    const card = await create(page,device,'Automatic cloud');
    await card.getByRole('button',{name:'Start helping'}).click();
    await expect.poll(() => device.text).toContain('[Invisible AI · cloud]\nHere is help with your note: Explain what Bitcoin is.\n[End Invisible AI]');
    await expect(card.getByRole('region',{name:'Cloud approval'})).toHaveCount(0);
    await expect(card.locator('.cloud-session-state')).toHaveText('Running');
    await expect(card.getByTestId('session-progress')).toContainText('Thinking with cloud help…');
    // Turning confirmation on applies to the next note without a restart.
    await confirm.click();
    await expect(confirm).toBeChecked();
    expect((await (await request.get('/api/assistance/preferences')).json()).hybrid_confirm).toBe(true);
    device.text += '\nAnd how is a block mined?';
    await expect(card.getByRole('region',{name:'Cloud approval'})).toBeVisible();
    await expect(card.locator('.cloud-session-state')).toHaveText('Awaiting approval');
  } finally {await stopAll(request); device.close();}
});
