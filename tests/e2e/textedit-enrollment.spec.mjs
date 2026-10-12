import {openWorkspace} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Public test wallet. Wallet authentication is deliberately enabled: a demo-only
// anonymous workspace cannot reproduce shared-capture vs note-writing access.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const owner = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const baseURL = 'http://127.0.0.1:3182';
let root, backend;
test.use({baseURL});
test.beforeAll(async ({request}) => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'textedit-enrollment-'));
  backend = spawn('./target/debug/causewaybay-server', ['serve','--demo','--no-semantic'], {
    env:{...process.env,CWBBOT_ADMIN:owner,APP_TOKEN:'',BIND_ADDR:'127.0.0.1:3182',DATA_DIR:path.join(root,'data'),CAUSEWAYBAY_CONFIG_DIR:root,
      XAI_API_KEY:'',OPENAI_API_KEY:'',ANTHROPIC_API_KEY:'',GEMINI_API_KEY:''},stdio:'ignore',
  });
  await expect.poll(async () => (await request.get('/api/health').catch(() => null))?.status()).toBe(200);
});
test.afterAll(async () => {
  if (backend?.exitCode === null && backend.signalCode === null) {
    const exited = once(backend,'exit'); backend.kill('SIGTERM'); await exited;
  }
  if (root) fs.rmSync(root,{recursive:true,force:true});
});
test.beforeEach(async ({page}) => {
  await page.goto('/');
  await page.getByLabel('Recovery phrase',{exact:true}).fill(phrase);
  await page.getByRole('button',{name:'Sign in',exact:true}).click();
  await expect.poll(async () => (await (await page.request.get('/api/auth/info')).json()).authenticated).toBe(true);
  for (const session of await (await page.request.get('/api/invisible/sessions')).json()) {
    await page.request.post(`/api/invisible/sessions/${session.id}/stop`);
    await page.request.delete(`/api/rooms/${session.id}`);
  }
  for (const [url,data] of [
    ['/api/settings',{provider:'apple',model:'system',local_ai:{local_only:false,apple_enabled:true,mlx_enabled:false,mlx_url:'http://127.0.0.1:8080/v1',speech_provider:'auto',speech_locale:'en-US',mlx_speech_model:'',auto_interval_seconds:2, local_interval_seconds:2, cloud_min_interval_seconds:2}}],
    ['/api/ai/preferences',{mode:'ondevice',local_provider:'apple',cloud_provider:'grok'}],
    ['/api/assistance/preferences',{textedit_enabled:true,textedit_settle_ms:500}],
  ]) expect((await page.request.patch(url,{data})).ok()).toBe(true);
});

async function mac({id=randomUUID(),wallet=null,capable=true}={}) {
  const commands = [];
  let text = 'What is Bitcoin?';
  const socket = new WebSocket('ws://127.0.0.1:3182/api/client/ws');
  await new Promise((resolve,reject) => {
    socket.onopen = () => socket.send(JSON.stringify({type:'hello',id,name:'Enrollment test Mac',platform:'macos',owner_wallet:wallet,textedit_notes_v1:capable}));
    socket.onerror = reject;
    socket.onmessage = event => {
      const message = JSON.parse(event.data);
      if (message.type === 'ready') resolve();
      if (message.type !== 'notes_command') return;
      commands.push(message);
      const ok = message.operation !== 'append' || message.expected === text;
      if (ok && message.operation === 'append') text += message.suffix;
      socket.send(JSON.stringify({type:'notes_result',request_id:message.request_id,ok,snapshot:{document:'Enrollment.txt',text}}));
    };
  });
  const timer = setInterval(() => {if (socket.readyState === 1) socket.send(JSON.stringify({type:'heartbeat'}));},10000);
  return {id,commands,get text(){return text;},close(){clearInterval(timer);socket.close();}};
}
async function choose(page,id) {
  await page.goto('/');
  const picker = page.getByRole('combobox',{name:'Active capture device',exact:true});
  await expect(picker.locator(`option[value="${id}"]`)).toHaveCount(1);
  await picker.selectOption(id);
}
async function device(page,id) {
  return (await (await page.request.get('/api/clients')).json()).find(c => c.id === id);
}
async function stopSessions(page) {
  for (const s of await (await page.request.get('/api/invisible/sessions')).json()) await page.request.post(`/api/invisible/sessions/${s.id}/stop`);
}

test('shared Mac keeps screen capture but explains enrollment before Auto or Cloud session creation', async ({page},info) => {
  const shared = await mac();
  try {
    await choose(page,shared.id);
    expect(await device(page,shared.id)).toMatchObject({can_capture:true,textedit:'needs_enrollment'});
    await openWorkspace(page); const auto = page.getByTestId('textedit-auto-toggle');
    await expect(auto).toBeDisabled();
    await expect(page.locator('#notes-start-hint')).toContainText('This Mac is shared for screen capture.');
    await expect(page.locator('.chat-dock').getByRole('button',{name:'Capture',exact:true})).toBeEnabled();
    expect((await page.request.post('/api/think',{data:{capture:true,client_id:shared.id}})).ok()).toBe(true);
    // A shared Mac can hold a capture session, but TextEdit replies need an enrolled Mac.
    const created = await page.request.post('/api/invisible/sessions',{data:{client_id:shared.id}});
    expect(created.status()).toBe(200);
    const session = await created.json();
    const denied = await page.request.post(`/api/invisible/sessions/${session.id}/start`);
    expect(denied.status()).toBe(400);
    expect(await denied.text()).toContain('enrolled');
    const listed = await (await page.request.get('/api/invisible/sessions')).json();
    expect(listed.find(s=>s.id===session.id)).toMatchObject({client_id:shared.id});
    expect(listed.every(s=>!s.status.active)).toBe(true);
    const started = await page.request.post('/api/invisible/sessions',{data:{client_id:shared.id,start:true}});
    expect(started.status()).toBe(200);
    expect((await started.json()).status).toMatchObject({active:false,error:expect.stringContaining('enrolled')});
    await page.getByTestId('language-select').selectOption('ko');
    await expect(page.locator('#notes-start-hint')).toContainText('이 Mac은 화면 캡처 공유 모드입니다.');
    await expect(auto).toHaveAccessibleName('자율 AI 실행 시작');
    await page.getByRole('link',{name:'Mac 연결 설정',exact:true}).click();
    // The setup command includes the authenticated wallet; never a guessed admin.
    await expect(page.locator('.copyable-command code').first()).toContainText(owner);
    await expect(page.locator('.device-enrollment-help')).toContainText('다음 실행에도 계정 연결이 유지됩니다.');
    await page.getByTestId('language-select').selectOption('en');
    await page.goto('/autonomous');
    await page.getByRole('button',{name:'New session',exact:true}).click();
    const picker = page.getByRole('combobox',{name:'Capture computer (optional)',exact:true});
    await expect(picker.locator(`option[value="${shared.id}"]`)).toHaveJSProperty("disabled",true);
    await expect(picker.locator(`option[value="${shared.id}"]`)).toContainText('Account setup required');
    await page.getByRole('textbox',{name:'Session name'}).fill('Shared Mac');
    await expect(page.getByRole('button',{name:'Create session',exact:true})).toBeDisabled();
    await expect(page.getByRole('form',{name:'New session',exact:true}).getByRole('link',{name:'Set up this Mac'})).toBeVisible();
    await page.screenshot({path:info.outputPath('enrollment-required.png'),fullPage:true});
    expect(shared.commands).toEqual([]);
  } finally {shared.close();}
});

test('enrolling the same Mac enables Auto without reload, writes a reply, and stops from Cloud', async ({page},info) => {
  let client = await mac();
  try {
    await choose(page,client.id);
    await openWorkspace(page); const auto = page.getByTestId('textedit-auto-toggle');
    await expect(auto).toBeDisabled();
    const id = client.id;
    client.close();
    client = await mac({id,wallet:owner});
    await expect.poll(async () => (await device(page,id)).textedit).toBe('ready');
    await expect(auto).toBeEnabled();
    await auto.click();
    await expect(auto).toHaveAttribute('aria-pressed','true');
    await expect.poll(() => client.text).toContain('[End Invisible AI]');
    expect(client.commands.some(c => c.operation === 'append')).toBe(true);
    const sessions = await (await page.request.get('/api/invisible/sessions')).json();
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({client_id:id,status:{active:true}});
    await page.goto('/autonomous');
    const card = page.locator(`[data-session-id="${sessions[0].id}"]`);
    await expect(card.locator('.cloud-session-state')).toHaveText('Running');
    await card.getByRole('button',{name:'Stop autonomous AI',exact:true}).click();
    await expect(card.locator('.cloud-session-state')).toHaveText('Stopped');
    await page.reload();
    await expect(card.getByRole('button',{name:'Start autonomous AI',exact:true})).toBeEnabled();
    await page.screenshot({path:info.outputPath('enrolled-session.png'),fullPage:true});
    // Losing account enrollment cannot restart an existing session through the API.
    client.close(); client = await mac({id});
    await expect(card.getByRole('button',{name:'Start autonomous AI',exact:true})).toBeDisabled();
    await expect(card.locator('.cloud-start-hint')).toContainText('shared for screen capture');
    expect((await page.request.post(`/api/invisible/sessions/${sessions[0].id}/start`)).status()).toBe(400);
  } finally {await stopSessions(page);client.close();}
});

test('an enrolled Mac needs a capable live CLI and recovers after updating it', async ({page}) => {
  let client = await mac({wallet:owner,capable:false});
  try {
    await choose(page,client.id);
    await openWorkspace(page); const auto = page.getByTestId('textedit-auto-toggle');
    await expect(auto).toBeDisabled();
    await expect(page.locator('#notes-start-hint')).toContainText('Reconnect the updated Mac CLI');
    const id = client.id;
    client.close(); client = await mac({id,wallet:owner});
    await expect(auto).toBeEnabled();
    client.close();
    await expect(auto).toBeDisabled();
    await expect(page.locator('#notes-start-hint')).toContainText('Reconnect this capture Mac');
  } finally {client.close();}
});
