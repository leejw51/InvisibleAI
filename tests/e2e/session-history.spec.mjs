import {test, expect} from '@playwright/test';
import {randomUUID} from 'node:crypto';
import {isolatedBackend} from './invisible-fixture.mjs';
import {openConversationOptions} from './workspace-controls.mjs';

isolatedBackend(test, 3171);
async function node(id) {
  const socket = new WebSocket('ws://127.0.0.1:3171/api/client/ws');
  await new Promise((resolve,reject) => {
    socket.onopen=()=>socket.send(JSON.stringify({type:'hello',id,name:id,platform:'linux'}));
    socket.onerror=reject;
    socket.onmessage=e=>{if(JSON.parse(e.data).type==='ready')resolve();};
  });
  const timer=setInterval(()=>socket.readyState===1&&socket.send(JSON.stringify({type:'heartbeat'})),10000);
  return ()=>{clearInterval(timer);socket.close();};
}

test('Cloud opens the same chatroom, memos persist, and replies retain their actual local or cloud model',async({page,request})=>{
  await request.patch('/api/ai/preferences',{data:{mode:'ondevice',local_provider:'apple',cloud_provider:'grok'}});
  await page.goto('/autonomous');
  await page.getByRole('button',{name:'Add a helper',exact:true}).click();
  const title='Memo context '+randomUUID();
  await page.getByRole('textbox',{name:'Name your helper'}).fill(title);
  await page.getByRole('combobox',{name:'Which Mac?'}).selectOption('');
  await page.getByRole('button',{name:'Save for later',exact:true}).click();
  // A helper without a Mac is a saved chat: a link under "Saved chats", not a running card.
  const saved=page.locator('.saved-chats');
  if((await saved.getAttribute('open'))===null) await saved.locator(':scope > summary').click();
  const link=saved.getByRole('link',{name:title});
  const id=new URL(await link.getAttribute('href'),'http://local').searchParams.get('room');
  await link.click();
  await openConversationOptions(page);
  await expect(page.getByRole('combobox',{name:'Chat room'})).toHaveValue(id);
  await expect(page.locator('.capture-options')).toHaveCount(0);
  await expect(page.locator('.capture-action-hint')).toHaveText('Capture saves a screenshot to this chat.');
  await expect(page).toHaveURL(new RegExp(`/chat\\?room=${id}`));
  const editor=page.getByRole('textbox',{name:'Chat message',exact:true});
  await editor.fill('Project tangerine uses one capture computer.');
  await page.getByRole('button',{name:'Save memo',exact:true}).click();
  await expect(page.locator('.from-user')).toContainText('Kept as a note');
  await expect(page.locator('.from-ai')).toHaveCount(0);
  await editor.fill('Remember this project context.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-ai .chat-message-meta')).toContainText('Local AI · apple ·');
  await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
  await request.patch('/api/ai/preferences',{data:{mode:'cloudai',local_provider:'apple',cloud_provider:'grok'}});
  await page.reload();
  await openConversationOptions(page);
  await expect(page.getByRole('combobox',{name:'Chat room'})).toHaveValue(id);
  await editor.fill('Continue with the cloud model.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-ai')).toHaveCount(2);
  await expect(page.locator('.from-ai .chat-message-meta').last()).toContainText('Cloud LLM · grok ·');
  await expect(page.locator('.from-ai [data-testid="chat-llm-log"]').last()).toContainText('Answered by Cloud AI · grok ·');
  await expect(page.locator('.from-ai .chat-message-meta').first()).toContainText('Local AI · apple ·');
  await page.reload();
  await expect(page.locator('.from-user').first()).toContainText('Project tangerine');
  await expect(page.locator('.from-ai .chat-message-meta').first()).toContainText('Local AI');
  await expect(page.locator('.from-ai .chat-message-meta').last()).toContainText('Cloud LLM');
  await page.goto('/autonomous');
  // Still no Mac, so still a saved chat: it stays listed under "Saved chats".
  if((await saved.getAttribute('open'))===null) await saved.locator(':scope > summary').click();
  await expect(saved.getByRole('link',{name:title})).toBeVisible();
  const menu=page.getByRole('button',{name:'Open navigation',exact:true});
  if(await menu.isVisible())await menu.click();
  await expect(page.locator('[data-nav="autonomous"]')).toBeVisible();
  await expect(page.locator('[data-nav="chat"]')).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
});

// Since the hero redesign (3ba973a) a chat without a Mac is a saved chat with no capture card,
// so a capture computer can no longer be attached from the Autonomous page as this test does.
test.fixme('Cloud owns capture and polling controls while chat keeps the same session history',async({page,request},info)=>{
  const first='node-'+randomUUID(),second='node-'+randomUUID();
  const closeFirst=await node(first),closeSecond=await node(second);
  let room;
  try {
    await request.patch('/api/ai/preferences',{data:{mode:'cloudai',local_provider:'apple',cloud_provider:'grok'}});
    // Cloud AI polling is floored at one minute by default; lower it for this test.
    expect((await request.patch('/api/settings',{data:{local_ai:{local_only:false,apple_enabled:false,mlx_enabled:false,mlx_url:'http://127.0.0.1:8080/v1',speech_provider:'auto',speech_locale:'en-US',mlx_speech_model:'',auto_interval_seconds:60,local_interval_seconds:5,cloud_min_interval_seconds:2}}})).ok()).toBeTruthy();
    room=(await (await request.post('/api/rooms',{data:{title:'Polling session'}})).json()).roomid;
    await page.goto('/autonomous');
    const card=page.locator(`[data-session-id="${room}"]`);
    await card.getByTestId('manual-capture').locator('> summary').click();
    await card.getByRole('combobox',{name:'Capture computer',exact:true}).selectOption(first);
    await card.getByRole('button',{name:'Capture now',exact:true}).click();
    await expect.poll(async()=> (await (await request.get(`/api/invisible/sessions/${room}`)).json()).client_id).toBe(first);
    await expect(card.getByRole('combobox',{name:'Capture computer',exact:true})).toHaveCount(0);
    await card.getByLabel('Capture interval',{exact:true}).fill('1');
    await card.getByRole('button',{name:'Start polling',exact:true}).click();
    await expect(page.getByRole('alert')).toContainText('Choose an interval from 2 to 300 seconds');
    await card.getByLabel('Capture interval',{exact:true}).fill('8');
    await page.waitForTimeout(2200); // A session refresh must preserve the edited interval.
    await expect(card.getByLabel('Capture interval',{exact:true})).toHaveValue('8');
    await card.getByRole('button',{name:'Start polling',exact:true}).click();
    await expect(card.getByRole('button',{name:'Stop polling',exact:true})).toBeVisible();
    await expect(card.getByLabel('Capture interval',{exact:true})).toBeDisabled();
    await expect(card.getByLabel('Capture interval',{exact:true})).toHaveValue('8');
    await page.reload();
    await expect(card.getByRole('button',{name:'Stop polling',exact:true})).toBeVisible();
    await card.getByRole('link',{name:'Open chat history',exact:true}).click();
    const picker=page.getByRole('combobox',{name:'Active capture device'});
    await expect(page.locator('.capture-options')).toHaveCount(0);
    await expect(page.getByLabel('Capture interval',{exact:true})).toHaveCount(0);
    await expect(page.getByRole('link',{name:'Manage in Autonomous AI'})).toBeVisible();
    await expect(picker).toBeDisabled();
    await expect(picker).toHaveValue(first);
    await expect.poll(async()=>(await (await request.get(`/api/rooms/${room}/capture`)).json()).next_capture_at).not.toBeNull();
    const before=await (await request.get(`/api/rooms/${room}/capture`)).json();
    await page.getByRole('main').getByRole('button',{name:'Capture',exact:true}).click();
    await expect.poll(async()=>{
      const after=await (await request.get(`/api/rooms/${room}/capture`)).json();
      return !!after.next_capture_at&&after.next_capture_at>before.next_capture_at;
    }).toBe(true);
    await expect(page.locator('.from-user')).toHaveCount(3);
    expect((await request.post('/api/think',{data:{room_id:room,message_id:randomUUID(),capture:true,client_id:second}})).ok()).toBe(false);
    await page.reload();
    await expect(picker).toBeDisabled();
    await expect(picker).toHaveValue(first);
    await page.getByRole('link',{name:'Manage in Autonomous AI'}).click();
    await card.getByRole('button',{name:'Capture now',exact:true}).click();
    await expect.poll(async()=> (await (await request.get(`/api/rooms/${room}/sync`)).json()).messages.filter(m=>m.role==='user'&&!m.deleted).length).toBe(4);
    await card.getByRole('button',{name:'Stop polling',exact:true}).click();
    await expect(card.getByRole('button',{name:'Start polling',exact:true})).toBeVisible();
    await expect(card.getByLabel('Capture interval',{exact:true})).toHaveValue('8');
    await page.getByTestId('language-select').selectOption('ko');
    await expect(card.getByRole('button',{name:'주기적 캡처 시작',exact:true})).toBeVisible();
    await page.screenshot({path:info.outputPath('cloud-session-controls.png'),fullPage:true});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  } finally {
    if(room)await request.delete(`/api/rooms/${room}/capture`);
    closeFirst();closeSecond();
  }
});
