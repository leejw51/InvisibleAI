import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

test('default backend folder owns saved data across restarts and stays out of settings', async ({page, request}) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-default-storage-'));
  const isolatedHome = path.join(fixture, 'user');
  const root = path.join(isolatedHome, '.causewaybaybot');
  const cwd = path.join(fixture, 'launch');
  fs.mkdirSync(cwd);
  const base = 'http://127.0.0.1:3159';
  // Only the fixture child receives a synthetic home; the developer's home is untouched.
  const env = {...process.env, HOME:isolatedHome, CWBBOT_ADMIN:'', APP_TOKEN:'',
    BIND_ADDR:'127.0.0.1:3159', SEMANTIC_SEARCH:'off', DEMO_MODE:'1',
    XAI_API_KEY:'', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:''};
  delete env.DATA_DIR; delete env.CAUSEWAYBAY_CONFIG_DIR;
  let server;
  async function start() {
    server = spawn(path.resolve('target/debug/causewaybay-server'),
      ['serve', '--assets', path.resolve('dist')], {cwd, env, stdio:'ignore'});
    await expect.poll(async () => (await request.get(base+'/api/health').catch(() => null))?.status()).toBe(200);
  }
  async function stop() {
    if (server && server.exitCode === null && server.signalCode === null) {
      const done = once(server, 'exit'); server.kill('SIGTERM'); await done;
    }
  }
  try {
    await start();
    expect(fs.existsSync(path.join(root, 'data/causewaybay.sqlite3'))).toBe(true);
    expect((await request.patch(base+'/api/assistance/preferences', {data:{wiki_enabled:true}})).ok()).toBe(true);
    const note = await (await request.post(base+'/api/memos', {data:{title:'Saved here', content:'A persistent thought #storage'}})).json();
    const contents = Buffer.from('A private attachment');
    const file = await (await request.post(base+'/api/files', {data:{name:'kept.txt', mime:'text/plain', data_base64:contents.toString('base64')}})).json();
    // Text lives in SQLite only: a text file has no path on disk and is served
    // from the database; memos and Wiki pages are never mirrored as Markdown.
    expect(file.path).toBe('');
    expect(await (await request.get(base+`/api/files/${file.id}/download`)).body()).toEqual(contents);
    expect((await (await request.get(base+`/api/files/${file.id}/text`)).json()).text).toBe('A private attachment');
    expect((await (await request.get(base+`/api/memos/${note.id}`)).json()).input).toContain('persistent thought');
    expect(fs.existsSync(path.join(root, 'data/markdown'))).toBe(false);
    expect(fs.existsSync(path.join(root, 'data/wiki'))).toBe(false);
    expect(fs.existsSync(path.join(root, 'data/files'))).toBe(false);
    expect((await request.patch(base+'/api/settings', {data:{display:2}})).ok()).toBe(true);
    for (const field of ['data_dir', 'config_dir', 'storage_path']) {
      expect((await request.patch(base+'/api/settings', {data:{[field]:'/outside'}})).ok()).toBe(false);
    }
    const saved = JSON.parse(fs.readFileSync(path.join(root, 'backend.jsonl'), 'utf8').trim().split('\n').at(-1)).config;
    expect(saved).not.toHaveProperty('data_dir');
    expect(saved).not.toHaveProperty('config_dir');
    expect(fs.readdirSync(cwd)).toEqual([]);
    if (process.platform !== 'win32') expect(fs.statSync(root).mode & 0o777).toBe(0o700);
    await stop(); await start();
    expect((await (await request.get(base+'/api/settings')).json()).display).toBe(2);
    expect((await (await request.get(base+`/api/files/${file.id}/download`)).body())).toEqual(contents);
    await page.goto(base+'/wiki');
    await expect(page.locator('.wiki-card .wiki-tag').filter({hasText:'#storage'})).toBeVisible();
    await page.getByRole('button', {name:'Open settings', exact:true}).click();
    await expect(page.getByRole('dialog').getByLabel(/^(Data directory|Storage path|Config directory)$/)).toHaveCount(0);
    expect(fs.readdirSync(cwd)).toEqual([]);
  } finally { await stop(); fs.rmSync(fixture, {recursive:true, force:true}); }
});
