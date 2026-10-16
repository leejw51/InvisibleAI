import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {openNav, openConversationOptions, openSettings} from './workspace-controls.mjs';
const base = 'http://127.0.0.1:3123';
// Public BIP-39 test vectors; never use these wallets for assets.
const rootPhrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const memberPhrase = 'test test test test test test test test test test test junk';
const rootAddress = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const memberAddress = '0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266';
let server, dir;
test.beforeAll(async ({ request }) => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: rootAddress, BIND_ADDR: '127.0.0.1:3123', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore'
  });
  await expect.poll(async () => (await request.get(base+'/api/health').catch(()=>null))?.status()).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });
async function login(page, phrase) {
  await page.goto(base);
  await page.getByLabel('Recovery phrase').fill(phrase);
  await page.getByRole('button', { name:'Sign in', exact:true }).click();
  await expect.poll(async()=> (await(await page.request.get(base+'/api/auth/info')).json()).authenticated).toBe(true);
}
async function wsCall(page, method, route, body) {
  return page.evaluate(({method,route,body}) => new Promise((resolve,reject) => {
    const socket = new WebSocket(location.origin.replace('http','ws')+'/api/ws');
    const timeout=setTimeout(()=>{socket.close();reject(new Error('WS timeout'));},5000);
    socket.onopen=()=>socket.send(JSON.stringify({type:'hello',id:'test-auth-'+Math.random(),name:'Auth test',platform:'test'}));
    socket.onmessage=e=>{const v=JSON.parse(e.data);if(v.type==='ready')socket.send(JSON.stringify({type:'request',id:1,method,path:route,body}));if(v.type==='response'){clearTimeout(timeout);socket.close();resolve(v);}};
  }), {method,route,body});
}
test('mnemonic login, waiting list, admin approval and promotion, revocation, and privacy', async ({ browser, request }) => {
  test.setTimeout(90000);
  const adminCtx=await browser.newContext(), memberCtx=await browser.newContext();
  const admin=await adminCtx.newPage(), member=await memberCtx.newPage();
  const payloads=[];const errors=[];
  for(const p of [admin,member]) {p.on('request',r=>{if(r.postData())payloads.push(r.postData());});p.on('pageerror',e=>errors.push(e.message));}
  expect((await request.get(base+'/api/files')).status()).toBe(401);
  await login(admin,rootPhrase);
  await openSettings(admin);await expect(admin.locator('[data-nav="admin"]')).toBeVisible();
  await login(member,memberPhrase);
  await expect(member.getByRole('heading',{name:'You’re on the waiting list.'})).toBeVisible();
  expect((await memberCtx.request.get(base+'/api/rooms')).status()).toBe(403);
  await expect(member.locator('[data-nav="admin"]')).toHaveCount(0);
  const name=await member.getByLabel('Your name',{exact:true}).textContent();
  await expect(member.getByRole('heading',{name:'You’re on the waiting list.'})).toBeVisible();
  await member.reload();
  await expect(member.getByRole('heading',{name:'You’re on the waiting list.'})).toBeVisible();
  await openNav(admin,'admin');
  const row=admin.locator(`[data-address="${memberAddress}"]`);
  await expect(row).toContainText(name);
  await expect(admin.getByRole('heading',{name:'Waiting for approval · 1'})).toBeVisible();
  await admin.screenshot({path:'test-results/admin-waiting.png',fullPage:true});
  await row.getByRole('button',{name:'Approve user',exact:true}).click();
  await expect(member.getByRole('heading',{name:'Hey, what’s on your mind?',exact:true})).toBeVisible();
  await expect(member.locator('[data-nav="admin"]')).toHaveCount(0);
  await expect(member.getByRole('button',{name:'Open settings'})).toHaveCount(0);
  await openNav(member,'devices');
  await expect(member.getByLabel('Capture client command')).toHaveText(`./bin/client ${base} ${memberAddress}`);
  await expect(member.getByRole('group',{name:'Tailscale DNS',exact:true})).toHaveCount(0);
  await member.getByRole('link',{name:'Return to New thought'}).click();
  await expect(member.getByRole('heading',{name:'Hey, what’s on your mind?',exact:true})).toBeVisible();
  expect((await memberCtx.request.patch(base+'/api/settings',{data:{display:2}})).status()).toBe(403);
  expect((await wsCall(member,'PATCH','/api/settings',{display:2})).status).toBe(403);
  expect((await wsCall(member,'GET','/api/admin/users',null)).status).toBe(403);
  await row.getByRole('button',{name:'Make admin'}).click();
  await openSettings(member);await expect(member.locator('[data-nav="admin"]')).toBeVisible();
  expect((await wsCall(member,'PATCH','/api/settings',{display:1})).status).toBe(200);
  expect((await memberCtx.request.patch(base+`/api/admin/users/${rootAddress}`,{data:{role:'blocked'}})).status()).toBe(403);
  await row.getByRole('button',{name:'Make user'}).click();
  await expect(member.locator('[data-nav="admin"]')).toHaveCount(0);
  await row.getByRole('button',{name:'Block access'}).click();
  await expect(member.getByRole('heading',{name:'Access is unavailable.'})).toBeVisible();
  expect((await memberCtx.request.get(base+'/api/files')).status()).toBe(403);
  for(const p of [admin,member]) {
    // A phrase sign-in keeps the phrase only inside the wallet entry (a6be17f); it must appear nowhere else.
    const local=await p.evaluate(()=>JSON.stringify(Object.fromEntries(Object.entries(localStorage).filter(([k])=>k!=='invisible-ai.wallet.key'))));
    expect(local).not.toContain(rootPhrase);expect(local).not.toContain(memberPhrase);
    expect(await p.evaluate(()=>document.cookie)).not.toContain('cwb_session');
  }
  expect(payloads.join('\n')).not.toContain(rootPhrase);expect(payloads.join('\n')).not.toContain(memberPhrase);
  const records=fs.readFileSync(path.join(dir,'users.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
  expect(records.at(-1).config.users[0].role).toBe('blocked');
  expect(records.at(-1).config.actor).toBe(rootAddress);
  await admin.getByRole('button',{name:'Sign out',exact:true}).click();
await admin.getByRole('button',{name:'Confirm sign out',exact:true}).click();
  await expect(admin.getByLabel('Recovery phrase')).toBeVisible();
  expect((await adminCtx.request.get(base+'/api/settings')).status()).toBe(401);
  expect(errors).toEqual([]);
  await adminCtx.close();await memberCtx.close();
});

test('CLI URL plus owner wallet persists ownership; shared URL permits other approved users',async({browser,request})=>{
  const adminCtx=await browser.newContext(), memberCtx=await browser.newContext();
  const admin=await adminCtx.newPage(),member=await memberCtx.newPage();
  await login(admin,rootPhrase);await openSettings(admin);await expect(admin.locator('[data-nav="admin"]')).toBeVisible();
  await login(member,memberPhrase);
  // This test also works on its own, before the workflow test has registered the wallet.
  await memberCtx.request.post(base+'/api/auth/apply',{data:{name:'MacBook owner'}});
  expect((await adminCtx.request.patch(base+`/api/admin/users/${memberAddress}`,{data:{role:'user'}})).ok()).toBeTruthy();
  const ownDir=fs.mkdtempSync(path.join(os.tmpdir(),'cwb-owned-node-'));
  const sharedDir=fs.mkdtempSync(path.join(os.tmpdir(),'cwb-shared-node-'));
  const owner=spawn('./target/debug/rust_cli',[base,memberAddress],{env:{...process.env,APP_TOKEN:'',CAUSEWAYBAY_CLIENT_CONFIG_DIR:ownDir},stdio:'ignore'});
  const shared=spawn('./target/debug/rust_cli',[base],{env:{...process.env,APP_TOKEN:'',CAUSEWAYBAY_CLIENT_CONFIG_DIR:sharedDir},stdio:'ignore'});
  try {
    await expect.poll(async()=> (await (await memberCtx.request.get(base+'/api/clients')).json()).filter(c=>c.online).length).toBe(2);
    const clients=await (await memberCtx.request.get(base+'/api/clients')).json();
    const privateNode=clients.find(c=>c.owner_wallet===memberAddress), publicNode=clients.find(c=>c.owner_wallet===null);
    expect(privateNode.can_capture).toBe(true);expect((await (await adminCtx.request.get(base+'/api/clients')).json()).some(c=>c.id===privateNode.id)).toBe(false);expect(publicNode.can_capture).toBe(true);
    expect(JSON.parse(fs.readFileSync(path.join(ownDir,'client.jsonl'),'utf8').trim().split('\n').at(-1)).config.owner_wallet).toBe(memberAddress);
    expect(JSON.parse(fs.readFileSync(path.join(sharedDir,'client.jsonl'),'utf8').trim().split('\n').at(-1)).config.owner_wallet).toBeNull();
    expect((await adminCtx.request.post(base+'/api/think',{data:{capture:true,client_id:privateNode.id}})).ok()).toBe(false);
    expect((await memberCtx.request.post(base+'/api/think',{data:{capture:true,client_id:privateNode.id}})).ok()).toBe(true);
    const all=await (await adminCtx.request.post(base+'/api/capture/batch',{data:{all:true,request:{input_mode:'text'}}})).json();
    expect(all.map(x=>x.client.id)).toEqual([publicNode.id]);
    const room=await (await adminCtx.request.post(base+'/api/rooms',{data:{}})).json();
    expect((await adminCtx.request.post(base+`/api/rooms/${room.roomid}/capture`,{data:{client_ids:[privateNode.id],interval_seconds:2,request:{}}})).ok()).toBe(false);
    // An ordinary user can designate their own capture node without admin rights.
    await member.goto(base+'/devices');
    await member.locator(`[data-client-id="${privateNode.id}"]`).getByRole('button',{name:`Use ${privateNode.name} as default`,exact:true}).click();
    await openNav(member,'home'); await member.getByRole('button',{name:'New chat',exact:true}).click();
    await expect(member.getByRole('combobox',{name:'Active capture device'})).toHaveValue(privateNode.id);
    const ownerKey=`causewaybay.capture.v1:${memberAddress}`;
    expect(await member.evaluate(key=>JSON.parse(localStorage.getItem(key)).default,ownerKey)).toBe(privateNode.id);
    await member.evaluate(({key,id})=>localStorage.setItem(key,JSON.stringify({active:id,default:id})),{key:`causewaybay.capture.v1:${rootAddress}`,id:publicNode.id});
    await member.getByRole('button',{name:'Sign out',exact:true}).click();
await member.getByRole('button',{name:'Confirm sign out',exact:true}).click();
    await expect(member.getByLabel('Recovery phrase')).toBeVisible();
    await login(member,rootPhrase);
    await expect(member.getByRole('combobox',{name:'Active capture device'})).toHaveValue(publicNode.id);
    expect(await member.evaluate(key=>JSON.parse(localStorage.getItem(key)).default,ownerKey)).toBe(privateNode.id);

  } finally {owner.kill('SIGTERM');shared.kill('SIGTERM');await adminCtx.close();await memberCtx.close();}
});

test('language is available before wallet login and remains selected after logout', async ({page}) => {
  await page.goto(base);
  await page.getByRole('button',{name:'Switch to dark mode'}).click();
  await page.getByTestId('language-select').selectOption('ko');
  await expect(page.getByRole('heading',{name:'내 공간에 오신 것을 환영합니다.'})).toBeVisible();
  await page.getByLabel('복구 구문',{exact:true}).fill(rootPhrase);
  // Changing UI language must not remount the login form or discard its input.
  await page.getByTestId('language-select').selectOption('ja');
  await expect(page.getByLabel('リカバリーフレーズ',{exact:true})).toHaveValue(rootPhrase);
  await page.getByRole('button',{name:'ログイン',exact:true}).click();
  await expect(page.getByRole('heading',{name:'ねえ、何を考えてる？',exact:true})).toBeVisible();
  await openNav(page,'admin');
  await expect(page.getByRole('heading',{name:'スペースへのアクセス管理。'})).toBeVisible();
  await page.reload();
  await expect(page.getByTestId('language-select')).toHaveValue('ja');
  await page.getByRole('button',{name:'ログアウト',exact:true}).click();
  // The confirmation follows the language too.
  await page.getByRole('button',{name:'ログアウトを確認',exact:true}).click();
  await expect(page.getByLabel('リカバリーフレーズ',{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>localStorage.getItem('causewaybay.language.v1'))).toBe('ja');
  await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
  await page.screenshot({path:'test-results/login-dark.png',fullPage:true});
});

for (const action of ['clear','delete','message','all']) {
  test(`later wallet login reconciles ${action} and discovers newly synced rooms`,async({browser})=>{
    const activeContext=await browser.newContext(), laterContext=await browser.newContext();
    const active=await activeContext.newPage(), later=await laterContext.newPage();
    const secret=`signed-out-private-${action}-${Date.now()}`;
    const cached=()=>later.evaluate(owner=>new Promise((resolve,reject)=>{
      const request=indexedDB.open(`causewaybay-messages-${owner}`);
      request.onerror=()=>reject(request.error);
      request.onsuccess=()=>{const db=request.result;if(!db.objectStoreNames.contains('messages')){db.close();return resolve([]);}
        const all=db.transaction('messages').objectStore('messages').getAll();
        all.onsuccess=()=>{resolve(all.result.map(v=>{try{return JSON.parse(v)}catch{return null}}).filter(v=>v&&typeof v==='object'));db.close();};};
    }),rootAddress);
    const send=async(room,prompt)=>{
      const result=await active.request.post(base+'/api/think',{data:{room_id:room,message_id:crypto.randomUUID(),prompt}});
      expect(result.ok()).toBe(true);const memo=await result.json();
      await expect.poll(async()=>(await(await active.request.get(base+`/api/memos/${memo.id}`)).json()).status).toBe('complete');
    };
    try {
      await login(active,rootPhrase);await login(later,rootPhrase);
      const room=await(await active.request.post(base+'/api/rooms',{data:{title:secret}})).json();
      await send(room.roomid,secret);
      await later.goto(base+'/chat');await openConversationOptions(later);await later.getByLabel('Chat room',{exact:true}).selectOption(room.roomid);
      await expect(later.locator('.from-user')).toContainText(secret);
      await expect.poll(async()=>(await cached()).some(m=>m.text===secret)).toBe(true);
      await later.getByRole('button',{name:'Sign out',exact:true}).click();
await later.getByRole('button',{name:'Confirm sign out',exact:true}).click();
      await expect(later.getByLabel('Recovery phrase')).toBeVisible();
      let response;
      if(action==='clear')response=await active.request.post(base+`/api/rooms/${room.roomid}/clear`);
      else if(action==='delete')response=await active.request.delete(base+`/api/rooms/${room.roomid}`);
      else if(action==='all')response=await active.request.delete(base+'/api/user/data');
      else {
        const sync=await(await active.request.get(base+`/api/rooms/${room.roomid}/sync`)).json();
        const message=sync.messages.find(m=>m.role==='user'&&!m.deleted);
        response=await active.request.delete(base+`/api/rooms/${room.roomid}/messages/${message.messageid}`,{data:{messagesyncid:message.messagesyncid}});
      }
      expect(response.ok()).toBe(true);
      const fresh=await(await active.request.post(base+'/api/rooms',{data:{title:`New while signed out ${action}`}})).json();
      await send(fresh.roomid,`Fresh sync ${action}`);
      await login(later,rootPhrase);
      await expect.poll(async()=>(await cached()).some(m=>m.text===secret)).toBe(false);
      await later.goto(base+'/chat');
      await openConversationOptions(later);
      await expect(later.getByLabel('Chat room',{exact:true}).locator(`option[value="${fresh.roomid}"]`)).toHaveCount(1);
      if(action==='clear'||action==='message') {
        await openConversationOptions(later);
        await later.getByLabel('Chat room',{exact:true}).selectOption(room.roomid);
        await expect(later.locator('.chat-messages')).not.toContainText(secret);
      await openConversationOptions(later);
      } else await expect(later.getByLabel('Chat room',{exact:true}).locator(`option[value="${room.roomid}"]`)).toHaveCount(0);
      await openConversationOptions(later);
      await later.getByLabel('Chat room',{exact:true}).selectOption(fresh.roomid);
      await expect(later.locator('.from-user')).toContainText(`Fresh sync ${action}`);
    } finally {await activeContext.close();await laterContext.close();}
  });
}

test('indexed wallet names, copy-only recovery, reload and approval', async ({browser}) => {
  const context=await browser.newContext({permissions:['clipboard-read','clipboard-write']}), adminContext=await browser.newContext();
  const page=await context.newPage(), admin=await adminContext.newPage();
  const payloads=[];
  page.on('request',r=>{if(r.postData())payloads.push(r.postData());});
  try {
    await page.goto(base);
    await expect(page.getByLabel('Wallet index',{exact:true})).toHaveValue('0');
    await page.getByLabel('Recovery phrase',{exact:true}).fill(memberPhrase);
    await expect(page.getByLabel('User name',{exact:true})).not.toHaveValue('');
    const defaultName=await page.getByLabel('User name',{exact:true}).inputValue();
    await page.getByLabel('Wallet index',{exact:true}).fill('2147483648');
    await page.getByRole('button',{name:'Sign in',exact:true}).click();
    await expect(page.getByRole('alert')).toContainText('Wallet index must be a whole number');
    await page.getByLabel('Wallet index',{exact:true}).fill('1');
    await expect(page.getByLabel('User name',{exact:true})).not.toHaveValue(defaultName);
    await expect(page.getByLabel('User name',{exact:true})).toHaveValue(/70997970/);
    await page.getByLabel('User name',{exact:true}).fill('My indexed wallet');
    await page.getByRole('button',{name:'Sign in',exact:true}).click();
    await expect(page.getByRole('heading',{name:'You’re on the waiting list.'})).toBeVisible();
    let info=await(await context.request.get(base+'/api/auth/info')).json();
    expect(info.address).toBe('0x70997970c51812dc3a010c7d01b50e0d17dc79c8');
    expect(info.wallet_index).toBe(1);expect(info.name).toBe('My indexed wallet');
    expect((await context.request.get(base+'/api/rooms')).status()).toBe(403);
    await openSettings(page); await page.getByRole('button',{name:'Account details',exact:true}).click();
    const dialog=page.getByRole('dialog',{name:'Account details'});
    // Secrets are never displayed: no reveal, only a confirmation-gated Copy.
    // Hardhat account 1 (public test vector) is "test … junk" at index 1.
    const indexedKey='0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d';
    await expect(dialog.getByRole('button',{name:/Show recovery/})).toHaveCount(0);
    await expect(dialog.getByLabel('Wallet index',{exact:true})).toHaveText('1');
    await page.evaluate(()=>navigator.clipboard.writeText(''));
    page.once('dialog',d=>d.dismiss());
    await dialog.getByRole('button',{name:'Copy private key'}).click();
    expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe('');
    page.once('dialog',d=>d.accept());
    await dialog.getByRole('button',{name:'Copy private key'}).click();
    await expect(dialog.getByText('Private key copied')).toBeVisible();
    expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(indexedKey);
    expect(await page.evaluate(()=>document.documentElement.outerHTML)).not.toContain(indexedKey.slice(2));
    await dialog.getByLabel('User name',{exact:true}).fill('Saved across logins');
    await dialog.getByRole('button',{name:'Save name',exact:true}).click();
    await expect(dialog.getByRole('status').filter({hasText:'Name saved'})).toHaveText('Name saved');
    await dialog.getByRole('button',{name:'Close account details'}).click();
    await expect(page.getByLabel('Your name',{exact:true})).toHaveText('Saved across logins');
    // The key survives a reload in local storage; the phrase was never kept.
    await page.reload();
    await openSettings(page); await page.getByRole('button',{name:'Account details',exact:true}).click();
    await expect(dialog.getByLabel('Wallet index',{exact:true})).toHaveText('1');
    await expect(dialog.getByRole('button',{name:'Copy private key'})).toBeVisible();
    expect(await page.evaluate(()=>JSON.parse(localStorage.getItem('invisible-ai.wallet.key')))).toEqual({key:indexedKey,index:1,mnemonic:memberPhrase});
    await dialog.getByRole('button',{name:'Close account details'}).click();
    await login(admin,rootPhrase);
    expect((await adminContext.request.patch(base+`/api/admin/users/${info.address}`,{data:{role:'user'}})).status()).toBe(200);
    await expect(page.getByRole('heading',{name:'Hey, what’s on your mind?',exact:true})).toBeVisible();
    await openSettings(page); await page.getByRole('button',{name:'Account details',exact:true}).click();
    await expect(dialog.getByLabel('User name',{exact:true})).toHaveValue('Saved across logins');
    await expect(dialog.getByLabel('Recovery phrase words')).toHaveCount(0);
    await page.screenshot({path:'test-results/wallet-account.png',fullPage:true});
    await dialog.getByRole('button',{name:'Close account details'}).click();
    await page.getByRole('button',{name:'Sign out',exact:true}).click();
await page.getByRole('button',{name:'Confirm sign out',exact:true}).click();
    await expect(page.getByLabel('Recovery phrase',{exact:true})).toBeVisible();
    await page.getByLabel('Recovery phrase',{exact:true}).fill(memberPhrase);
    await page.getByLabel('Wallet index',{exact:true}).fill('1');
    await page.getByRole('button',{name:'Sign in',exact:true}).click();
    await expect(page.getByRole('heading',{name:'Hey, what’s on your mind?',exact:true})).toBeVisible();
    expect((await(await context.request.get(base+'/api/auth/info')).json()).name).toBe('Saved across logins');
    expect(payloads.join('\n')).not.toContain(memberPhrase);expect(payloads.join('\n')).not.toContain(rootPhrase);
    // The phrase may live only in the wallet entry (a6be17f); nothing else in either storage holds it.
    expect(await page.evaluate(()=>JSON.stringify({local:Object.fromEntries(Object.entries(localStorage).filter(([k])=>k!=='invisible-ai.wallet.key')),session:sessionStorage}))).not.toContain(memberPhrase);
  } finally {await context.close();await adminContext.close();}
});

test('new recovery phrase requires confirmation and a copied backup before registration', async ({browser}) => {
  const context=await browser.newContext({permissions:['clipboard-read','clipboard-write'],viewport:{width:390,height:844}});
  const page=await context.newPage();
  await page.goto(base);
  const payloads=[];page.on('request',r=>{if(r.postData())payloads.push(r.postData());});
  page.once('dialog',d=>d.dismiss());
  await page.getByRole('button',{name:'Create new recovery phrase'}).click();
  await expect(page.getByLabel('Recovery phrase words')).toHaveCount(0);
  page.once('dialog',d=>d.accept());
  await page.getByRole('button',{name:'Create new recovery phrase'}).click();
  // The words stay hidden until Show is pressed, so they can be written on paper.
  const words=page.getByTestId('recovery-words');
  await expect(words).toHaveCount(0);
  await expect(page.getByRole('button',{name:'12 words'})).toHaveAttribute('aria-pressed','true');
  await page.getByRole('button',{name:'Copy recovery phrase'}).click();
  await expect(page.getByText('Recovery phrase copied')).toBeVisible();
  let phrase=await page.evaluate(()=>navigator.clipboard.readText());
  expect(phrase.split(' ')).toHaveLength(12);
  expect(await page.evaluate(()=>document.documentElement.outerHTML+document.body.innerText)).not.toContain(phrase);
  await page.getByRole('button',{name:'Show recovery phrase'}).click();
  await expect(words.locator('li')).toHaveText(phrase.split(' '));
  await expect(page.getByRole('button',{name:'Hide recovery phrase'})).toHaveAttribute('aria-pressed','true');
  await page.getByRole('button',{name:'Hide recovery phrase'}).click();
  await expect(words).toHaveCount(0);
  // 24 words is a new phrase, hidden again until shown.
  await page.getByRole('button',{name:'24 words'}).click();
  await expect(words).toHaveCount(0);
  await page.getByRole('button',{name:'Show recovery phrase'}).click();
  await expect(words.locator('li')).toHaveCount(24);
  await page.getByRole('button',{name:'Copy recovery phrase'}).click();
  await expect.poll(()=>page.evaluate(()=>navigator.clipboard.readText())).not.toBe(phrase);
  phrase=await page.evaluate(()=>navigator.clipboard.readText());
  await expect(words.locator('li')).toHaveText(phrase.split(' '));
  await page.screenshot({path:'test-results/recovery-24-shown.png',fullPage:true});
  await page.getByRole('button',{name:'Hide recovery phrase'}).click();
  await expect(page.getByLabel('User name',{exact:true})).not.toHaveValue('');
  await expect(page.getByRole('button',{name:'Sign in',exact:true})).toBeDisabled();
  await page.getByLabel('I saved my recovery phrase and wallet index.').check();
  await page.getByLabel('Wallet index',{exact:true}).fill('3');
  await expect(page.getByRole('button',{name:'Sign in',exact:true})).toBeDisabled();
  await page.getByLabel('I saved my recovery phrase and wallet index.').check();
  await page.getByRole('button',{name:'Sign in',exact:true}).click();
  await expect(page.getByRole('heading',{name:'You’re on the waiting list.'})).toBeVisible();
  const info=await(await page.request.get(base+'/api/auth/info')).json();
  expect(info.wallet_index).toBe(3);expect(info.approved).toBe(false);
  expect((await page.request.post(base+'/api/think',{data:{prompt:'Not approved'}})).status()).toBe(403);
  await openSettings(page); await page.getByRole('button',{name:'Account details',exact:true}).click();
  await expect(page.getByRole('button',{name:'Copy private key'})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.screenshot({path:'test-results/wallet-mobile.png',fullPage:true});
  expect(payloads.join('\n')).not.toContain(phrase);
  // The phrase is kept in local storage beside the key (for the non-EVM
  // chains), and nowhere else: not in session storage, not in any request.
  const local=await page.evaluate(()=>JSON.parse(localStorage.getItem('invisible-ai.wallet.key')));
  expect(local.mnemonic).toBe(phrase);
  expect(await page.evaluate(()=>JSON.stringify(sessionStorage))).not.toContain(phrase);
  await expect(page.getByRole('button',{name:'Copy recovery phrase'})).toBeVisible();
  await context.close();
});

test('a legacy signed-in wallet can still submit its registration', async ({page}) => {
  // Older sessions could exist before a user clicked Apply for registration.
  // Keep this recovery path available even though new logins apply automatically.
  let legacy=true;
  await page.route('**/api/auth/info',async route=>{
    if (legacy) await route.fulfill({json:{authenticated:true,approved:false,wallet_login:true,required:true,role:'unregistered',address:memberAddress,name:'Legacy wallet',wallet_index:0,is_admin:false}});
    else await route.fulfill({json:{authenticated:true,approved:false,wallet_login:true,required:true,role:'pending',address:memberAddress,name:'Legacy wallet',wallet_index:0,is_admin:false}});
  });
  await page.route('**/api/auth/apply',async route=>{legacy=false;await route.fulfill({json:{ok:true,role:'pending'}});});
  await page.goto(base);
  await page.getByRole('button',{name:'Apply for registration',exact:true}).click();
  await expect(page.getByRole('heading',{name:'You’re on the waiting list.'})).toBeVisible();
  await expect(page.getByRole('button',{name:'Apply for registration',exact:true})).toHaveCount(0);
});
