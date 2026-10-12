import {expectProcessingMode, openProcessingSettings, openWorkspace, openConversationOptions} from './workspace-controls.mjs';
import {isolatedBackend} from './invisible-fixture.mjs';
import {test,expect} from '@playwright/test';
import {randomUUID} from 'node:crypto';

const baseURL=isolatedBackend(test,3154);
const local={local_only:false,apple_enabled:true,mlx_enabled:true,mlx_url:'http://127.0.0.1:8080/v1',speech_provider:'auto',speech_locale:'en-US',mlx_speech_model:'',auto_interval_seconds:2, local_interval_seconds:2, cloud_min_interval_seconds:2};
const mode={mode:'ondevice',local_provider:'apple',cloud_provider:'grok'};
test.beforeEach(async({request})=>{
 for(const room of await(await request.get('/api/rooms')).json())await request.delete(`/api/rooms/${room.roomid}`);
 await request.patch('/api/settings',{data:{provider:'apple',model:'system',local_ai:local}});
 expect((await request.patch('/api/ai/preferences',{data:mode})).ok()).toBeTruthy();
});
test.afterEach(async({request})=>{
 await request.patch('/api/settings',{data:{provider:'grok',local_ai:{...local,apple_enabled:false,mlx_enabled:false,auto_interval_seconds:60}}});
 await request.patch('/api/ai/preferences',{data:{...mode,mode:'cloudai'}});
});

async function fakeMac(existingId,name='My capture Mac'){
 const id=existingId || 'notes-'+randomUUID(),commands=[];let text='Bitcoin is interesting. Help me understand it.';
 const ws=new WebSocket('ws://127.0.0.1:3154/api/client/ws');
 await new Promise((resolve,reject)=>{ws.onopen=()=>ws.send(JSON.stringify({type:'hello',id,name,platform:'macos',textedit_notes_v1:true}));ws.onerror=reject;ws.onmessage=event=>{const v=JSON.parse(event.data);if(v.type==='ready')resolve();if(v.type==='notes_command'){commands.push(v);const ok=v.operation!=='append'||v.expected===text;if(ok&&v.operation==='append')text+=v.suffix;ws.send(JSON.stringify({type:'notes_result',request_id:v.request_id,ok,snapshot:{document:'window:My note.txt',text}}));}};});
 const timer=setInterval(()=>{if(ws.readyState===1)ws.send(JSON.stringify({type:'heartbeat'}));},10000);
 return {id,commands,get text(){return text;},set text(value){text=value;},close(){clearInterval(timer);ws.close();}};
}

test('Auto start and stop stay directly tappable above the fixed composer in short Safari windows',async({page,request},info)=>{
 const mac=await fakeMac(undefined,'My MacBook');
 const sizes=info.project.name==='webkit-iphone' ? [{width:390,height:700},{width:390,height:844}] : [{width:1280,height:812},{width:1152,height:730},{width:1024,height:648}];
 const tap=async(auto)=>{
  // Element visibility alone is insufficient: Playwright's normal click would
  // scroll a covered control into view and conceal this real iPad regression.
  const rect=await auto.boundingBox(),dock=await page.locator('.chat-dock').boundingBox();
  expect(rect.height).toBeGreaterThanOrEqual(44);
  expect(rect.y).toBeGreaterThanOrEqual(0);
  expect(rect.y+rect.height).toBeLessThanOrEqual(dock.y-8);
  const point={x:rect.x+rect.width/2,y:rect.y+rect.height/2};
  expect(await auto.evaluate((button,point)=>button.contains(document.elementFromPoint(point.x,point.y)),point)).toBe(true);
  // Physical coordinates do not get locator click's automatic scrolling.
  await page.mouse.click(point.x,point.y);
 };
 try{
  await page.addInitScript(()=>localStorage.setItem('causewaybay.language.v1','ko'));
  await request.patch('/api/ai/preferences',{data:{...mode,mode:'cloudai'}});
  await page.goto('/');
  await openWorkspace(page); await page.getByRole('combobox',{name:'활성 캡처 기기',exact:true}).selectOption(mac.id);
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');
  await expect(auto).toBeEnabled();
  await expect(page.getByTestId('connection-activity')).toHaveCount(0);
  for(const size of sizes){
   await page.setViewportSize(size);
   await page.evaluate(()=>window.scrollTo(0,0));
   await expect(auto).toHaveAccessibleName('자율 AI 실행 시작');
   await expect(page.locator('.processing-summary')).toContainText('클라우드 AI');
   await tap(auto);
   await expect(auto).toHaveAttribute('aria-pressed','true');
   await expect(auto).toHaveAccessibleName('자율 AI 실행 중지');
   await expect(auto).toBeEnabled();
   await tap(auto);
   await expect(auto).toHaveAttribute('aria-pressed','false');
   expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
   await page.screenshot({path:info.outputPath(`auto-visible-${size.width}x${size.height}.png`)});
  }
 }finally{
  for(const session of await(await request.get('/api/invisible/sessions')).json())if(session.client_id===mac.id){await request.post(`/api/invisible/sessions/${session.id}/stop`);await request.delete(`/api/rooms/${session.id}`);}
  mac.close();
 }
});

test('Capture, TextEdit Auto and processing location have separate actions and clear destinations',async({page,request},info)=>{
 if(info.project.name==='webkit-ipad')await page.setViewportSize({width:1180,height:760});
 const mac=await fakeMac(undefined,'My MacBook');let other,id;
 const captures=[];
 page.on('websocket',socket=>socket.on('framesent',frame=>{
  const message=JSON.parse(String(frame.payload));
  if(message.path==='/api/think')captures.push(message.body);
 }));
 try {
  await page.goto('/');
  const device=page.getByRole('combobox',{name:'Active capture device',exact:true});
  await device.selectOption(mac.id);
  const settings=page.getByTestId('processing-settings');
  await expect(settings).toContainText('AI processing location');
  await expect(settings).toContainText('On-device');
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');
  await expect(auto).toHaveAccessibleName('Start autonomous AI');
  await expect(auto).toHaveAttribute('aria-pressed','false');
  await expect(page.locator('.notes-target')).toHaveText('TextEdit on My MacBook');
  await expect(page.getByRole('combobox',{name:'Invisible AI session'})).toHaveValue(await page.getByLabel('Chat room',{exact:true}).inputValue());
  await expect(page.getByText('This session is stopped. Start it to resume replies in TextEdit.')).toBeVisible();
  const capture=page.getByRole('button',{name:'Capture',exact:true});
  await expect(capture).toHaveCount(1);
  await expect(page.getByRole('button',{name:'Image mode',exact:true})).toBeDisabled();
  await expect(page.getByRole('button',{name:'Text mode',exact:true})).toHaveAttribute('aria-pressed','true');
  await expect(page.locator('#local-ocr-hint')).toHaveText('Local AI reads captured text with OCR.');
  await expect(capture).toHaveAccessibleDescription('Capture saves a screenshot to this chat.');
  let modes=await openProcessingSettings(page);
  await modes.getByRole('button',{name:/Cloud AI/}).click();
  await expect(modes.getByRole('button',{name:/Cloud AI/})).toHaveAttribute('aria-pressed','true');
  await page.goBack();
  await expectProcessingMode(page,'Cloud AI');
  await page.getByRole('button',{name:'Image mode',exact:true}).click();
  await expect(page.getByRole('button',{name:'Image mode',exact:true})).toHaveAttribute('aria-pressed','true');
  modes=await openProcessingSettings(page);
  await modes.getByRole('button',{name:/Hybrid/}).click();
  await expect(modes.getByRole('button',{name:/Hybrid/})).toHaveAttribute('aria-pressed','true');
  await page.goBack();
  await expectProcessingMode(page,'Hybrid');
  await expect(page.getByRole('button',{name:'Image mode',exact:true})).toBeDisabled();
  await expect(page.getByRole('button',{name:'Text mode',exact:true})).toHaveAttribute('aria-pressed','true');
  expect((await(await request.get('/api/ai/preferences')).json()).mode).toBe('hybrid');
  expect((await(await request.get('/api/invisible/sessions')).json())[0].status.active).toBe(false);
  expect(mac.commands).toHaveLength(0);
  await expect(auto).toHaveAttribute('aria-pressed','false');
  modes=await openProcessingSettings(page);
  await modes.getByRole('button',{name:/On-device/}).click();
  await expect(modes.getByRole('button',{name:/On-device/})).toHaveAttribute('aria-pressed','true');
  await page.goBack();
  await expectProcessingMode(page,'On-device');
  await page.getByTestId('language-select').selectOption('ko');
  await expect(auto).toHaveAccessibleName('자율 AI 실행 시작');
  await page.evaluate(()=>window.scrollTo(0,0));
  if(info.project.name==='webkit-ipad'){
   const target=await page.locator('#active-capture-device').boundingBox();
   expect(target.height).toBeGreaterThanOrEqual(44);
   const action=await auto.boundingBox(),dock=await page.locator('.chat-dock').boundingBox();
   expect(action.y+action.height).toBeLessThanOrEqual(dock.y);
  }
  await page.screenshot({path:`test-results/controls-initial-ko-${info.project.name}.png`,fullPage:true});
  await page.getByTestId('language-select').selectOption('en');
  // Demo AI exercises the UI/backend request; it does not capture native pixels.
  await capture.click();
  await expect.poll(()=>captures.length).toBe(1);
  expect(captures[0]).toMatchObject({capture:true,client_id:mac.id});
  await expect(page.locator('.from-ai').last()).toContainText('Demo response from a fixed OCR fixture.');
  expect((await(await request.get('/api/invisible/sessions')).json())[0].status.active).toBe(false);
  expect(mac.commands).toHaveLength(0);
  await auto.click();
  await expect(auto).toHaveAccessibleName('Stop autonomous AI');
  await expect(auto).toHaveAttribute('aria-pressed','true');
  await expect(page.locator('.auto-state')).toHaveText('Running');
  id=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  await expect.poll(()=>mac.text).toContain('[End Invisible AI]');
  // Both capture paths keep the same room binding.
  other=await fakeMac(undefined,'Second Mac');await expect(device).toBeDisabled();
  await expect(page.locator('.notes-target')).toHaveText('TextEdit on My MacBook');
  expect((await(await request.get(`/api/invisible/sessions/${id}`)).json()).client_id).toBe(mac.id);
  expect(other.commands).toHaveLength(0);
  await auto.click();
  await expect(auto).toHaveAccessibleName('Start autonomous AI');
  await expect(page.locator('.auto-state')).toHaveText('Off');
  await expect(device).toHaveValue(mac.id);
  await page.getByTestId('language-select').selectOption('ko');
  await expect(auto).toHaveAccessibleName('자율 AI 실행 시작');
  await expect(page.locator('.processing-summary small')).toHaveText('AI 처리 위치');
  await expect(page.locator('.capture-reading-mode')).toContainText('캡처 분석 방식');
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.evaluate(()=>window.scrollTo(0,0));
  await page.screenshot({path:`test-results/controls-clarity-ko-${info.project.name}.png`,fullPage:true});
  // The badge is read-only; its Change button opens the Autonomous AI page.
  await settings.getByRole('button').click();
  await expect(page).toHaveURL(/\/autonomous$/);
  await page.screenshot({path:`test-results/controls-expanded-ko-${info.project.name}.png`,fullPage:true});
  await page.goBack();
  await expectProcessingMode(page,'온디바이스');
  await page.getByRole('button',{name:'다크 모드로 전환'}).click();
  await page.screenshot({path:`test-results/controls-dark-ko-${info.project.name}.png`,fullPage:true});
 }finally{
  if(id){await request.post(`/api/invisible/sessions/${id}/stop`);await request.delete(`/api/rooms/${id}`);}
  other?.close();mac.close();
 }
});

test('polished workspace saves AI modes to the backend and restores them on another device',async({page,browser,request},info)=>{
 await page.goto('/');
 await expectProcessingMode(page,'On-device');
 await expect(page.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','false');
 const group=await openProcessingSettings(page);
 await expect(group.getByRole('button',{name:/On-device/})).toHaveAttribute('aria-pressed','true');
 await group.getByRole('button',{name:/Hybrid/}).click();
 await expect(group.getByRole('button',{name:/Hybrid/})).toHaveAttribute('aria-pressed','true');
 expect((await(await request.get('/api/ai/preferences')).json()).mode).toBe('hybrid');
 const context=await browser.newContext();const second=await context.newPage();await second.goto(baseURL+'/');
 await expectProcessingMode(second,'Hybrid');await context.close();
 await group.getByRole('button',{name:/Cloud AI/}).click();await expect(group.getByRole('button',{name:/Cloud AI/})).toHaveAttribute('aria-pressed','true');
 await group.getByRole('button',{name:/On-device/}).click();await expect(group.getByRole('button',{name:/On-device/})).toHaveAttribute('aria-pressed','true');
 await page.goBack();
 await expectProcessingMode(page,'On-device');
 // The composer, not a banner, leads New thought; the Autonomous AI card stays compact above the dock.
 await expect(page.locator('.invisible-hero')).toHaveCount(0);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await expect(page.getByTestId("connection-activity")).toHaveCount(0);
 await page.evaluate(()=>window.scrollTo(0,0));
 await page.screenshot({path:`test-results/invisible-ai-${info.project.name}.png`,fullPage:true});
 await page.getByRole('button',{name:'Switch to dark mode'}).click();
 await page.evaluate(()=>window.scrollTo(0,0));
 await page.screenshot({path:`test-results/invisible-ai-dark-${info.project.name}.png`,fullPage:true});
});

test('Auto answers ordinary note text once, stops explicitly, and stays off after disconnect',async({page,request})=>{
 const mac=await fakeMac();
 try {
  await page.goto('/');await openConversationOptions(page);await expect(page.getByRole('combobox',{name:'Chat room'})).not.toHaveValue('');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');await expect(auto).toBeEnabled();
  expect(mac.commands).toHaveLength(0);await auto.click();await expect(auto).toHaveAttribute('aria-pressed','true');
  await expect.poll(()=>mac.text).toContain('[End Invisible AI]');
  await expect(page.locator('.notes-progress')).toContainText('Answer added');
  await expect(page.locator('.from-ai .chat-message-meta').last()).toContainText('Local AI · apple ·');
  await expect(page.locator('.from-ai [data-testid="chat-llm-log"]').last()).toHaveText(/Answered by Local LLM · apple · system/);
  await expect(page.locator('.from-ai [data-testid="chat-usage"]').last()).toContainText('No cloud cost');
  await expect(page.getByLabel('Chat room',{exact:true})).toHaveValue(await page.getByLabel('Invisible AI session',{exact:true}).inputValue());
  await auto.click();await expect(auto).toHaveAttribute('aria-pressed','false');
  expect(mac.commands.filter(v=>v.operation==='append')).toHaveLength(1);
  expect(mac.commands.some(v=>v.operation==='stop')).toBeTruthy();
  await openWorkspace(page);
  const room=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  await auto.click();await expect(auto).toHaveAttribute('aria-pressed','true');mac.close();
  await expect.poll(async()=>(await(await request.get(`/api/rooms/${room}/notes`)).json()).active).toBe(false);
  await expect(auto).toHaveAttribute('aria-pressed','false');
 }finally{mac.close();}
});

test('Hybrid displays the exact note and requires a one-time cloud decision',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{hybrid_confirm:true}});
 await request.patch('/api/ai/preferences',{data:{...mode,mode:'hybrid'}});
 const mac=await fakeMac();
 try{
  await page.goto('/');await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); await page.getByTestId('textedit-auto-toggle').click();
  const card=page.getByRole('region',{name:'Cloud approval'});
  await expect(card).toBeVisible();await expect(card.locator('pre')).toHaveText(mac.text);
  expect(mac.commands.filter(v=>v.operation==='append')).toHaveLength(0);
  await card.getByRole('button',{name:'Approve once'}).click();
  await expect.poll(()=>mac.text).toContain('[Invisible AI · cloud approved]');await expect(card).toHaveCount(0);
  await openWorkspace(page); await page.getByTestId('textedit-auto-toggle').click();
  await expect(page.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','false');
 }finally{mac.close();}
});

test('a replacement CLI connection turns Auto off before any note commands reach it',async({page,request})=>{
 const mac=await fakeMac();let replacement;
 try{
  await page.goto('/');await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); await page.getByTestId('textedit-auto-toggle').click();
  await expect(page.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','true');
  await openWorkspace(page);
  const room=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  replacement=await fakeMac(mac.id);
  await expect.poll(async()=>(await(await request.get(`/api/rooms/${room}/notes`)).json()).active).toBe(false);
  expect(replacement.commands).toHaveLength(0);
 }finally{mac.close();replacement?.close();}
});


test('headless sessions continue with no UI and can be managed from another browser',async({page,browser,request})=>{
 const mac=await fakeMac();let id;let context;
 try{
  const response=await request.post('/api/invisible/sessions',{data:{client_id:mac.id,title:'Headless notes'}});
  expect(response.ok()).toBeTruthy();id=(await response.json()).id;
  expect(mac.commands).toHaveLength(0);
  expect((await request.post(`/api/invisible/sessions/${id}/start`)).ok()).toBeTruthy();
  await expect.poll(()=>mac.text).toContain('[End Invisible AI]');
  await page.goto('/');
  const picker=page.getByRole('combobox',{name:'Invisible AI session'});
  await expect(picker.locator(`option[value="${id}"]`)).toHaveCount(1);
  await picker.selectOption(id);
  await expect(page.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','true');
  await page.close();
  mac.text+='Explain the next idea';
  await expect.poll(()=>mac.text.match(/\[End Invisible AI\]/g)?.length||0).toBe(2);
  context=await browser.newContext();const second=await context.newPage();
  await second.goto(baseURL+'/');
  await openWorkspace(second); await second.getByRole('combobox',{name:'Invisible AI session'}).selectOption(id);
  await expect(second.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','true');
  await openWorkspace(second); await second.getByTestId('textedit-auto-toggle').click();
  await expect(second.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','false');
  expect((await(await request.get(`/api/invisible/sessions/${id}`)).json()).status.active).toBe(false);
  expect(mac.commands.filter(v=>v.operation==='append')).toHaveLength(2);
 }finally{
  if(id)await request.post(`/api/invisible/sessions/${id}/stop`).catch(()=>{});
  await context?.close();mac.close();
 }
});

test('new session is created by the backend and remains paused until explicitly started',async({page,request})=>{
 const mac=await fakeMac();let id;
 try{
  await page.goto('/');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await page.getByRole('link',{name:'Manage sessions',exact:true}).click();
  await page.getByRole('button',{name:'New session',exact:true}).click();
  await page.getByRole('textbox',{name:'Session name'}).fill('Created in Cloud');
  await page.getByRole('combobox',{name:'Capture computer (optional)'}).selectOption(mac.id);
  await page.getByRole('button',{name:'Create session',exact:true}).click();
  const card=page.getByRole('article',{name:'Created in Cloud',exact:true});
  await expect(card).toBeVisible();id=await card.getAttribute('data-session-id');
  await page.goto('/');
  const picker=page.getByRole('combobox',{name:'Invisible AI session'});
  await picker.selectOption(id);await expect(picker).toHaveValue(id);
  const session=await(await request.get(`/api/invisible/sessions/${id}`)).json();
  expect(session.client_id).toBe(mac.id);expect(session.status.active).toBe(false);
  expect(mac.commands).toHaveLength(0);
  await page.reload();await expect(picker).toHaveValue(id);
  await expect(page.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','false');
 }finally{mac.close();}
});

test('New thought starts separate context and the original session remains selectable and stoppable', async ({page,request}) => {
 const mac=await fakeMac();let id;
 try {
  await page.goto('/');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');
  await auto.click();await expect(auto).toHaveAttribute('aria-pressed','true');
  id=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  await expect.poll(()=>mac.text).toContain('[End Invisible AI]');
  const previous=await page.getByLabel('Chat room',{exact:true}).inputValue();
  const navigation=page.getByRole('button',{name:'Open navigation',exact:true});
  if(await navigation.isVisible())await navigation.click();
  await page.getByRole('button',{name:'New thought',exact:true}).click();
  await expect(page.getByLabel('Chat room',{exact:true})).not.toHaveValue(previous);
  await expect(page.getByRole('combobox',{name:'Invisible AI session'})).not.toHaveValue(id);
  await expect(auto).toHaveAttribute('aria-pressed','false');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Invisible AI session'}).selectOption(id);
  await expect(page.getByLabel('Chat room',{exact:true})).toHaveValue(id);
  await expect(auto).toHaveAttribute('aria-pressed','true');
  await auto.click();await expect(auto).toHaveAttribute('aria-pressed','false');
  expect((await(await request.get(`/api/invisible/sessions/${id}`)).json()).status.active).toBe(false);
  expect(mac.commands.some(v=>v.operation==='stop')).toBeTruthy();
 }finally{if(id)await request.post(`/api/invisible/sessions/${id}/stop`);mac.close();}
});

test('clearing the session chatroom stops Auto and revokes its Hybrid approval', async ({page,request}) => {
 await request.patch('/api/assistance/preferences',{data:{hybrid_confirm:true}});
 await request.patch('/api/ai/preferences',{data:{...mode,mode:'hybrid'}});
 const mac=await fakeMac();let id;
 try {
  await page.goto('/');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); await page.getByTestId('textedit-auto-toggle').click();
  const approval=page.getByRole('region',{name:'Cloud approval'});
  await expect(approval).toBeVisible();
  id=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  const before=await(await request.get(`/api/invisible/sessions/${id}`)).json();
  // The session and current chatroom share the same history.
  await page.getByLabel('Chat room',{exact:true}).selectOption(id);
  await page.getByRole('button',{name:'Clear chatroom',exact:true}).click();
  await page.getByRole('button',{name:'Confirm clear chatroom',exact:true}).click();
  await expect.poll(async()=>(await(await request.get(`/api/rooms/${id}/notes`)).json()).active).toBe(false);
  const navigation=page.getByRole('button',{name:'Open navigation',exact:true});
  if(await navigation.isVisible())await navigation.click();
  await page.getByRole('button',{name:'New thought',exact:true}).click();
  await expect(page.getByTestId('textedit-auto-toggle')).toHaveAttribute('aria-pressed','false');
  await expect(approval).toHaveCount(0);
  const replay=await request.post(`/api/rooms/${id}/notes/approval`,{data:{id:before.status.approval.id,approve:true}});
  expect(replay.status()).toBe(400);
  await expect.poll(()=>mac.commands.some(v=>v.operation==='stop')).toBe(true);
  expect(mac.commands.filter(v=>v.operation==='append')).toHaveLength(0);
  expect((await(await request.get('/api/invisible/sessions')).json()).find(s=>s.id===id)).toMatchObject({id,client_id:mac.id,capture_count:0});
 }finally{mac.close();}
});

test('the same question written below an answer receives another answer without looping', async ({page,request}) => {
 const mac=await fakeMac();const question=mac.text;let id;
 try {
  await page.goto('/');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');await auto.click();
  await expect(auto).toHaveAttribute('aria-pressed','true');
  id=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  await expect.poll(()=>mac.commands.filter(v=>v.operation==='append').length).toBe(1);
  mac.text+=question;
  await expect.poll(()=>mac.commands.filter(v=>v.operation==='append').length).toBe(2);
  // Observe two further actual polls rather than relying on a fixed UI delay.
  const reads=()=>mac.commands.filter(v=>v.operation==='read').length;
  const after=reads();await expect.poll(reads).toBeGreaterThanOrEqual(after+2);
  expect(mac.commands.filter(v=>v.operation==='append')).toHaveLength(2);
  await auto.click();await expect(auto).toHaveAttribute('aria-pressed','false');
 }finally{if(id)await request.post(`/api/invisible/sessions/${id}/stop`);mac.close();}
});

test('quick Auto uses the saved session Mac and explains offline, busy and missing AI setup',async({page,request})=>{
 let mac=await fakeMac();let aiMissing=false;
 try {
  const create=async title=>{
   const response=await request.post('/api/invisible/sessions',{data:{client_id:mac.id,title}});
   expect(response.ok()).toBe(true);return response.json();
  };
  const first=await create('Quick controls first'),second=await create('Quick controls second');
  await page.route('**/api/settings',async route=>{
   const response=await route.fetch(),settings=await response.json();
   if(aiMissing){settings.demo=false;for(const p of settings.providers)p.configured=false;}
   await route.fulfill({response,json:settings});
  });
  await page.goto('/');
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');
  const select=page.getByLabel('Invisible AI session', {exact:true});
  await select.selectOption(first.id);
  await expect(auto).toBeEnabled();
  mac.close();
  await expect.poll(async()=>(await(await request.get('/api/clients')).json()).find(c=>c.id===mac.id)?.online).toBe(false);
  await expect(auto).toBeDisabled();
  await expect(page.locator('.notes-start-hint')).toContainText('Reconnect this capture Mac to start the session.');
  mac=await fakeMac(mac.id);
  await expect(auto).toBeEnabled();
  expect((await request.post(`/api/invisible/sessions/${second.id}/start`)).ok()).toBe(true);
  await expect(auto).toBeDisabled();
  await expect(page.locator('.notes-start-hint')).toContainText('Another session is running on this Mac. Stop it first.');
  await request.post(`/api/invisible/sessions/${second.id}/stop`);
  await expect(auto).toBeEnabled();
  aiMissing=true;await page.reload();await select.selectOption(first.id);
  await expect(auto).toBeDisabled();
  await expect(page.locator('.notes-start-hint')).toContainText('Enable the selected AI provider in Backend settings before starting.');
  aiMissing=false;await page.reload();await select.selectOption(first.id);await auto.click();
  await expect(auto).toHaveAttribute('aria-pressed','true');
  aiMissing=true;await page.reload();
  await expect(auto).toHaveAttribute('aria-pressed','true');
  await expect(auto).toBeEnabled();await auto.click();
  await expect(auto).toHaveAttribute('aria-pressed','false');
 } finally {
  for(const session of await(await request.get('/api/invisible/sessions')).json()) await request.post(`/api/invisible/sessions/${session.id}/stop`);
  mac.close();
 }
});

test('Cloud shows which LLM ran, where it ran, how long it waited, and every step',async({page,request})=>{
 const mac=await fakeMac(undefined,'Progress Mac');let id;
 try{
  await page.goto('/');
  await openWorkspace(page); await page.getByRole('combobox',{name:'Active capture device',exact:true}).selectOption(mac.id);
  await openWorkspace(page); const auto=page.getByTestId('textedit-auto-toggle');await auto.click();await expect(auto).toHaveAttribute('aria-pressed','true');
  id=await page.getByRole('combobox',{name:'Invisible AI session'}).inputValue();
  await expect.poll(()=>mac.text).toContain('[End Invisible AI]');
  await page.goto('/autonomous');
  const card=page.locator(`[data-session-id="${id}"]`);
  const progress=card.getByTestId('session-progress');
  await expect(progress).toBeVisible();
  await expect(progress.locator('.cloud-now')).toHaveClass(/is-running/);
  await expect(progress.locator('.cloud-timeline')).toContainText('Thinking about your note…');
  await expect(progress.locator('.cloud-timeline')).toContainText('Answer added. Keep writing below it.');
  await expect(progress.locator('.cloud-timeline .ai-location.is-local').first()).toHaveText('Local LLM');
  await expect(progress.locator('.cloud-timeline .cloud-llm').first()).toContainText('Apple local');
  const table=progress.getByRole('table',{name:'Recent AI requests'});
  await expect(table.locator('tbody tr')).toHaveCount(1);
  await expect(table.locator('tbody tr').first()).toContainText('Apple local');
  await expect(table.locator('tbody tr').first()).toContainText('No cloud cost');
  await expect(table.locator('tbody tr').first()).toContainText('Complete');
  await expect(table.locator('tbody tr').first()).toContainText('— / —');
  await expect(progress.locator('.cloud-totals')).toContainText('AI requests1');
  const stats=page.getByTestId('processing-stats');
  await expect(stats.getByTestId('stats-local-total').locator('strong')).toHaveText(/^[1-9]\d*$/);
  await expect(stats.getByTestId('stats-local-total')).toContainText('Apple local');
  await expect(stats.getByTestId('stats-cloud-total').locator('strong')).toHaveText('0');
  await expect(stats).toContainText('Updated');
  const summary=await(await request.get('/api/activity/summary')).json();
  expect(summary.total.local.requests).toBeGreaterThanOrEqual(1);
  expect(summary.day.local.providers[0][0]).toBe('apple');
  expect(summary.total.cloud.requests).toBe(0);
  const api=await(await request.get(`/api/invisible/sessions/${id}`)).json();
  expect(api.activity.inferences[0]).toMatchObject({provider:'apple',location:'local',status:'complete',estimated_usd:null,input_tokens:null});
  expect(api.activity.inferences[0].elapsed_ms).toBeGreaterThan(0);
  expect(api.activity.entries.map(e=>e.stage)).toEqual(expect.arrayContaining(['Connecting to TextEdit, Notes or Terminal…','Thinking about your note…','Answer added. Keep writing below it.']));
  expect(api.activity.entries.find(e=>e.stage==='Thinking about your note…')).toMatchObject({provider:'apple',model:'system',location:'local',source:'textedit'});
  expect(api.status.provider).toBe('apple');
  await card.getByRole('button',{name:'Stop autonomous AI',exact:true}).click();
  await expect(progress.locator('.cloud-now')).not.toHaveClass(/is-running/);
  await expect(progress.locator('.cloud-now')).toContainText('Last reply took');
 }finally{if(id)await request.post(`/api/invisible/sessions/${id}/stop`).catch(()=>{});mac.close();}
});

test('Create and start suggests a name and begins TextEdit replies immediately',async({page,request})=>{
 const mac=await fakeMac(undefined,'Instant Mac');let id;
 try{
  await page.goto('/autonomous');
  await page.getByRole('button',{name:'New session',exact:true}).click();
  const name=page.getByRole('textbox',{name:'Session name'});
  const suggested=await name.inputValue();
  expect(suggested).toMatch(/^[A-Z][a-z]+ [A-Z][a-z]+ \d{2}$/);
  await page.getByRole('button',{name:'Suggest a name',exact:true}).click();
  await expect.poll(()=>name.inputValue()).toMatch(/^[A-Z][a-z]+ [A-Z][a-z]+ \d{2}$/);
  await page.getByRole('combobox',{name:'Capture computer (optional)'}).selectOption(mac.id);
  const start=page.getByRole('button',{name:'Create and start',exact:true});
  await expect(start).toBeEnabled();
  const title=await name.inputValue();
  await start.click();
  const card=page.getByRole('article',{name:title,exact:true});
  await expect(card).toBeVisible();id=await card.getAttribute('data-session-id');
  await expect(card.locator('.cloud-session-state')).toHaveText('Running');
  await expect.poll(()=>mac.text).toContain('[End Invisible AI]');
  await expect(card.getByTestId('session-progress').locator('.cloud-timeline')).toContainText('Answer added. Keep writing below it.');
  expect((await(await request.get(`/api/invisible/sessions/${id}`)).json()).status.active).toBe(true);
  // A second session for the same Mac cannot start while the first runs.
  await page.getByRole('button',{name:'New session',exact:true}).click();
  await page.getByRole('combobox',{name:'Capture computer (optional)'}).selectOption(mac.id);
  await expect(page.getByRole('button',{name:'Create and start',exact:true})).toBeDisabled();
  await expect(page.getByRole('button',{name:'Create session',exact:true})).toBeEnabled();
 }finally{if(id)await request.post(`/api/invisible/sessions/${id}/stop`).catch(()=>{});mac.close();}
});
