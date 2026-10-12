import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import net from 'node:net';

const base = 'http://127.0.0.1:3142';
// Public BIP-39 fixtures; no real account credentials are used.
const rootPhrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const memberPhrase = 'test test test test test test test test test test test junk';
const rootAddress = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const memberAddress = '0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266';
const picker = page => page.getByRole('combobox', {name:'Chat room', exact:true});
const prompt = page => page.getByRole('textbox', {name:'Chat message', exact:true});
const button = (page, name) => page.getByRole('button', {name, exact:true});
let server, dir;

test.beforeAll(async ({request}) => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-device-sync-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env:{...process.env, CWBBOT_ADMIN:rootAddress, BIND_ADDR:'127.0.0.1:3142', DATA_DIR:path.join(dir,'data'), CAUSEWAYBAY_CONFIG_DIR:dir,
      APP_TOKEN:'', XAI_API_KEY:'', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:''}, stdio:'ignore',
  });
  await expect.poll(async () => (await request.get(`${base}/api/health`).catch(() => null))?.status()).toBe(200);
});
test.afterAll(async () => {
  if (server?.exitCode === null && server.signalCode === null) {
    const exited = once(server, 'exit'); server.kill('SIGTERM'); await exited;
  }
  fs.rmSync(dir, {recursive:true, force:true});
});

async function observeSse(page) {
  await page.addInitScript(() => {
    const NativeEventSource = window.EventSource;
    window.deviceSync = {sources:[], events:[], opens:0};
    window.EventSource = class extends NativeEventSource {
      constructor(url, options) {
        super(url, options);
        if (url === '/api/notifications') {
          window.deviceSync.sources.push(this);
          this.addEventListener('open', () => window.deviceSync.opens++);
        }
        this.addEventListener('message', event => {
          window.deviceSync.events.push({url, data:JSON.parse(event.data)});
        });
      }
    };
  });
}
async function login(page, phrase = rootPhrase, url = base) {
  await page.goto(url);
  await page.getByLabel('Recovery phrase', {exact:true}).fill(phrase);
  await button(page, 'Sign in').click();
  await expect.poll(async () => (await (await page.request.get(`${base}/api/auth/info`)).json()).authenticated).toBe(true);
}
async function ready(page) {
  await expect(picker(page)).not.toHaveValue('');
  await expect(prompt(page)).toBeEnabled();
  await expect.poll(() => page.evaluate(() => window.deviceSync.sources.some(source => source.readyState === EventSource.OPEN))).toBe(true);
}
async function navigation(page) {
  const menu = button(page, 'Open navigation');
  if (await menu.isVisible() && await menu.getAttribute('aria-expanded') !== 'true') await menu.click();
}
async function fresh(page) {
  const old = await picker(page).inputValue();
  await navigation(page);
  await button(page, 'New thought').click();
  await expect(picker(page)).not.toHaveValue(old);
  await ready(page);
  return picker(page).inputValue();
}
async function send(page, text) {
  await prompt(page).fill(text);
  await button(page, 'Send message').click();
  await expect(page.locator('.from-user').last()).toContainText(text);
  await expect(button(page, 'Stop answer')).toHaveCount(0);
}
async function notice(page, kind, room) {
  await expect.poll(() => page.evaluate(({kind, room}) => window.deviceSync.events.some(event =>
    event.url === '/api/notifications' && event.data.kind === kind && event.data.data.roomid === room), {kind,room})).toBe(true);
}
async function secondDevice(browser, info) {
  const {viewport, userAgent, isMobile, hasTouch} = info.project.use;
  const context = await browser.newContext({viewport,userAgent,isMobile,hasTouch,reducedMotion:'reduce'});
  const page = await context.newPage();
  await observeSse(page);
  return {context, page};
}

// Drop the actual notification TCP stream while keeping the WebSocket alive.
// Browser offline emulation does not consistently close established SSE streams.
async function notificationProxy() {
  let blocked = false, muteReplay = false;
  const streams = new Set(), sockets = new Set();
  const proxy = http.createServer((request, response) => {
    const notification = request.url === '/api/notifications';
    if (notification && blocked) { response.destroy(); return; }
    const upstream = http.request({host:'127.0.0.1', port:3142, path:request.url, method:request.method, headers:request.headers}, incoming => {
      response.writeHead(incoming.statusCode, incoming.headers);
      if (notification && muteReplay) {
        // Reopen SSE without delivering historical events. The open handshake
        // itself must reconcile state; periodic room polling cannot hide a gap.
        response.flushHeaders(); incoming.resume();
        incoming.on('end', () => response.end());
      } else incoming.pipe(response);
      incoming.on('error', () => response.destroy());
    });
    if (notification) streams.add(response);
    response.on('close', () => {streams.delete(response); upstream.destroy();});
    upstream.on('error', () => response.destroy());
    request.pipe(upstream);
  });
  proxy.on('connection', socket => {sockets.add(socket); socket.on('close', () => sockets.delete(socket));});
  proxy.on('upgrade', (request, socket, head) => {
    const upstream = net.connect(3142, '127.0.0.1', () => {
      upstream.write(`${request.method} ${request.url} HTTP/${request.httpVersion}\r\n${Object.entries(request.headers).map(([key,value]) => `${key}: ${value}`).join('\r\n')}\r\n\r\n`);
      if (head.length) upstream.write(head);
      socket.pipe(upstream); upstream.pipe(socket);
    });
    socket.on('close', () => upstream.destroy()); socket.on('error', () => upstream.destroy());
    upstream.on('error', () => socket.destroy());
  });
  await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
  return {
    url:`http://127.0.0.1:${proxy.address().port}`,
    disconnect() {blocked = true; for (const response of streams) response.destroy();},
    resume() {blocked = false; muteReplay = true;},
    async close() {for (const socket of sockets) socket.destroy(); await new Promise(resolve => proxy.close(resolve));},
  };
}

test('same account follows New thought on both devices and receives messages through SSE', async ({page, browser}, info) => {
  const other = await secondDevice(browser, info);
  try {
    await observeSse(page);
    await login(page); await ready(page);
    await login(other.page); await ready(other.page);
    await expect(picker(other.page)).toHaveValue(await picker(page).inputValue());
    await prompt(other.page).fill('Old device draft');
    await button(other.page, 'Multi Capture').click();
    await expect(other.page.locator('.chat-attachment')).toHaveCount(1);
    const room = await fresh(page);
    await notice(other.page, 'room.thought', room);
    await expect(picker(other.page)).toHaveValue(room);
    await expect(prompt(other.page)).toHaveValue('');
    await expect(other.page.locator('.chat-attachment, .chat-message')).toHaveCount(0);
    await send(page, 'Question from device one');
    await expect(other.page.locator('.from-user')).toContainText('Question from device one');
    await expect.poll(() => other.page.evaluate(room => window.deviceSync.events.some(event =>
      event.url.includes(`/rooms/${room}/events`) && event.data.text === 'Question from device one'), room)).toBe(true);
    const next = await fresh(other.page);
    await notice(page, 'room.thought', next);
    await expect(picker(page)).toHaveValue(next);
    await expect(page.locator('.chat-message')).toHaveCount(0);
    await send(other.page, 'Question from device two');
    await expect(page.locator('.from-user')).toContainText('Question from device two');
  } finally { await other.context.close(); }
});

test('chat room creation and rename sync while selected conversations and drafts stay independent', async ({page, browser}, info) => {
  const other = await secondDevice(browser, info);
  try {
    await observeSse(page);
    await login(page); await ready(page);
    await login(other.page); await ready(other.page);
    const thought = await picker(other.page).inputValue();
    await prompt(other.page).fill('Keep this thought draft');
    await button(page, 'New chat').click();
    await expect(page).toHaveURL(`${base}/chat`); await ready(page);
    const room = await picker(page).inputValue();
    await notice(other.page, 'room.created', room);
    await expect(picker(other.page).locator(`option[value="${room}"]`)).toHaveCount(1);
    await expect(picker(other.page)).toHaveValue(thought);
    await expect(prompt(other.page)).toHaveValue('Keep this thought draft');
    await picker(other.page).selectOption(room);
    await send(page, 'Shared chat message');
    await expect(other.page.locator('.from-user')).toContainText('Shared chat message');
    expect((await page.request.patch(`${base}/api/rooms/${room}`, {data:{title:'Shared room renamed'}})).ok()).toBe(true);
    await notice(other.page, 'room.renamed', room);
    await expect(picker(other.page).locator('option:checked')).toHaveText('Shared room renamed');
    await prompt(other.page).fill('Keep this chat draft');
    const next = await fresh(page);
    await notice(other.page, 'room.thought', next);
    await expect(picker(other.page).locator(`option[value="${next}"]`)).toHaveCount(1);
    await expect(picker(other.page)).toHaveValue(room);
    await expect(prompt(other.page)).toHaveValue('Keep this chat draft');
    expect((await page.request.post(`${base}/api/rooms/${room}/clear`)).ok()).toBe(true);
    await notice(other.page, 'room.cleared', room);
    await expect(other.page.locator('.chat-message')).toHaveCount(0);
    await expect(prompt(other.page)).toHaveValue('');
    expect((await page.request.delete(`${base}/api/rooms/${room}`)).ok()).toBe(true);
    await expect(picker(other.page).locator(`option[value="${room}"]`)).toHaveCount(0);
  } finally { await other.context.close(); }
});

test('notification reconnect finds the latest thought after its creation event was erased', async ({page, browser}, info) => {
  const other = await secondDevice(browser, info);
  const proxy = await notificationProxy();
  try {
    await observeSse(page);
    await login(page); await ready(page);
    await login(other.page, rootPhrase, proxy.url); await ready(other.page);
    const previous = await picker(other.page).inputValue();
    const opened = await other.page.evaluate(() => window.deviceSync.opens);
    proxy.disconnect();
    await expect.poll(() => other.page.evaluate(() => window.deviceSync.sources.at(-1).readyState)).not.toBe(1);
    let latest;
    for (let i = 0; i < 3; i++) {
      const response = await page.request.post(`${base}/api/rooms/thought`, {data:{fresh:true}});
      expect(response.ok()).toBe(true); latest = (await response.json()).roomid;
    }
    expect((await page.request.post(`${base}/api/rooms/${latest}/clear`)).ok()).toBe(true);
    proxy.resume();
    await expect.poll(() => other.page.evaluate(() => window.deviceSync.opens)).toBeGreaterThan(opened);
    await expect(picker(other.page)).toHaveValue(latest);
    await expect(picker(page)).toHaveValue(latest);
    await prompt(other.page).fill('Preserve after replay');
    // Duplicate or delayed old notifications trigger a current-state read,
    // never a switch to the historical room referenced by the event.
    await other.page.evaluate(previous => window.deviceSync.sources.at(-1).dispatchEvent(new MessageEvent('message', {
      data:JSON.stringify({type:'notification', id:1, kind:'room.thought', data:{roomid:previous}}),
    })), previous);
    await expect(prompt(other.page)).toHaveValue('Preserve after replay');
    await send(other.page, 'Message after reconnect');
    await expect(page.locator('.from-user')).toContainText('Message after reconnect');
  } finally { await other.context.close(); await proxy.close(); }
});

test('logging in again resumes the shared thought without an extra reset', async ({page, browser}, info) => {
  const other = await secondDevice(browser, info);
  try {
    await observeSse(page);
    await login(page); await ready(page);
    await login(other.page); await ready(other.page);
    await fresh(other.page);
    await navigation(other.page); await button(other.page, 'Sign out').click();
    await expect(other.page.getByLabel('Recovery phrase', {exact:true})).toBeVisible();
    const current = await fresh(page);
    await send(page, 'Continue on another device');
    const before = await (await page.request.get(`${base}/api/rooms`)).json();
    // Sign in in-place: the app retains its earlier navigation state.
    await other.page.getByLabel('Recovery phrase', {exact:true}).fill(rootPhrase);
    await button(other.page, 'Sign in').click();
    await ready(other.page);
    await expect(picker(other.page)).toHaveValue(current);
    await expect(other.page.locator('.from-user')).toContainText('Continue on another device');
    expect((await (await page.request.get(`${base}/api/rooms`)).json()).length).toBe(before.length);
  } finally { await other.context.close(); }
});

test('a delayed current-thought read cannot restore an old room after a local reset', async ({page}) => {
  let holdNext = false;
  const pending = new Set(), held = [];
  await page.routeWebSocket('**/api/ws', socket => {
    const server = socket.connectToServer();
    socket.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if (holdNext && message.method === 'GET' && message.path === '/api/rooms/thought') {
        pending.add(message.id); holdNext = false;
      }
      server.send(raw);
    });
    server.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if (pending.delete(message.id)) held.push(() => socket.send(raw));
      else socket.send(raw);
    });
  });
  await observeSse(page);
  await login(page); await ready(page);
  holdNext = true;
  await page.evaluate(() => window.deviceSync.sources.at(-1).dispatchEvent(new MessageEvent('message', {
    data:JSON.stringify({type:'notification', id:1, kind:'room.thought', data:{roomid:'historical-notice'}}),
  })));
  await expect.poll(() => held.length).toBe(1);
  const current = await fresh(page);
  await prompt(page).fill('Fresh draft survives stale lookup');
  held.shift()();
  await expect(picker(page)).toHaveValue(current);
  await expect(prompt(page)).toHaveValue('Fresh draft survives stale lookup');
  await send(page, 'Submit after the stale lookup');
  await expect(picker(page)).toHaveValue(current);
});

test('late initial room lists still show every saved room after login', async ({page, browser}, info) => {
  const other = await secondDevice(browser, info);
  const proxy = await notificationProxy();
  proxy.resume(); // Suppress replay so notifications cannot mask a lost snapshot.
  const requests = new Map(), lists = [], thoughts = [];
  let holdThoughts = true;
  await other.page.routeWebSocket('**/api/ws', socket => {
    const server = socket.connectToServer();
    socket.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if (message.method === 'GET' && message.path === '/api/rooms') requests.set(message.id, lists);
      if (holdThoughts && message.path === '/api/rooms/thought') requests.set(message.id, thoughts);
      server.send(raw);
    });
    server.onMessage(raw => {
      const message = JSON.parse(String(raw));
      const queue = requests.get(message.id);
      if (queue) {queue.push(() => socket.send(raw)); requests.delete(message.id);}
      else socket.send(raw);
    });
  });
  try {
    await observeSse(page);
    await login(page); await ready(page);
    const saved = await (await page.request.post(`${base}/api/rooms`, {data:{title:'Saved before login'}})).json();
    const expected = await (await page.request.get(`${base}/api/rooms`)).json();
    await login(other.page, rootPhrase, proxy.url);
    await expect.poll(() => other.page.evaluate(() => window.deviceSync.opens)).toBeGreaterThan(0);
    await expect.poll(() => thoughts.length).toBeGreaterThan(0);
    await expect.poll(() => lists.length).toBeGreaterThan(0);
    const before = lists.length;
    holdThoughts = false;
    for (const release of thoughts) release();
    await ready(other.page);
    await expect.poll(() => lists.length).toBeGreaterThan(before);
    for (const release of lists.reverse()) release();
    await expect(picker(other.page).locator('option:not([value=""])')).toHaveCount(expected.length);
    await expect(picker(other.page).locator(`option[value="${saved.roomid}"]`)).toHaveText('Saved before login');
  } finally {await other.context.close(); await proxy.close();}
});

test('another account never receives the shared room list, current thought or room SSE', async ({page, browser}, info) => {
  const other = await secondDevice(browser, info);
  try {
    await observeSse(page);
    await login(page); await ready(page);
    await login(other.page, memberPhrase);
    await other.page.request.post(`${base}/api/auth/apply`, {data:{name:'Separate test account'}});
    expect((await page.request.patch(`${base}/api/admin/users/${memberAddress}`, {data:{role:'user'}})).ok()).toBe(true);
    await ready(other.page);
    const privateRoom = await picker(other.page).inputValue();
    const rootRoom = await fresh(page);
    await send(page, 'ROOT_PRIVATE_QUESTION');
    const rooms = await (await other.page.request.get(`${base}/api/rooms`)).json();
    expect(rooms.some(room => room.roomid === rootRoom)).toBe(false);
    expect((await (await other.page.request.get(`${base}/api/rooms/thought`)).json()).roomid).toBe(privateRoom);
    expect((await other.page.request.get(`${base}/api/rooms/${rootRoom}/events`)).ok()).toBe(false);
    await expect(picker(other.page)).toHaveValue(privateRoom);
    expect(await other.page.evaluate(room => window.deviceSync.events.some(event => event.data.data?.roomid === room || event.data.roomid === room), rootRoom)).toBe(false);
    await expect(other.page.locator('.chat-messages')).not.toContainText('ROOT_PRIVATE_QUESTION');
  } finally { await other.context.close(); }
});
