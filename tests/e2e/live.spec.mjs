import {openConversationOptions} from './workspace-controls.mjs';
import { test, expect } from '@playwright/test';
import { observeEvents } from './events.mjs';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

test.use({ baseURL: 'http://127.0.0.1:3100', viewport: { width: 1024, height: 1366 } });
test.setTimeout(180000);
const configDir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-live-client-'));
let client;
function connectClient() {
  // Real Rust CLI and real WebSocket. The input is a known image instead of private screen content.
  client = spawn('./target/debug/causewaybay', ['client', '--backend', 'http://127.0.0.1:3100', '--name', 'Playwright MacBook', '--fixture-image', 'target/test-fixtures/capture.png', '--retry-seconds', '1'], { env: { ...process.env, CAUSEWAYBAY_CLIENT_CONFIG_DIR: configDir, APP_TOKEN: '' }, stdio: 'ignore' });
}
test.beforeAll(async ({ request }) => {
  expect(Boolean(process.env.XAI_API_KEY), 'XAI_API_KEY must be present for live verification').toBeTruthy();
  expect(fs.existsSync('target/test-fixtures/capture.png'), 'Run make test-vision first').toBeTruthy();
  connectClient();
  await expect.poll(async () => (await (await request.get('/api/clients')).json()).some(c => c.name === 'Playwright MacBook' && c.online), { timeout: 30000 }).toBeTruthy();
});
test.afterAll(() => client?.kill('SIGTERM'));

async function open(page) {
  await page.goto('/overview');
  await expect(page.getByRole('combobox', { name: 'Capture client' })).toContainText('Playwright MacBook');
  await expect(page.locator('.composer-provider')).toContainText('Grok');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('causewaybay.settings.v1') || '{}'))).not.toHaveProperty('keys');
}

test('MacBook CLI → backend Apple Vision → live Grok → iPad UI → SQLite', async ({ page, request }) => {
  const events = [];
  await observeEvents(page, event => events.push(event));
  await open(page);
  await openConversationOptions(page);
  await page.getByRole('button', { name: 'Text mode', exact: true }).click();
  await page.getByRole('textbox', { name: 'Ask a question' }).fill('Answer the arithmetic question on the captured screen.');
  await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.locator('.answer-content')).toContainText('12', { timeout: 150000 });
  await expect(page.locator('.save-status')).toHaveText('Saved to your library', { timeout: 150000 });
  expect(events.some(e => e.kind === 'ocr' && e.text.includes('7 + 5'))).toBeTruthy();
  expect(events.some(e => e.kind === 'stage' && e.text.includes('Apple Vision'))).toBeTruthy();
  expect(events.some(e => e.kind === 'delta')).toBeTruthy();
  await page.locator('.source-details > summary').click();
  await expect(page.getByTestId('capture-preview').locator('img')).toBeVisible();
  const memos = await (await request.get('/api/memos')).json();
  expect(memos.memos[0].has_image).toBeTruthy();
  expect(memos.memos[0].answer).toContain('12');
  expect(memos.memos[0].provider).toBe('grok');
  await page.screenshot({ path: 'test-results/live-text-ipad.png', fullPage: true });
});

test('Image mode sends a real image to Grok without OCR', async ({ page }) => {
  const events = [];
  await observeEvents(page, event => events.push(event));
  await open(page);
  await openConversationOptions(page);
  await page.getByRole('button', { name: 'Image mode', exact: true }).click();
  await page.getByRole('textbox', { name: 'Ask a question' }).fill('Solve the question in the screenshot.');
  await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.locator('.answer-content')).toContainText('12', { timeout: 150000 });
  await expect(page.locator('.save-status')).toHaveText('Saved to your library', { timeout: 150000 });
  expect(events.some(e => e.kind === 'stage' && e.text.includes('Apple Vision'))).toBeFalsy();
  expect(events.some(e => e.kind === 'ocr')).toBeFalsy();
  expect(events.some(e => e.kind === 'mode' && e.text === 'image')).toBeTruthy();
  await page.screenshot({ path: 'test-results/live-image-ipad.png', fullPage: true });
});

test('live Grok coding agent writes and compiles a Rust file', async ({ page, request }) => {
  await open(page);
  await page.goto('/chat');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await page.getByRole('checkbox', { name: 'Allow file creation & compilation', exact: true }).check();
  await page.getByRole('textbox', { name: 'Chat message', exact: true }).fill('Use your tools: write_file named live_sum.rs containing pub fn sum(a: i32, b: i32) -> i32 { a + b }. Then call compile_code on that file. Report whether compilation succeeded.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const reply = page.locator('.from-ai').last();
  await reply.getByRole('link', { name: 'Workspace activity', exact: true }).click({ timeout: 150000 });
  await expect(page.locator('.save-status')).toHaveText('Saved to your library', { timeout: 150000 });
  await expect(page.locator('.tool-activity')).toContainText('write file');
  await expect(page.locator('.tool-activity')).toContainText('compile code');
  await expect(page.locator('.tool-activity')).toContainText('Compilation succeeded');
  const files = await (await request.get('/api/files')).json();
  expect(files.some(f => f.name === 'live_sum.rs')).toBeTruthy();
  await page.screenshot({ path: 'test-results/live-agent-ipad.png', fullPage: true });
});

test('capture client reconnects with its JSONL identity', async ({ request }) => {
  const before = (await (await request.get('/api/clients')).json()).find(c => c.name === 'Playwright MacBook');
  client.kill('SIGTERM');
  await expect.poll(async () => (await (await request.get('/api/clients')).json()).find(c => c.id === before.id)?.online).toBe(false);
  connectClient();
  await expect.poll(async () => (await (await request.get('/api/clients')).json()).find(c => c.id === before.id)?.online).toBe(true);
  const records = fs.readFileSync(path.join(configDir, 'client.jsonl'), 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
  expect(records.at(-1).config.client_id).toBe(before.id);
});

test('capture all delivers SSE commands to two Rust clients and saves both Grok answers',async({page,request})=>{
  const folder=fs.mkdtempSync(path.join(os.tmpdir(),'causewaybay-live-second-'));
  let log='';
  const second=spawn('./target/debug/causewaybay',['client','--backend','http://127.0.0.1:3100','--name','Second capture Mac','--fixture-image','target/test-fixtures/capture.png'],{env:{...process.env,CAUSEWAYBAY_CLIENT_CONFIG_DIR:folder,APP_TOKEN:''},stdio:['ignore','pipe','pipe']});
  second.stdout.on('data',b=>{log+=b;});second.stderr.on('data',b=>{log+=b;});
  try{
    await expect.poll(async()=> (await (await request.get('/api/clients')).json()).filter(c=>c.online).length).toBe(2);
    const seen=new Map();
    await observeEvents(page,(event,id)=>{if(!seen.has(id))seen.set(id,[]);seen.get(id).push(event);});
    await open(page);
    await openConversationOptions(page);
    await page.getByRole('button',{name:'Text mode',exact:true}).click();
    await page.getByRole('combobox',{name:'Capture client'}).selectOption('all');
    await page.getByRole('textbox',{name:'Ask a question'}).fill('Solve the arithmetic question on the screenshot.');
    await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
    await expect(page.getByTestId('capture-result')).toHaveCount(2);
    await expect(page.locator('.batch-progress')).toHaveCount(0,{timeout:150000});
    expect(seen.size).toBe(2);
    for(const [id,events] of seen){
      expect(events.some(e=>e.kind==='ocr'&&e.text.includes('7 + 5'))).toBe(true);
      const memo=await(await request.get(`/api/memos/${id}`)).json();
      expect(memo.has_image).toBe(true);expect(memo.answer).toContain('12');expect(memo.status).toBe('complete');
    }
    expect(log).toContain('SSE callbacks connected.');
    expect(log).toContain('Screenshot received by backend.');
    await page.screenshot({path:'test-results/live-multiple-clients.png',fullPage:true});
  }finally{second.kill('SIGTERM');}
});

async function startNewChat(page) {
  await page.goto('/chat');
  await openConversationOptions(page);
  const select=page.getByRole('combobox',{name:'Chat room'});
  await expect(page.getByRole('button',{name:'New chat',exact:true})).toBeVisible();
  const before=await select.inputValue();
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  await expect(select).not.toHaveValue(before);
  await expect(select).not.toHaveValue('');
  await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toBeEnabled();
  return select.inputValue();
}

test('chat shows captured images in both modes, exact OCR in text mode, and checks the AI backend',async({page,request})=>{
  await startNewChat(page);
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await expect(page.getByLabel('AI server')).toContainText('grok');
  await page.getByRole('button',{name:'Check AI connection'}).click();
  await expect(page.getByLabel('AI server')).toContainText('responded',{timeout:60000});
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await page.getByRole('button',{name:'Text mode',exact:true}).click();
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Answer the arithmetic on the screenshot.');
  await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.locator('.chat-image img')).toHaveCount(1);
  await expect(page.locator('.chat-ocr')).toContainText('7 + 5',{timeout:30000});
  await expect(page.locator('.from-ai .answer-content').first()).toContainText('12',{timeout:150000});
  await openConversationOptions(page);
  await page.getByRole('button',{name:'Image mode',exact:true}).click();
  await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.locator('.chat-image img')).toHaveCount(2);
  await expect(page.locator('.from-ai .answer-content').nth(1)).toContainText('12',{timeout:150000});
  await expect(page.locator('.chat-ocr')).toHaveCount(1);
  await openConversationOptions(page);
  const room=await page.getByRole('combobox',{name:'Chat room'}).inputValue();
  const data=await (await request.get(`/api/rooms/${room}/sync`)).json();
  expect(data.messages.filter(m=>m.image_path).every(m=>m.image_path.startsWith('screenshots/'))).toBeTruthy();
  await page.screenshot({path:'test-results/live-chat-modes.png',fullPage:true});
});

test('Cloud polling uses real CLI callbacks, discards unchanged screens, and stops',async({page,request})=>{
  const room=await startNewChat(page);
  await request.patch('/api/settings',{data:{input_mode:'text'}});
  const clients=await (await request.get('/api/clients')).json();
  const client=clients.find(c=>c.online&&c.can_capture);
  expect(client).toBeTruthy();
  await page.goto('/autonomous');
  const card=page.locator(`[data-session-id="${room}"]`);
  await card.getByTestId('manual-capture').locator('> summary').click();
  await card.getByRole('combobox',{name:'Capture computer',exact:true}).selectOption(client.id);
  await card.getByLabel('Capture interval').fill('2');
  await card.getByRole('button',{name:'Start polling',exact:true}).click();
  try {
    await card.getByRole('link',{name:'Open chat history',exact:true}).click();
    await expect(page.locator('.from-ai .answer-content').first()).toContainText('12',{timeout:150000});
    await expect(page.locator('.chat-ocr').first()).toContainText('7 + 5');
    await expect.poll(async()=>(await (await request.get(`/api/rooms/${room}/capture`)).json()).unchanged_count).toBeGreaterThan(0);
    await page.getByRole('link',{name:'Manage in Autonomous AI'}).click();
    await card.getByRole('button',{name:'Stop polling',exact:true}).click();
    await expect(card.getByRole('button',{name:'Start polling',exact:true})).toBeVisible();
  } finally {
    expect((await request.delete(`/api/rooms/${room}/capture`)).ok()).toBeTruthy();
  }
  const before=(await (await request.get(`/api/rooms/${room}/sync`)).json()).next;
  await page.waitForTimeout(3000);
  expect((await (await request.get(`/api/rooms/${room}/sync?after=${before}`)).json()).messages).toEqual([]);
});

test('audio upload → backend xAI transcription → chat answer and backend file storage',async({page,request})=>{
  expect(fs.existsSync('target/test-fixtures/audio-question.wav'),'Run make test-audio-fixture').toBeTruthy();
  await startNewChat(page);
  await page.getByLabel('Attach to chat').setInputFiles('target/test-fixtures/audio-question.wav');
  await expect(page.locator('.chat-attachment')).toContainText('audio-question.wav');
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Answer the spoken math question.');
  await page.getByRole('button',{name:'Send message'}).click();
  await expect(page.locator('.chat-ocr')).toContainText(/seven|7/i,{timeout:90000});
  await expect(page.locator('.from-ai .answer-content')).toContainText('12',{timeout:150000});
  await expect(page.locator('audio')).toBeVisible();
  const file=(await (await request.get('/api/files')).json()).find(f=>f.name==='audio-question.wav');
  expect(file.mime).toBe('audio/wav');expect(file.path.startsWith('files/')).toBeTruthy();
  expect((await (await request.get(`/api/files/${file.id}/download`)).body()).equals(fs.readFileSync('target/test-fixtures/audio-question.wav'))).toBeTruthy();
});

test('ordinary chat automatically calculates and searches the private library',async({page,request})=>{
  await request.post('/api/memos',{data:{title:'Library test detail',content:'The launch codeword for the Orchid project is apricot-lantern-719.'}});
  const room=await startNewChat(page);
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Use calculate to compute (123 + 77) * 9. Then search my saved notes for the Orchid project launch codeword. Give both results.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-ai')).toContainText('apricot-lantern-719',{timeout:150000});
  await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0,{timeout:150000});
  await expect(page.locator('.from-ai')).toContainText(/1,?800/);
  const sync=await(await request.get(`/api/rooms/${room}/sync`)).json();const memo=sync.messages.find(m=>m.role==='user').memo_id;
  const events=await(await request.get(`/api/memos/${memo}/events`)).text();
  expect(events).toContain('calculate');expect(events).toContain('search_notes');
  expect((await(await request.get(`/api/memos/${memo}`)).json()).status).toBe('complete');
});

test('ordinary chat performs live web search with sources',async({page,request})=>{
  const room=await startNewChat(page);
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Use web_search to find the official Rust documentation for ownership. Cite the official source URL and give one sentence explaining ownership.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(1);
  await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0,{timeout:150000});
  await expect(page.locator('.from-ai a[href*="doc.rust-lang.org"]').first()).toBeVisible();
  const sync=await(await request.get(`/api/rooms/${room}/sync`)).json();const memo=sync.messages.find(m=>m.role==='user').memo_id;
  const events=await(await request.get(`/api/memos/${memo}/events`)).text();
  expect(events).toContain('web_search');expect(events).toContain('tool_result');expect(events).toContain('sources');
  expect((await(await request.get(`/api/memos/${memo}`)).json()).status).toBe('complete');
});
