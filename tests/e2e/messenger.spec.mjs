import {openConversationOptions, openSettings, openNav} from './workspace-controls.mjs';
import {test,expect} from '@playwright/test';

async function cachedMessages(page) {
  return page.evaluate(() => new Promise((resolve, reject) => {
    const request = indexedDB.open('causewaybay-messages-local');
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const all = db.transaction('messages').objectStore('messages').getAll();
      all.onsuccess = () => {
        resolve(all.result.map(value => {
          try { return JSON.parse(value); } catch { return null; }
        }).filter(value => value && typeof value === 'object'));
        db.close();
      };
      all.onerror = () => { db.close(); reject(all.error); };
    };
  }));
}

test('Reload resumes the current thought, capture answers automatically, and model choice stays in this browser',async({page,request})=>{
  const frames=[];page.on('websocket',ws=>ws.on('framesent',f=>{try{frames.push(JSON.parse(String(f.payload)));}catch{}}));
  await page.goto('/');await expect(page.getByRole('heading',{name:'Hey, what’s on your mind?',exact:true})).toBeVisible();
  await openConversationOptions(page);
  await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');const room=await page.getByLabel('Chat room',{exact:true}).inputValue();expect(room).toMatch(/^[a-f\d-]{36}$/);
  const before=await (await request.get('/api/settings')).json();
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await page.getByLabel('Chat AI provider').selectOption('openai');
  await page.getByLabel('Chat AI model').fill('personal-test-model');await page.getByLabel('Chat AI model').press('Tab');
  await openConversationOptions(page);
  await page.reload();await expect(page.getByLabel('Chat room',{exact:true})).toHaveValue(room);
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await expect(page.getByLabel('Chat AI provider')).toHaveValue('openai');await expect(page.getByLabel('Chat AI model')).toHaveValue('personal-test-model');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.locator('.from-ai').last()).toContainText('Change - to +');
  await expect(page.locator('.from-ai').last().getByRole('button',{name:'Edit message',exact:true})).toBeVisible();
  const think=frames.find(v=>v.path==='/api/think');expect(think.body).toMatchObject({capture:true,prompt:'',input_mode:'image',room_id:room,provider:'openai',model:'personal-test-model'});
  expect(think.body).not.toHaveProperty('api_key');
  const after=await (await request.get('/api/settings')).json();expect(after.provider).toBe(before.provider);
  expect(frames.some(v=>v.path==='/api/think')).toBeTruthy();
  // The capture toolbar stays visible without scrolling to the composer.
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeInViewport();
  await page.screenshot({path:'test-results/messenger-desktop.png',fullPage:true});
  await page.setViewportSize({width:390,height:844});
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeVisible();
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeInViewport();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  await page.screenshot({path:'test-results/messenger-mobile.png',fullPage:true});
});

test('message edits and removals replay over SSE; room rename/delete reaches another device',async({page,browser})=>{
  const secondContext=await browser.newContext();const other=await secondContext.newPage();
  try{
    await page.goto('/chat');await openConversationOptions(page);await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');const previous=await page.getByLabel('Chat room',{exact:true}).inputValue();await page.getByRole('button',{name:'New chat',exact:true}).click();await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue(previous);
    await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');const room=await page.getByLabel('Chat room',{exact:true}).inputValue();
    await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Original messenger note');
    await page.getByRole('textbox',{name:'Chat message',exact:true}).press('Enter');
    await expect(page.locator('.from-ai').getByRole('button',{name:'Edit message',exact:true})).toBeVisible();
    await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0);
    await other.goto('/chat');await openConversationOptions(other);await other.getByLabel('Chat room',{exact:true}).selectOption(room);
    await expect(other.locator('.from-user')).toContainText('Original messenger note');
    await secondContext.setOffline(true);
    const message=page.locator('.from-user');const originalSync=BigInt(await message.getAttribute('data-sync-id'));
    await message.getByRole('button',{name:'Edit message',exact:true}).click();
    await message.getByRole('textbox',{name:'Edit message',exact:true}).fill('Revised messenger note');
    await message.getByRole('button',{name:'Save message',exact:true}).click();
    await expect(message).toContainText('Revised messenger note');
    expect(BigInt(await message.getAttribute('data-sync-id'))).toBeGreaterThan(originalSync);
    await secondContext.setOffline(false);await expect(other.locator('.from-user')).toContainText('Revised messenger note');
    await page.locator('.from-ai').getByRole('button',{name:'Remove message',exact:true}).click();
    await page.getByRole('button',{name:'Confirm remove message',exact:true}).click();
    await expect(page.locator('.from-ai')).toHaveCount(0);await expect(other.locator('.from-ai')).toHaveCount(0);
    await openConversationOptions(other);
    await other.reload();await openConversationOptions(other);await other.getByLabel('Chat room',{exact:true}).selectOption(room);await expect(other.locator('.from-ai')).toHaveCount(0);
    await openConversationOptions(page);
    await page.getByRole('button',{name:'Rename chatroom'}).click();await page.getByLabel('Chatroom name').fill('Daily work');await page.getByRole('button',{name:'Save name'}).click();
    await openConversationOptions(other);
    await expect(other.getByLabel('Chat room',{exact:true}).locator('option:checked')).toHaveText('Daily work');
    await openConversationOptions(page);
    await page.getByRole('button',{name:'Delete chatroom'}).click();await page.getByRole('button',{name:'Confirm delete chatroom'}).click();
    await openConversationOptions(other);
    await expect(other.getByLabel('Chat room',{exact:true}).locator(`option[value="${room}"]`)).toHaveCount(0);
    expect((await other.request.get(`/api/rooms/${room}/sync`)).ok()).toBeTruthy(); // Tombstones remain replayable.
  }finally{await secondContext.close();}
});

test('clearing a chatroom erases attachments and offline cache; the same room accepts new messages',async({page,browser,request})=>{
  const context=await browser.newContext();const other=await context.newPage();
  const cached=cachedMessages;
  try{
    await page.goto('/');await openConversationOptions(page);await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');const room=await page.getByLabel('Chat room',{exact:true}).inputValue();
    await page.getByLabel('Attach to chat').setInputFiles({name:'erase-room.txt',mimeType:'text/plain',buffer:Buffer.from('room clear private original')});
    await expect(page.locator('.chat-attachment')).toContainText('erase-room.txt');
    await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('erase-room-private-message');
    await page.getByRole('button',{name:'Send message',exact:true}).click();
    await expect(page.locator('.from-user').last()).toContainText('erase-room-private-message');
    await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0);
    const file=(await(await request.get('/api/files')).json()).find(f=>f.name==='erase-room.txt');expect(file).toBeTruthy();
    await other.goto('/');await expect(other.locator('.from-user').last()).toContainText('erase-room-private-message');
    await expect.poll(async()=>(await cached(other)).some(m=>m.text==='erase-room-private-message')).toBe(true);
    // Deletion notifications must purge cached chat even while viewing a different page.
    if(!(await other.locator('[data-nav="files"]').isVisible())) await other.locator('.more-tools > summary').click();await other.getByRole('button',{name:/^Files(?: \d+)?$/}).click();await context.setOffline(true);
    await openConversationOptions(page);
    await page.getByRole('button',{name:'Clear chatroom'}).click();await page.getByRole('button',{name:'Confirm clear chatroom'}).click();
    await openConversationOptions(page);
    await expect(page.locator('.chat-message')).toHaveCount(0);await expect(page.getByLabel('Chat room',{exact:true})).toHaveValue(room);
    expect((await request.get(`/api/files/${file.id}/download`)).status()).toBe(404);
    await context.setOffline(false);
    await expect.poll(async()=>(await cached(other)).filter(m=>m.roomid===room&&!m.deleted).length).toBe(0);
    await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('fresh message after clear');await page.getByRole('textbox',{name:'Chat message',exact:true}).press('Enter');
    await expect(page.locator('.from-user')).toContainText('fresh message after clear');
    await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0);
    await page.reload();await expect(page.locator('.from-user')).toContainText('fresh message after clear');
    await expect.poll(async()=>(await cached(page)).some(m=>m.text==='fresh message after clear')).toBe(true);
    await openConversationOptions(other);
    await openNav(other,'home');await other.getByRole('button',{name:'New chat',exact:true}).click();await openConversationOptions(other);await other.getByLabel('Chat room',{exact:true}).selectOption(room);await expect(other.locator('.from-user')).toContainText('fresh message after clear');
    await expect(other.locator('.chat-messages')).not.toContainText('erase-room-private-message');
  }finally{await context.close()}
});

test('late and duplicate room events cannot resurrect cleared content in UI or IndexedDB', async ({page, request}) => {
  // Keep the real HTTP stream, but retain its events to simulate delayed delivery.
  await page.addInitScript(() => {
    const NativeEventSource = window.EventSource;
    window.roomStreams = [];
    window.EventSource = class extends NativeEventSource {
      constructor(url, options) {
        super(url, options);
        if (String(url).includes('/api/rooms/')) window.roomStreams.push(this);
      }
    };
  });
  const roomResponse = await request.post('/api/rooms', {data:{title:'Delayed replay'}});
  expect(roomResponse.ok()).toBe(true);
  const {roomid} = await roomResponse.json();
  await page.goto('/chat');
  await openConversationOptions(page);
  await page.getByLabel('Chat room', {exact:true}).selectOption(roomid);
  await expect.poll(() => page.evaluate(room => window.roomStreams.some(source => source.url.includes(room) && source.readyState === EventSource.OPEN), roomid)).toBe(true);
  // Hold the cache briefly so two selections finish reading together. This used
  // to let an older subscription's destructor close the newer room stream.
  await page.evaluate(() => new Promise((resolve, reject) => {
    const request = indexedDB.open('causewaybay-messages-local');
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const transaction = db.transaction('messages', 'readwrite');
      const store = transaction.objectStore('messages');
      window.holdRoomCache = true;
      const pump = () => { store.get('__test_lock__').onsuccess = () => { if (window.holdRoomCache) pump(); }; };
      transaction.oncomplete = () => db.close();
      pump();
      resolve();
    };
  }));
  await openConversationOptions(page);
  await page.getByLabel('Chat room', {exact:true}).selectOption(roomid);
  await openConversationOptions(page);
  await page.getByLabel('Chat room', {exact:true}).selectOption(roomid);
  await page.evaluate(() => { window.holdRoomCache = false; });
  const compose = page.getByRole('textbox', {name:'Chat message', exact:true});
  await compose.fill('private content erased before delayed replay');
  await compose.press('Enter');
  await expect(page.locator('.from-user')).toContainText('private content erased');
  await expect(page.locator('.from-ai')).toHaveCount(1);
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  const old = await (await request.get(`/api/rooms/${roomid}/sync`)).json();
  const replay = async messages => page.evaluate(({roomid, messages}) => {
    const stream = window.roomStreams.findLast(source => source.url.includes(roomid) && source.readyState !== EventSource.CLOSED);
    if (!stream) throw new Error('No active room stream');
    for (const message of messages) stream.dispatchEvent(new MessageEvent('message', {data:JSON.stringify(message)}));
  }, {roomid, messages});
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Clear chatroom'}).click();
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Confirm clear chatroom'}).click();
  await openConversationOptions(page);
  await expect(page.getByRole('button', {name:'Confirm clear chatroom'})).toHaveCount(0);
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeEnabled();
  await expect(page.locator('.chat-message')).toHaveCount(0);
  // Newest first, duplicates and old revisions: delivery order must not matter.
  await replay([...old.messages].reverse().concat(old.messages));
  await compose.fill('fresh surviving message');
  await compose.press('Enter');
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-user')).toContainText('fresh surviving message');
  await expect(page.locator('.from-ai')).toHaveCount(1);
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  const current = await (await request.get(`/api/rooms/${roomid}/sync?after=${old.next}`)).json();
  await replay(current.messages.concat(old.messages, current.messages));
  await expect.poll(async () => (await cachedMessages(page)).filter(m => m.roomid === roomid && !m.deleted).map(m => m.messageid).sort())
    .toEqual([...new Set(current.messages.filter(m => !m.deleted).map(m => m.messageid))].sort());
  await page.reload();
  await openConversationOptions(page);
  await page.getByLabel('Chat room', {exact:true}).selectOption(roomid);
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-user')).toContainText('fresh surviving message');
  await expect(page.locator('.chat-messages')).not.toContainText('private content erased');
});


test('chat defaults to full-image capture and remembers an explicit Text mode choice', async ({page}) => {
  await page.goto('/');
  await openConversationOptions(page);
  await expect(page.getByRole('button',{name:'Image mode',exact:true})).toHaveAttribute('aria-pressed','true');
  await openConversationOptions(page);
  await page.getByRole('button',{name:'Text mode',exact:true}).click();
  await page.reload();
  await openConversationOptions(page);
  await expect(page.getByRole('button',{name:'Text mode',exact:true})).toHaveAttribute('aria-pressed','true');
  await openConversationOptions(page);
  await page.getByRole('button',{name:'Image mode',exact:true}).click();
  await page.reload();
  await openConversationOptions(page);
  await expect(page.getByRole('button',{name:'Image mode',exact:true})).toHaveAttribute('aria-pressed','true');
});

// The room's single AI reply at its final revision: stage "Complete" on the
// backend (written after the status already says complete), the same revision
// rendered in the page, and no newer revision arriving right after.
async function settledReply(page, request, room) {
  const latest = async () => {
    const {messages} = await (await request.get(`/api/rooms/${room}/sync`)).json();
    // Every revision is its own row: keep the newest per message.
    return messages.filter(m => !m.deleted && m.role !== 'user')
      .reduce((best, m) => best && BigInt(best.messagesyncid) > BigInt(m.messagesyncid) ? best : m, null);
  };
  const reply = page.locator('.from-ai');
  await expect(reply).toHaveCount(1);
  let revision;
  await expect.poll(async () => { revision = await latest(); return revision?.stage; }).toBe('Complete');
  await expect(reply).toHaveAttribute('data-sync-id', String(revision.messagesyncid));
  await page.waitForTimeout(400);
  expect(String((await latest()).messagesyncid)).toBe(String(revision.messagesyncid));
  await expect(reply).toHaveAttribute('data-sync-id', String(revision.messagesyncid));
}

test('Multi Capture stages screenshots until Send message sends one turn; copy buttons use the correct text', async ({page, context, request}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/');
  await openConversationOptions(page);
  await expect(page.getByLabel('Chat room', {exact:true})).not.toHaveValue('');
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Clear chatroom'}).click();
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Confirm clear chatroom'}).click();
  await openConversationOptions(page);
  await expect(page.getByRole('button', {name:'Confirm clear chatroom'})).toHaveCount(0);
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeEnabled();
  await expect(page.locator('.chat-message')).toHaveCount(0);
  await openConversationOptions(page);
  const room = await page.getByLabel('Chat room', {exact:true}).inputValue();
  const compose = page.getByRole('textbox', {name:'Chat message', exact:true});
  await compose.fill('Use both screenshots together');
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Multi Capture', exact:true}).click();
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await expect(page.locator('.chat-message')).toHaveCount(0);
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Multi Capture', exact:true}).click();
  await expect(page.locator('.chat-attachment')).toHaveCount(2);
  expect((await (await request.get(`/api/rooms/${room}/sync`)).json()).messages.every(m=>m.deleted)).toBe(true);
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-user .chat-photo')).toHaveCount(2);
  await expect(page.locator('.from-ai')).toHaveCount(1);
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  await expect(page.getByRole('button', {name:'Send message', exact:true})).toBeDisabled();
  // The backend keeps revising a reply after it shows as complete (the "done"
  // stage, then Wiki tags), and Edit freezes the revision it was clicked on: a
  // save against a stale revision is refused as changed on another device. Edit
  // only once the reply has reached its final revision on both sides.
  await settledReply(page, request, room);
  await page.locator('.from-ai').getByRole('button', {name:'Edit message',exact:true}).click();
  const response = 'Explanation with **formatting**.\n\n```rust\nfn answer() -> i32 { 42 }\n```\n\nMore explanation.\n\n```text\nsecond block <safe>\n```';
  await page.locator('.from-ai').getByRole('textbox', {name:'Edit message',exact:true}).fill(response);
  await page.getByRole('button', {name:'Save message',exact:true}).click();
  await expect(page.locator('.from-ai .chat-edit')).toHaveCount(0);
  await expect(page.locator('.error-banner')).toHaveCount(0);
  const codeButtons = page.getByRole('button', {name:'Copy code',exact:true});
  await expect(codeButtons).toHaveCount(2);
  await codeButtons.first().click();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe('fn answer() -> i32 { 42 }\n');
  await codeButtons.last().click();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe('second block <safe>\n');
  await page.locator('.from-ai').getByRole('button', {name:'Copy message',exact:true}).click();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(response);
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Copy all text',exact:true}).click();
  const all = await page.evaluate(()=>navigator.clipboard.readText());
  expect(all).toContain('Use both screenshots together'); expect(all).toContain(response);
  expect(all).not.toContain('Copy code');
});

async function openDeleteAll(page) {
  const toggle=page.getByRole('button',{name:'Open navigation',exact:true});
  if(await toggle.isVisible() && await toggle.getAttribute('aria-expanded')!=='true') await toggle.click();
  await openSettings(page);
  await page.locator('[data-nav="delete-all"]').click();
}
test('Delete all requires confirmation and purges other rooms and offline sessions on any page', async ({page, browser, request}) => {
  const context = await browser.newContext(); const other = await context.newPage();
  try {
    await page.goto('/');
    await openConversationOptions(page);
    await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');
    await openConversationOptions(page);
    const room = await page.getByLabel('Chat room',{exact:true}).inputValue();
    await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('delete-all-private-message');
    await page.getByRole('button',{name:'Send message',exact:true}).click();
    await expect(page.locator('.from-user').last()).toContainText('delete-all-private-message');
    await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0);
    const another = await (await request.post('/api/rooms', {data:{title:'also delete this room'}})).json();
    const file = await (await request.post('/api/files', {data:{name:'delete-all-secret.txt',data_base64:Buffer.from('private outside chat').toString('base64')}})).json();
    await other.goto('/');
    await expect(other.locator('.from-user').last()).toContainText('delete-all-private-message');
    if(!(await other.locator('[data-nav="files"]').isVisible())) await other.locator('.more-tools > summary').click();await other.getByRole('button',{name:/^Files(?: \d+)?$/}).click();
    await expect(other.getByText('delete-all-secret.txt',{exact:true})).toBeVisible();
    await context.setOffline(true);
    // Delete all lives in the main menu only, behind a confirmation dialog.
    await openDeleteAll(page);
    await page.getByTestId('delete-all-dialog').getByRole('button',{name:'Cancel',exact:true}).click();
    await expect(page.getByTestId('delete-all-dialog')).toHaveCount(0);
    expect((await request.get(`/api/files/${file.id}/download`)).ok()).toBe(true);
    await openDeleteAll(page);
    await page.getByTestId('delete-all-confirm').click();
    await expect(page.locator('.chat-message')).toHaveCount(0);
    await expect(page.getByTestId('delete-all-dialog')).toHaveCount(0);
    expect((await request.get(`/api/files/${file.id}/download`)).status()).toBe(404);
    expect((await (await request.get('/api/rooms')).json()).some(r=>[room,another.roomid].includes(r.roomid))).toBe(false);
    await context.setOffline(false);
    await expect.poll(async()=>(await cachedMessages(other)).filter(m=>!m.deleted).length).toBe(0);
    await expect(other.getByText('delete-all-secret.txt',{exact:true})).toHaveCount(0);
    await openConversationOptions(page);
    await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');
    await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('new content survives erasure replay');
    await page.getByRole('button',{name:'Send message',exact:true}).click();
    await expect(page.locator('.from-user')).toContainText('new content survives erasure replay');
    await page.reload();
    await expect(page.locator('.from-user')).toContainText('new content survives erasure replay');
    await expect(page.locator('.chat-messages')).not.toContainText('delete-all-private-message');
  } finally { await context.close(); }
});

for(const action of ['clear','all'])test(`a file still loading during ${action} cannot resurrect erased content`,async({page,request})=>{
  await page.addInitScript(()=>{
    const original=File.prototype.arrayBuffer;
    File.prototype.arrayBuffer=function(){
      if(this.name==='delayed-private.txt')return new Promise(resolve=>{window.releasePrivateUpload=()=>original.call(this).then(resolve)});
      return original.call(this);
    };
  });
  const responses=[];page.on('websocket',ws=>ws.on('framereceived',f=>responses.push(String(f.payload))));
  await page.goto('/');await openConversationOptions(page);await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue('');
  const room=await page.getByLabel('Chat room',{exact:true}).inputValue();
  await page.getByLabel('Attach to chat').setInputFiles({name:'delayed-private.txt',mimeType:'text/plain',buffer:Buffer.from('must never be saved after erasure')});
  await expect.poll(()=>page.evaluate(()=>typeof window.releasePrivateUpload)).toBe('function');
  if(action==='clear') {
    await openConversationOptions(page);
    await page.getByRole('button',{name:'Clear chatroom'}).click();await page.getByRole('button',{name:'Confirm clear chatroom'}).click();
  } else {
    await openDeleteAll(page);await page.getByTestId('delete-all-confirm').click();
  }
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeEnabled();
  await page.evaluate(()=>window.releasePrivateUpload());
  await expect.poll(()=>responses.join('\n')).toMatch(/was (cleared|erased)|Database operation failed|Room not found|Not found/);
  expect((await(await request.get('/api/files')).json()).some(f=>f.name==='delayed-private.txt')).toBe(false);
  await expect(page.locator('.chat-attachment')).toHaveCount(0);
  await openConversationOptions(page);
  if(action==='clear')await expect(page.getByLabel('Chat room',{exact:true})).toHaveValue(room);
});
