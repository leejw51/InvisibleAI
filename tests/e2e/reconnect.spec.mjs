import { observeEvents } from './events.mjs';
import { test, expect } from '@playwright/test';
import { spawn as spawnChild } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import net from 'node:net';

// A test that times out skips its finally block, so every process this file
// starts is also stopped after each test; a stray server would hold the port
// for the next run.
const children = new Set();
function spawn(...args) { const child = spawnChild(...args); children.add(child); return child; }
test.afterEach(async () => {
  for (const child of children) {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit');
      child.kill('SIGTERM');
      await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 3000))]);
      if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await exited; }
    }
  }
  children.clear();
});


test('browser replays SSE deltas after a severed HTTP stream without duplicating an answer', async ({ page, request }) => {
  let interrupted=false, replayed=false, memoId, requests=0;
  const sockets=new Set();
  const proxy=http.createServer((req,res)=>{
    if(req.url.includes('/events') && req.headers['last-event-id'])replayed=true;
    const upstream=http.request({host:'127.0.0.1',port:3099,path:req.url,method:req.method,headers:req.headers},response=>{
      res.writeHead(response.statusCode,response.headers);
      let cut=false;
      response.on('data',chunk=>{
        if(cut)return;
        res.write(chunk);
        if(!interrupted && req.url.includes('/events') && chunk.toString().includes('"kind":"delta"')){
          interrupted=true;cut=true;
          setTimeout(()=>{res.destroy();response.destroy();},30);
        }
      });
      response.on('end',()=>{if(!cut)res.end();});
      response.on('error',()=>res.destroy());
    });
    upstream.on('error',()=>res.destroy());req.pipe(upstream);
    res.on('close',()=>upstream.destroy());
  });
  proxy.on('connection',socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));});
  proxy.on('upgrade',(req,socket,head)=>{
    const upstream=net.connect(3099,'127.0.0.1',()=>{
      upstream.write(`${req.method} ${req.url} HTTP/${req.httpVersion}\r\n${Object.entries(req.headers).map(([k,v])=>`${k}: ${v}`).join('\r\n')}\r\n\r\n`);
      if(head.length)upstream.write(head);socket.pipe(upstream);upstream.pipe(socket);
    });
    socket.on('close',()=>upstream.destroy());socket.on('error',()=>upstream.destroy());upstream.on('error',()=>socket.destroy());
  });
  await new Promise(resolve=>proxy.listen(3117,'127.0.0.1',resolve));
  try{
    page.on('websocket', ws=>ws.on('framesent', frame=>{const v=JSON.parse(frame.payload);if(v.path==='/api/think')requests++;}));
    await observeEvents(page,(_,id)=>{memoId=id;});
    await page.goto('http://127.0.0.1:3117/overview');
    await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
    await expect(page.locator('.save-status')).toHaveText('Saved to your library');
    expect(interrupted).toBeTruthy();expect(replayed).toBeTruthy();expect(requests).toBe(1);
    const memo=await(await request.get(`/api/memos/${memoId}`)).json();
    expect(memo.status).toBe('complete');
    await expect(page.locator('.answer-content h3')).toHaveCount(1);
    await expect(page.locator('.answer-content code')).toContainText(['b','-','+','fn add','add(2, 3)']);
  }finally{for(const socket of sockets)socket.destroy();await new Promise(resolve=>proxy.close(resolve));}
});

test('Rust CLI restores saved ownership, sleeps between retries and reconnects after backend restart', async ({ request }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-retry-'));
  const backend = 'http://127.0.0.1:3112';
  const env = { ...process.env, CAUSEWAYBAY_CONFIG_DIR: dir, CAUSEWAYBAY_CLIENT_CONFIG_DIR: path.join(dir, 'client'), DATA_DIR: path.join(dir, 'data'), BIND_ADDR: '127.0.0.1:3112', SEMANTIC_SEARCH: 'off', DEMO_MODE: '1', APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', GEMINI_API_KEY: '', ANTHROPIC_API_KEY: '' };
  const owner = '0x1111111111111111111111111111111111111111';
  let log = '';
  const failureTimes = [];
  function startClient(args) {
    const child = spawn('./target/debug/rust-cli', args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let errors = '';
    child.stderr.on('data', data => {
      const text = data.toString();
      log += text;
      errors += text;
      const lines = errors.split('\n');
      errors = lines.pop();
      for (const line of lines) {
        if (line.startsWith('Connection failed:')) failureTimes.push(performance.now());
      }
    });
    child.stdout.on('data', data => { log += data.toString(); });
    return child;
  }
  let cli = startClient(['client', '--backend', backend, '--owner-wallet', owner, '--retry-seconds', '1']);
  let server;
  async function stop(child) {
    if (child && child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit'); child.kill('SIGTERM'); await exited;
    }
  }
  async function start() {
    server = spawn('./target/debug/causewaybay-server', [], { env, stdio: 'ignore' });
    await expect.poll(async () => {
      try { return (await request.get(`${backend}/api/health`)).ok(); } catch { return false; }
    }).toBeTruthy();
    await expect.poll(async () => (await (await request.get(`${backend}/api/clients`)).json()).some(c => c.online)).toBeTruthy();
  }
  try {
    await expect.poll(() => failureTimes.length).toBeGreaterThan(0);
    const configPath = path.join(env.CAUSEWAYBAY_CLIENT_CONFIG_DIR, 'client.jsonl');
    const saved = JSON.parse(fs.readFileSync(configPath, 'utf8').trim().split('\n').at(-1)).config;
    expect(saved).toMatchObject({ backend, owner_wallet: owner, retry_seconds: 1 });
    await stop(cli);
    const beforeResume = failureTimes.length;
    cli = startClient([]);
    await expect.poll(() => failureTimes.length, { intervals: [50] }).toBeGreaterThanOrEqual(beforeResume + 3);
    const attempts = failureTimes.slice(-3);
    // Allow delivery jitter while rejecting a busy loop; unit tests check the exact timer deadline.
    expect(attempts[1] - attempts[0]).toBeGreaterThanOrEqual(800);
    expect(attempts[2] - attempts[1]).toBeGreaterThanOrEqual(800);
    await start();
    const first = (await (await request.get(`${backend}/api/clients`)).json())[0];
    expect(first).toMatchObject({ id: saved.client_id, owner_wallet: owner, online: true });
    const failuresBeforeDisconnect = failureTimes.length;
    const disconnects = (log.match(/Connection (failed|closed)/g) || []).length;
    await stop(server);
    const connections = (log.match(/Connected\./g) || []).length;
    await expect.poll(() => (log.match(/Connection (failed|closed)/g) || []).length).toBeGreaterThan(disconnects);
    await expect.poll(() => failureTimes.length, { intervals: [50] }).toBeGreaterThanOrEqual(failuresBeforeDisconnect + 2);
    expect(failureTimes.at(-1) - failureTimes.at(-2)).toBeGreaterThanOrEqual(800);
    await start();
    await expect.poll(() => (log.match(/Connected\./g) || []).length).toBeGreaterThan(connections);
    const reconnected = (await (await request.get(`${backend}/api/clients`)).json())[0];
    expect(reconnected.id).toBe(first.id);
    expect(reconnected.online).toBeTruthy();
    expect(reconnected.owner_wallet).toBe(owner);
    expect(JSON.parse(fs.readFileSync(configPath, 'utf8').trim().split('\n').at(-1)).config).toEqual(saved);
  } finally { await stop(cli); await stop(server); }
});

test('Rust CLI exits promptly on Ctrl+C while sleeping before retry', async () => {
  const listener = net.createServer();
  await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
  const backend = `http://127.0.0.1:${listener.address().port}`;
  await new Promise(resolve => listener.close(resolve));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-retry-stop-'));
  fs.writeFileSync(path.join(dir, 'client.jsonl'), JSON.stringify({ version: 1, config: {
    backend, owner_wallet: null, client_id: 'retry-stop-node', name: 'Retry stop fixture', retry_seconds: 60,
  } }) + '\n');
  const cli = spawn('./target/debug/rust-cli', [], {
    env: { ...process.env, APP_TOKEN: '', CAUSEWAYBAY_CLIENT_CONFIG_DIR: dir },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let log = '';
  cli.stderr.on('data', data => { log += data.toString(); });
  try {
    await expect.poll(() => log).toContain('Connection failed:');
    cli.kill('SIGINT');
    await expect.poll(() => cli.exitCode, { timeout: 3000, intervals: [25, 50, 100] }).toBe(0);
    expect(cli.signalCode).toBeNull();
    expect((log.match(/Connection failed:/g) || []).length).toBe(1);
  } finally {
    if (cli.exitCode === null && cli.signalCode === null) {
      const exited = once(cli, 'exit');
      cli.kill('SIGTERM');
      await exited;
    }
  }
});
