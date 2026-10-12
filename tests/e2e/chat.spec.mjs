import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
async function newChat(page) {
  await page.goto('/chat');
  await expect(page.getByRole('heading',{name:'Keep the thought going.'})).toBeVisible();
  const before=await page.getByRole('combobox',{name:'Chat room'}).inputValue();
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  await expect(page.getByRole('combobox',{name:'Chat room'})).not.toHaveValue(before);
  await expect(page.getByRole('combobox',{name:'Chat room'})).not.toHaveValue('');
  await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toBeEnabled();
  return page.getByRole('combobox',{name:'Chat room'}).inputValue();
}
async function cached(page) {
  return page.evaluate(()=>new Promise((resolve,reject)=>{
    const r=indexedDB.open('causewaybay-messages-local');r.onerror=()=>reject(r.error);r.onsuccess=()=>{const db=r.result;const q=db.transaction('messages').objectStore('messages').getAll();q.onsuccess=()=>{db.close();resolve(q.result.map(JSON.parse));};q.onerror=()=>reject(q.error);};
  }));
}
test('chat GUIDs, IndexedDB-only message cache, room sync, reconnect, and mobile layout',async({page,context,request})=>{
  const room=await newChat(page);expect(room).toMatch(/^[a-f\d-]{36}$/);
  const prompt='Please remember the word tangerine '+Date.now();
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill(prompt);await page.getByRole('button',{name:'Send message'}).click();
  await expect(page.locator('.from-user')).toContainText(prompt);
  await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
  await expect.poll(async()=> (await cached(page)).filter(m=>m.roomid===room&&m.role==='assistant'&&m.status==='complete').length).toBe(1);
  const record=(await cached(page)).find(m=>m.roomid===room);expect(record.messageid).toMatch(/^[a-f\d-]{36}$/);expect(Number.isSafeInteger(record.messagesyncid)).toBe(true);
  expect(await page.evaluate(()=>JSON.stringify(localStorage))).not.toContain(prompt);
  await page.reload();await page.getByRole('combobox',{name:'Chat room'}).selectOption(room);
  await expect(page.locator('.from-user')).toContainText(prompt);await expect(page.locator('.from-ai')).toHaveCount(1);
  await context.setOffline(true);
  const second='Sent while the browser was disconnected';
  const submitted=await request.post('/api/think',{data:{room_id:room,message_id:crypto.randomUUID(),prompt:second}});expect(submitted.ok()).toBeTruthy();
  await context.setOffline(false);
  await expect(page.locator('.from-user')).toHaveCount(2);await expect(page.locator('.from-user').last()).toContainText(second);
  await expect(page.locator('.from-ai')).toHaveCount(2);
  await page.screenshot({path:'test-results/chat-desktop.png',fullPage:true});
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  await page.screenshot({path:'test-results/chat-mobile.png',fullPage:true});
});
test('chat links to Autonomous AI to manage an existing backend capture session',async({page,request})=>{
  const folder=fs.mkdtempSync(path.join(os.tmpdir(),'cwb-continuous-'));
  const child=spawn('./target/debug/rust-cli',['client','--backend','http://127.0.0.1:3099','--name','Continuous Mac','--client-id','continuous-test'],{env:{...process.env,APP_TOKEN:'',CAUSEWAYBAY_CLIENT_CONFIG_DIR:folder},stdio:'ignore'});
  try {
    await expect.poll(async()=> (await (await request.get('/api/clients')).json()).some(c=>c.id==='continuous-test'&&c.online)).toBeTruthy();
    const room=await newChat(page);
    await expect(page.locator('.capture-options')).toHaveCount(0);
    // Cloud AI polling is floored at one minute by default; lower it for this test.
    expect((await request.patch('/api/settings',{data:{local_ai:{local_only:false,apple_enabled:false,mlx_enabled:false,mlx_url:'http://127.0.0.1:8080/v1',speech_provider:'auto',speech_locale:'en-US',mlx_speech_model:'',auto_interval_seconds:60,local_interval_seconds:5,cloud_min_interval_seconds:2}}})).ok()).toBeTruthy();
    expect((await request.post(`/api/rooms/${room}/capture`,{data:{client_ids:['continuous-test'],interval_seconds:2,request:{}}})).ok()).toBeTruthy();
    await expect(page.getByRole('link',{name:'Manage in Autonomous AI'})).toBeVisible();
    await expect.poll(()=>page.locator('.from-user').count()).toBeGreaterThanOrEqual(2);
    await page.getByRole('link',{name:'Manage in Autonomous AI'}).click();
    const card=page.locator(`[data-session-id="${room}"]`);
    await card.getByRole('button',{name:'Stop polling',exact:true}).click();
    await expect(card.getByRole('button',{name:'Start polling',exact:true})).toBeVisible();
    const before=(await (await request.get(`/api/rooms/${room}/sync`)).json()).next;
    await page.waitForTimeout(3000);
    expect((await (await request.get(`/api/rooms/${room}/capture`)).json()).active).toBe(false);
    expect((await (await request.get(`/api/rooms/${room}/sync?after=${before}`)).json()).messages).toEqual([]);
  } finally {child.kill('SIGTERM');}
});
test('chat uploads, file processing and deletion, plus the image library',async({page,request})=>{
  const room=await newChat(page);
  await page.getByLabel('Attach to chat').setInputFiles({name:'chat-data.csv',mimeType:'text/csv',buffer:Buffer.from('name,value\nhello,42\n')});
  await expect(page.locator('.chat-attachment')).toContainText('chat-data.csv');
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Explain this CSV.');await page.getByRole('button',{name:'Send message'}).click();
  await expect(page.locator('.chat-file')).toContainText('chat-data.csv');
  const files=await (await request.get('/api/files')).json();const file=files.find(f=>f.name==='chat-data.csv');
  expect(file.path).toBe('');// text lives in SQLite; only binary media gets a files/ path
  if(!(await page.locator('[data-nav="files"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.getByRole('button',{name:/^Files/}).click();
  const card=page.locator('.file-card').filter({has:page.getByRole('heading',{name:'chat-data.csv',exact:true})});
  await card.getByRole('heading').click();await page.getByRole('button',{name:'Process file',exact:true}).click();
  await expect(page.locator('.file-code')).toContainText('CSV rows: 2');
  await page.getByRole('button',{name:'Close dialog'}).click();
  await card.getByRole('button',{name:'Delete chat-data.csv',exact:true}).click();await card.getByRole('button',{name:'Confirm delete chat-data.csv',exact:true}).click();
  await expect(card).toHaveCount(0);
  await page.getByRole('button',{name:'Ask anything',exact:true}).click();await page.getByRole('combobox',{name:'Chat room'}).selectOption(room);
  await expect(page.locator('.chat-file')).toHaveCount(0);
  const image=await (await request.post('/api/files',{data:{name:'gallery-test.png',data_base64:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jJXcAAAAASUVORK5CYII='}})).json();
  if(!(await page.locator('[data-nav="images"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.getByRole('button',{name:'Images',exact:true}).click();
  const imageCard=page.locator('.image-card').filter({has:page.getByRole('heading',{name:'gallery-test.png'})});
  await expect(imageCard.locator('img')).toBeVisible();
  await imageCard.getByRole('button',{name:'Delete image',exact:true}).click();await imageCard.getByRole('button',{name:'Confirm delete image'}).click();
  await expect(imageCard).toHaveCount(0);expect((await request.get(`/api/files/${image.id}/download`)).ok()).toBe(false);
});

test('multiple photos and comments stay together across follow-ups, reload, and room clear',async({page,request})=>{
  const calls=[];
  page.on('websocket',ws=>ws.on('framesent',frame=>{
    const data=JSON.parse(String(frame.payload));
    if(data.path==='/api/think')calls.push(data.body);
  }));
  const room=await newChat(page);
  const png=Buffer.from(await page.evaluate(()=>{const canvas=document.createElement('canvas');canvas.width=90;canvas.height=70;const ctx=canvas.getContext('2d');ctx.fillStyle='royalblue';ctx.fillRect(0,0,90,70);return canvas.toDataURL('image/png').split(',')[1];}),'base64');
  const photo=name=>({name,mimeType:'image/png',buffer:png});
  await page.getByLabel('Attach to chat').setInputFiles([photo('first.png'),photo('second.png'),photo('discard.png')]);
  await expect(page.locator('.chat-attachment')).toHaveCount(3);
  await expect(page.locator('.chat-attachment img')).toHaveCount(3);
  await page.getByRole('button',{name:'Remove attachment: discard.png',exact:true}).click();
  await expect(page.locator('.chat-attachment')).toHaveCount(2);
  await page.getByLabel('Chat message',{exact:true}).fill('These two photos are the starting point.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-user .chat-photo')).toHaveCount(2);
  await expect(page.locator('.from-user')).toContainText('starting point');
  await expect.poll(()=>calls.length).toBe(1);
  expect(calls[0].file_ids).toHaveLength(2);
  expect(calls[0].input_mode).toBe('image');
  await expect(page.locator('.chat-attachment')).toHaveCount(0);
  await expect(page.locator('.from-ai .answer-content').last()).toContainText('demo mode');
  await expect(page.getByRole('button',{name:'Stop answer'})).toHaveCount(0);
  await page.getByLabel('Attach to chat').setInputFiles(photo('followup.png'));
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await page.getByLabel('Chat message',{exact:true}).fill('Compare this photo with the first two.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-user')).toHaveCount(2);
  await expect(page.locator('.from-user .chat-photo')).toHaveCount(3);
  await expect.poll(()=>calls.length).toBe(2);
  expect(calls[1].room_id).toBe(calls[0].room_id);
  expect(calls[1].file_ids).toHaveLength(1);
  await expect(page.locator('.from-ai .answer-content').last()).toContainText('demo mode');
  await expect(page.getByRole('button',{name:'Stop answer'})).toHaveCount(0);
  await page.getByLabel('Chat message',{exact:true}).fill('One more comment with no new photo.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-user')).toHaveCount(3);
  await expect(page.locator('.from-ai .answer-content').last()).toContainText('demo mode');
  await expect(page.getByRole('button',{name:'Stop answer'})).toHaveCount(0);
  await page.reload();
  await page.getByRole('combobox',{name:'Chat room'}).selectOption(room);
  await expect(page.locator('.from-user')).toHaveCount(3);
  await expect(page.locator('.from-user .chat-photo')).toHaveCount(3);
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect((await page.locator('.chat-messages').boundingBox()).height).toBeGreaterThanOrEqual(280);
  await page.screenshot({path:'test-results/photo-chat-mobile.png',fullPage:true});
  const files=(await (await request.get('/api/files')).json()).filter(f=>['first.png','second.png','discard.png','followup.png'].includes(f.name));
  await page.getByRole('button',{name:'Clear chatroom',exact:true}).click();
  await page.getByRole('button',{name:'Confirm clear chatroom',exact:true}).click();
  await expect(page.locator('.chat-message')).toHaveCount(0);
  for(const file of files)expect((await request.get(`/api/files/${file.id}/download`)).ok()).toBe(false);
});

test('attachment limit keeps the existing draft when too many photos are selected',async({page})=>{
  await newChat(page);
  const file=i=>({name:`photo-${i}.txt`,mimeType:'text/plain',buffer:Buffer.from('A sample comment')});
  await page.getByLabel('Attach to chat').setInputFiles(file(0));
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await page.getByLabel('Attach to chat').setInputFiles(Array.from({length:8},(_,i)=>file(i+1)));
  await expect(page.getByRole('alert')).toContainText('Attach up to 8 photos or files per message');
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await page.getByLabel('Chat message',{exact:true}).fill('My comment is still here.');
  await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeEnabled();
});
