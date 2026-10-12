import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import {DatabaseSync} from 'node:sqlite';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

async function stop(child) {
  if (child && child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit');
    child.kill('SIGTERM');
    await exited;
  }
}

for (const action of ['clear', 'delete', 'all']) {
  test(`room ${action} erases an offline Rust client's capture after backend restart`, async ({request}) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-client-erasure-'));
    const backend = 'http://127.0.0.1:3128';
    const clientDir = path.join(dir, 'client');
    const dataDir = path.join(dir, 'data');
    const env = {...process.env, CWBBOT_ADMIN:'', APP_TOKEN:'', DEMO_MODE:'1', SEMANTIC_SEARCH:'off',
      CAUSEWAYBAY_CONFIG_DIR:dir, CAUSEWAYBAY_CLIENT_CONFIG_DIR:clientDir,
      DATA_DIR:dataDir, BIND_ADDR:'127.0.0.1:3128',
      XAI_API_KEY:'', OPENAI_API_KEY:'', GEMINI_API_KEY:'', ANTHROPIC_API_KEY:''};
    let server, client, log = '';
    const startServer = async () => {
      server = spawn('./target/debug/causewaybay-server', [], {env, stdio:'ignore'});
      await expect.poll(async () => {
        try { return (await request.get(`${backend}/api/health`)).ok(); } catch { return false; }
      }).toBe(true);
    };
    const startClient = async args => {
      log = '';
      client = spawn('./target/debug/rust-cli', args, {env, stdio:['ignore','pipe','pipe']});
      client.stdout.on('data', data => { log += data; });
      client.stderr.on('data', data => { log += data; });
      await expect.poll(() => log).toContain('SSE callbacks connected.');
    };
    const createConversation = async title => {
      const response = await request.post(`${backend}/api/rooms`, {data:{title}});
      expect(response.ok()).toBe(true);
      const {roomid} = await response.json();
      const sent = await request.post(`${backend}/api/think`, {data:{room_id:roomid, message_id:randomUUID(), prompt:title}});
      expect(sent.ok()).toBe(true);
      const {id} = await sent.json();
      await expect.poll(async () => (await (await request.get(`${backend}/api/memos/${id}`)).json()).status).toBe('complete');
      return {roomid, memo:id};
    };
    try {
      await startServer();
      await startClient(['client', '--backend', backend, '--retry-seconds', '1']);
      const saved = JSON.parse(fs.readFileSync(path.join(clientDir, 'client.jsonl'), 'utf8').trim().split('\n').at(-1)).config;
      await stop(client);
      const erased = await createConversation('erase this room and its screenshot');
      const retained = await createConversation('retain unrelated room and screenshot');
      const erasedRequest = randomUUID(), retainedRequest = randomUUID();
      // Model captures saved before the device went offline, without recording a real desktop.
      // Purging, durable notification replay and filesystem deletion use the real processes.
      const db = new DatabaseSync(path.join(dataDir, 'causewaybay.sqlite3'));
      try {
        db.exec('PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;');
        const insert = db.prepare("INSERT INTO capture_requests(id,client_id,memo_id,status,mode,display,created_at) VALUES(?,?,?,'ready','screen',1,?)");
        insert.run(erasedRequest, saved.client_id, erased.memo, Date.now());
        insert.run(retainedRequest, saved.client_id, retained.memo, Date.now());
      } finally { db.close(); }
      const screenshots = path.join(clientDir, 'screenshots');
      fs.mkdirSync(screenshots, {recursive:true});
      const erasedPath = path.join(screenshots, `${erasedRequest}.png`);
      const retainedPath = path.join(screenshots, `${retainedRequest}.png`);
      const fixture = Buffer.from('synthetic screenshot fixture; no real screen content');
      fs.writeFileSync(erasedPath, fixture);
      fs.writeFileSync(retainedPath, fixture);
      const response = action === 'clear'
        ? await request.post(`${backend}/api/rooms/${erased.roomid}/clear`)
        : action === 'all' ? await request.delete(`${backend}/api/user/data`) : await request.delete(`${backend}/api/rooms/${erased.roomid}`);
      expect(response.ok()).toBe(true);
      expect((await request.get(`${backend}/api/memos/${erased.memo}`)).status()).toBe(404);
      expect((await request.get(`${backend}/api/memos/${retained.memo}`)).ok()).toBe(action !== 'all');
      expect(fs.existsSync(erasedPath)).toBe(true); // The device is still offline.
      await stop(server);
      await startServer();
      await startClient([]); // Restore the same node identity from its JSONL config.
      await expect.poll(() => fs.existsSync(erasedPath)).toBe(false);
      if(action === 'all') await expect.poll(() => fs.existsSync(retainedPath)).toBe(false);
      else expect(fs.readFileSync(retainedPath)).toEqual(fixture);
      expect(log).not.toContain('Connection failed:');
      // Replaying the same deletion on another connection is harmless.
      await stop(client);
      await startClient([]);
      expect(client.exitCode).toBeNull();
      expect(fs.existsSync(erasedPath)).toBe(false);
      if(action === 'all') await expect.poll(() => fs.existsSync(retainedPath)).toBe(false);
      else expect(fs.readFileSync(retainedPath)).toEqual(fixture);
      const rooms = await (await request.get(`${backend}/api/rooms`)).json();
      expect(rooms.some(room => room.roomid === erased.roomid)).toBe(action === 'clear');
    } finally {
      await stop(client);
      await stop(server);
      fs.rmSync(dir, {recursive:true, force:true});
    }
  });
}
