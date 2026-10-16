import { test, expect } from '@playwright/test';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

async function stop(child){if(child && child.exitCode===null && child.signalCode===null){const done=once(child,'exit');child.kill('SIGTERM');await done;}}

test('rust-cli without saved settings shows connection instructions without connecting',()=>{
  const home=fs.mkdtempSync(path.join(os.tmpdir(),'causewaybay-help-'));
  const help=execFileSync('./target/debug/rust-cli',[],{encoding:'utf8',timeout:5000,env:{...process.env,CAUSEWAYBAY_CLIENT_CONFIG_DIR:home}});
  expect(help).toContain('rust-cli http://backend.example:3000');
  expect(help).toContain('client');
  expect(help).toContain('--help');
  expect(fs.readdirSync(home)).toEqual([]);
});

for (const owner of [null, '0x1111111111111111111111111111111111111111']) {
  test(`no-argument restart restores saved URL and ${owner ? 'owner wallet' : 'shared access'}`, async ({ request }) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-resume-'));
    const backend = 'http://127.0.0.1:3099';
    const env = { ...process.env, CAUSEWAYBAY_CLIENT_CONFIG_DIR: dir, CAUSEWAYBAY_BACKEND: 'http://127.0.0.1:1', APP_TOKEN: '' };
    const config = () => JSON.parse(fs.readFileSync(path.join(dir, 'client.jsonl'), 'utf8').trim().split('\n').at(-1)).config;
    const clients = async () => (await (await request.get('/api/clients')).json());
    let child = spawn('./target/debug/rust_cli', owner ? [backend, owner] : [backend], { env, stdio: 'ignore' });
    try {
      await expect.poll(() => fs.existsSync(path.join(dir, 'client.jsonl'))).toBe(true);
      const saved = config();
      expect(saved.backend).toBe(backend);
      expect(saved.owner_wallet).toBe(owner);
      await expect.poll(async () => (await clients()).find(c => c.id === saved.client_id)?.online).toBe(true);
      await stop(child);
      await expect.poll(async () => (await clients()).find(c => c.id === saved.client_id)?.online).toBe(false);
      const beforeHelp = fs.readFileSync(path.join(dir, 'client.jsonl'), 'utf8');
      expect(execFileSync('./target/debug/rust-cli', ['--help'], { env, encoding: 'utf8', timeout: 5000 })).toContain('saved URL and owner wallet');
      expect(fs.readFileSync(path.join(dir, 'client.jsonl'), 'utf8')).toBe(beforeHelp);
      child = spawn('./target/debug/rust-cli', [], { env, stdio: 'ignore' });
      await expect.poll(async () => {
        const node = (await clients()).find(c => c.id === saved.client_id);
        return node?.online ? node.owner_wallet : 'offline';
      }).toBe(owner);
      expect(config()).toEqual(saved);
    } finally {
      await stop(child);
    }
  });
}

test('URL shortcut uses computer name; dashboard rename reaches CLI JSONL and survives reconnect',async({page,request})=>{
  const home=fs.mkdtempSync(path.join(os.tmpdir(),'causewaybay-shortcut-'));
  const env={...process.env,CAUSEWAYBAY_CLIENT_CONFIG_DIR:home,APP_TOKEN:''};
  let log='',child;
  const start=(args=['http://127.0.0.1:3099'])=>{child=spawn('./target/debug/rust-cli',args,{env,stdio:['ignore','pipe','pipe']});child.stdout.on('data',b=>{log+=b;});child.stderr.on('data',b=>{log+=b;});};
  // The CLI may be mid-write; a half-written last line reads as "not yet" so expect.poll keeps polling.
  const config=()=>{try{return JSON.parse(fs.readFileSync(path.join(home,'client.jsonl'),'utf8').trim().split('\n').at(-1)).config;}catch{return undefined;}};
  try{
    start();
    await expect.poll(()=>log).toContain('SSE callbacks connected.');
    const initial=config();
    expect(initial.backend).toBe('http://127.0.0.1:3099');
    if(process.platform==='darwin')expect(initial.name).toBe(execFileSync('/usr/sbin/scutil',['--get','ComputerName'],{encoding:'utf8'}).trim());
    await page.goto('/devices');
    const row=page.locator(`[data-client-id="${initial.client_id}"]`);
    await expect(row).toContainText(initial.name);
    await row.getByRole('button',{name:`Rename ${initial.name}`,exact:true}).click();
    await page.getByRole('textbox',{name:'Computer name',exact:true}).fill('My renamed capture computer');
    await page.getByRole('button',{name:'Save computer name',exact:true}).click();
    await expect(row).toContainText('My renamed capture computer');
    await expect.poll(()=>config()?.name).toBe('My renamed capture computer');
    await expect.poll(()=>log).toContain('Computer name updated: My renamed capture computer');
    await stop(child);
    await expect.poll(async()=> (await (await request.get('/api/clients')).json()).find(c=>c.id===initial.client_id)?.online).toBe(false);
    start([]);
    await expect.poll(async()=> (await (await request.get('/api/clients')).json()).find(c=>c.id===initial.client_id)?.online).toBe(true);
    expect(config().client_id).toBe(initial.client_id);
    expect(config().name).toBe('My renamed capture computer');
    await expect(row).toContainText('Connected');
  }finally{await stop(child);}
});

test('capture scope supports a selected subset and all connected computers',async({page,request})=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'causewaybay-many-'));
  const children=['Desk','Laptop','Studio'].map(name=>spawn('./target/debug/causewaybay',['client','--backend','http://127.0.0.1:3099','--name',name,'--client-id',`multi-${name}`],{env:{...process.env,CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(root,name),APP_TOKEN:''},stdio:'ignore'}));
  try{
    await expect.poll(async()=> (await (await request.get('/api/clients')).json()).filter(c=>c.id.startsWith('multi-')&&c.online).length).toBe(3);
    await page.goto('/overview');
    await page.getByRole('combobox',{name:'Capture client'}).selectOption('multiple');
    await page.getByRole('checkbox',{name:'Desk',exact:true}).check();
    await page.getByRole('checkbox',{name:'Laptop',exact:true}).check();
    await page.reload();
    await expect(page.getByRole('combobox',{name:'Capture client'})).toHaveValue('multiple');
    await expect(page.getByRole('checkbox',{name:'Desk',exact:true})).toBeChecked();
    await expect(page.getByRole('checkbox',{name:'Laptop',exact:true})).toBeChecked();
    await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
    await expect(page.getByTestId('capture-result')).toHaveCount(2);
    await expect(page.locator('.batch-progress')).toHaveCount(0);
    expect((await page.getByTestId('capture-result').locator('header h2').allTextContents()).sort()).toEqual(['Desk','Laptop']);
    await page.getByRole('button',{name:'Multi-device capture',exact:true}).click();
    await page.getByRole('combobox',{name:'Capture client'}).selectOption('all');
    await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
    await expect(page.getByTestId('capture-result')).toHaveCount(3);
    await expect(page.locator('.batch-progress')).toHaveCount(0);
    for(const name of ['Desk','Laptop','Studio'])await expect(page.getByTestId('capture-result').getByRole('heading',{name,exact:true})).toBeVisible();
    await page.screenshot({path:'test-results/multiple-clients.png',fullPage:true});
  }finally{await Promise.all(children.map(stop));}
});
