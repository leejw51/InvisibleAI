// Real screen capture and Apple OCR; deterministic loopback inference, no cloud.
import {test, expect, chromium} from '@playwright/test';
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {once} from 'node:events';
import http from 'node:http';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {openConversationOptions} from './workspace-controls.mjs';

const baseURL = 'http://127.0.0.1:3174';
test.use({baseURL});
test.setTimeout(90000);
let root, backend, client, displayBrowser, displayPage, inference;
let clientLog = '';
const requests = [];
const exec = promisify(execFile);
async function showTestScreen() {
  await displayPage.bringToFront();
  // Bringing a tab forward does not activate its macOS app. Activate this
  // dedicated test browser before waiting for the native Spaces animation.
  await exec('/usr/bin/open', ['-a', chromium.executablePath().split('.app/')[0]+'.app']);
  await displayPage.waitForTimeout(1500);
}
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const done = once(child, 'exit'); child.kill('SIGTERM'); await done;
}
async function patch(request, url, data) {
  const response = await request.patch(url, {data});
  expect(response.ok(), await response.text()).toBeTruthy();
}

test.beforeAll(async ({request}) => {
  test.skip(process.platform !== 'darwin' || process.env.RUN_NATIVE_SCREEN !== '1', 'Opt-in macOS screen capture');
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'invisible-native-screen-'));
  inference = http.createServer(async (req, res) => {
    let body = ''; for await (const part of req) body += part;
    if (req.url !== '/v1/chat/completions' || req.method !== 'POST') { res.writeHead(404).end(); return; }
    requests.push(JSON.parse(body));
    res.writeHead(200, {'content-type':'text/event-stream'});
    res.end('data: '+JSON.stringify({choices:[{delta:{content:'Capture test answer: 12.'},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n');
  });
  inference.listen(0, '127.0.0.1'); await once(inference, 'listening');
  const env = {...process.env, CWBBOT_ADMIN:'', APP_TOKEN:'', DEMO_MODE:'0', SEMANTIC_SEARCH:'off',
    BIND_ADDR:'127.0.0.1:3174', DATA_DIR:path.join(root,'data'), CAUSEWAYBAY_CONFIG_DIR:path.join(root,'backend'),
    CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(root,'client'),
    XAI_API_KEY:'', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:''};
  backend = spawn('./target/debug/causewaybay-server', ['serve','--no-semantic','--assets',path.resolve('dist')], {env,stdio:'ignore'});
  await expect.poll(async () => (await request.get('/api/health').catch(()=>null))?.status()).toBe(200);
  const policy = {mode:'ondevice',local_provider:'mlx',cloud_provider:'grok'};
  await patch(request, '/api/settings', {provider:'mlx',model:'capture-test',input_mode:'text',capture_mode:'screen',display:1,default_ai:policy,
    local_ai:{local_only:true,mlx_enabled:true,apple_enabled:false,mlx_url:`http://127.0.0.1:${inference.address().port}/v1`,speech_provider:'apple',speech_locale:'en-US',mlx_speech_model:'',auto_interval_seconds:60}});
  await patch(request, '/api/ai/preferences', policy);
  client = spawn('./target/debug/causewaybay', ['client','--backend',baseURL,'--client-id','native-screen','--name','Native screen test Mac'], {env,stdio:['ignore','pipe','pipe']});
  client.stdout.on('data', b => { clientLog += b; }); client.stderr.on('data', b => { clientLog += b; });
  await expect.poll(async () => (await (await request.get('/api/clients')).json()).some(c=>c.id==='native-screen'&&c.online)).toBe(true);
  displayBrowser = await chromium.launch({headless:false,args:['--kiosk','--start-fullscreen','--window-position=0,0']});
  displayPage = await displayBrowser.newPage({viewport:null,deviceScaleFactor:undefined,isMobile:undefined});
  await displayPage.setContent('<body style="margin:0;padding:24px;background:white;color:black;font:50px Arial"><h1 style="font:64px monospace;margin:0">SCREEN12345</h1><p>What is 7 + 5?</p><button onclick="document.documentElement.requestFullscreen()">Start test</button></body>');
  await displayPage.getByRole('button').click();
  await displayPage.waitForFunction(()=>document.fullscreenElement!==null);
  await showTestScreen();
});
test.afterAll(async () => {
  await displayBrowser?.close(); await stop(client); await stop(backend);
  if (inference) await new Promise(resolve => inference.close(resolve));
  if (root) await fs.rm(root,{recursive:true,force:true});
});
test.afterEach(async ({}, info) => {
  if (info.status !== info.expectedStatus) await info.attach('capture-client-log', {body:clientLog,contentType:'text/plain'});
});

test('Capture button reads the actual Mac display, reports an invalid display, and recovers', async ({page,request}) => {
  await page.goto('/');
  await page.getByRole('combobox',{name:'Active capture device'}).selectOption('native-screen');
  await openConversationOptions(page);
  await page.getByRole('button',{name:'Text mode',exact:true}).click();
  const capture = page.locator('.chat-dock').getByRole('button',{name:'Capture',exact:true});
  await showTestScreen();
  await capture.click();
  await expect(page.locator('.from-ai').last()).toContainText('Capture test answer: 12.',{timeout:45000});
  await expect(page.locator('.chat-image img').last()).toBeVisible();
  // Put the unique marker at the top so first-launch dialogs cannot cover it.
  expect(requests.some(r => r.messages.map(m=>m.content).join('\n').replace(/\s/g,'').includes('SCREEN12345')), 'OCR must see the controlled test screen').toBe(true);
  expect(clientLog).toContain('Screen capture permission:');
  expect(clientLog).toContain('Screenshot received by backend.');
  const before = requests.length;
  await patch(request, '/api/settings', {display:16});
  await page.reload();
  await capture.click();
  await expect(page.locator('.from-ai').last()).toContainText('Display 16 is unavailable on the capture Mac',{timeout:20000});
  await expect(page.locator('.from-ai .chat-message-meta').last()).toHaveText('Capture failedNo AI response');
  await expect(page.locator('.from-user .login-error')).toHaveCount(0);
  expect(requests).toHaveLength(before);
  await patch(request, '/api/settings', {display:1});
  await page.reload();
  await showTestScreen();
  await capture.click();
  await expect(page.locator('.from-ai').last()).toContainText('Capture test answer: 12.',{timeout:45000});
  await expect.poll(()=>requests.length).toBe(before+1);
  await page.screenshot({path:test.info().outputPath('recovered-capture.png'),fullPage:true});
});

test('Cloud polling captures a changed screen automatically and discards unchanged polls', async ({page,request}) => {
  const room = (await (await request.post('/api/rooms', {data:{title:'Native automatic polling'}})).json()).roomid;
  const status = async () => (await request.get(`/api/rooms/${room}/capture`)).json();
  const messages = async () => (await (await request.get(`/api/rooms/${room}/sync`)).json()).messages.filter(m=>!m.deleted);
  try {
    await displayPage.locator('h1').evaluate(element => {element.textContent='POLLFRAME111';});
    await page.goto('/autonomous');
    const card = page.locator(`[data-session-id="${room}"]`);
    await card.getByTestId('manual-capture').locator('> summary').click();
    await card.getByRole('combobox',{name:'Capture computer',exact:true}).selectOption('native-screen');
    await card.getByRole('spinbutton',{name:'Capture interval',exact:true}).fill('2');
    await showTestScreen();
    await card.getByRole('button',{name:'Start polling',exact:true}).click();
    await expect(card.getByRole('button',{name:'Stop polling',exact:true})).toBeVisible();
    await expect.poll(async () => (await messages()).filter(m=>m.role==='assistant'&&m.status==='complete').length,{timeout:45000}).toBe(1);
    expect(requests.some(r=>r.messages.map(m=>m.content).join('\n').replace(/\s/g,'').includes('POLLFRAME111'))).toBe(true);
    // Change only the displayed content. Do not click Capture a second time.
    await displayPage.locator('h1').evaluate(element => {element.textContent='POLLFRAME222';});
    await page.goto('/wiki'); // The backend must keep polling without Cloud open.
    await expect.poll(async () => (await messages()).filter(m=>m.role==='assistant'&&m.status==='complete').length,{timeout:45000}).toBe(2);
    expect(requests.some(r=>r.messages.map(m=>m.content).join('\n').replace(/\s/g,'').includes('POLLFRAME222'))).toBe(true);
    await expect.poll(async () => (await status()).unchanged_count,{timeout:20000}).toBeGreaterThan(0);
    expect((await messages()).filter(m=>m.role==='user')).toHaveLength(2);
    await page.goto('/autonomous');
    await expect(card.locator('.cloud-capture-status')).toContainText('Unchanged screenshots discarded');
    await card.getByRole('button',{name:'Stop polling',exact:true}).click();
    expect((await status()).active).toBe(false);
    await page.screenshot({path:test.info().outputPath('automatic-polling.png'),fullPage:true});
  } finally {await request.delete(`/api/rooms/${room}/capture`);}
});
