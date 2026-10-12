import {openConversationOptions} from './workspace-controls.mjs';
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Opt-in live-model behavior checks. Screens are synthetic, never the user's desktop.
test.use({baseURL:'http://127.0.0.1:3100'});
test.setTimeout(180000);
const cases = [
  {name:'coding quiz',text:'Coding quiz — Python\nWrite def add(a, b) that returns the sum of two numbers.\nExample: add(7, 5) returns 12.',kind:'code',check:async answer=>{
    await expect(answer.locator('pre code')).toContainText(/def add\(a, b\)/);
    await expect(answer.locator('pre code')).toContainText(/return a\s*\+\s*b/);
  }},
  {name:'math',text:'Math exercise\nSolve for x:\n3x + 5 = 26',kind:'question',check:async answer=>{
    await expect(answer).toContainText(/x\s*(?:=|is|equals)\s*7|x\s*\\?\)?\s*=\s*7/);
  }},
  {name:'idea',text:'My idea\nA shared office notebook app: all coworkers upload their entire screen every minute to a public gallery so nobody needs status meetings.\nGoal: help the team understand progress with less interruption.',kind:'question',check:async answer=>{
    await expect(answer).toContainText(/privacy|confidential|sensitive|consent/i);
    await expect(answer).toContainText(/instead|opt.in|private|pilot|experiment/i);
  }},
  {name:'messenger',text:'Project chat\nMina: Can you send the revised budget by Thursday?\nAlex: I am waiting for the vendor estimate.\nMina: Rough numbers are fine for now; we can update after the quote arrives.',kind:'reply',check:async answer=>{
    await expect(answer).toContainText('Thursday');
    await expect(answer).toContainText(/vendor|estimate|quote/i);
    await expect(answer).toContainText(/Mina/);
    await expect(answer).toContainText(/Alex/);
  }},
  {name:'photo question',text:'How many blue squares are in this picture?',visual:true,kind:'question',check:async answer=>{
    await expect(answer).toContainText(/^\s*(?:3|three)\b|(?:3|three)\s+(?:blue\s+)?squares|blue squares.*(?:3|three)/i);
  }},
];

for (const scenario of cases) {
  test(`Capture alone answers ${scenario.name}`, async ({page,browser,request}, testInfo) => {
    const settings=await (await request.get('/api/settings')).json();
    expect(settings.demo).toBe(false);
    expect(settings.providers.some(p=>p.id==='grok'&&p.configured), 'Configure XAI_API_KEY on the backend to run live behavior tests').toBe(true);
    const dir=fs.mkdtempSync(path.join(os.tmpdir(),'cw-capture-intent-'));
    const fixture=await browser.newPage({viewport:{width:1000,height:650}});
    const screenshot=path.join(dir,'screen.png');
    let cli;
    const calls=[];page.on('websocket',ws=>ws.on('framesent',frame=>{const v=JSON.parse(String(frame.payload));if(v.path==='/api/think')calls.push(v.body);}));
    try {
      await fixture.setContent('<html><head><style>body{margin:0;padding:45px;background:white;color:#111;font:28px/1.65 Arial}h1{font-size:28px;white-space:pre-wrap;font-weight:500}.shapes{display:flex;gap:26px;flex-wrap:wrap;padding-top:24px}.square{width:90px;height:90px;background:royalblue}.red{background:crimson}</style></head><body><h1></h1><div class="shapes"></div></body></html>');
      await fixture.locator('h1').evaluate((el,text)=>el.textContent=text,scenario.text);
      if(scenario.visual)await fixture.locator('.shapes').evaluate(el=>el.innerHTML='<div class="square"></div><div class="square red"></div><div class="square"></div><div class="square red"></div><div class="square"></div>');
      await fixture.screenshot({path:screenshot});
      const id=`intent-${testInfo.workerIndex}-${scenario.name.replaceAll(' ','-')}`;
      cli=spawn('./target/debug/rust-cli',['client','--backend','http://127.0.0.1:3100','--name',scenario.name,'--client-id',id,'--fixture-image',screenshot],{env:{...process.env,APP_TOKEN:'',CAUSEWAYBAY_CLIENT_CONFIG_DIR:dir},stdio:'ignore'});
      await expect.poll(async()=> (await (await request.get('/api/clients')).json()).some(c=>c.id===id&&c.online)).toBe(true);
      await page.goto('/');
      await expect(page.getByRole('combobox',{name:'Chat room'})).not.toHaveValue('');
      // Isolate the captured task from preceding cases in the fixed thought room.
      await page.getByRole('button',{name:'New chat',exact:true}).click();
      await expect(page).toHaveURL(/\/chat$/);
      await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toHaveValue('');
      await expect(page.getByRole('button',{name:'Image mode',exact:true})).toHaveAttribute('aria-pressed','true');
      await page.getByRole('combobox',{name:'Active capture device'}).selectOption(id);
      await openConversationOptions(page); await page.locator('.chat-options > summary').click();
      await page.getByLabel('Chat AI provider').selectOption('grok');
      await openConversationOptions(page); await page.locator('.chat-options > summary').click();
      await page.getByRole('button',{name:'Capture',exact:true}).click();
      await expect.poll(()=>calls.length).toBe(1);
      expect(calls[0]).toMatchObject({prompt:'',capture:true,input_mode:'image',client_id:id,agent:false});
      const room=calls[0].room_id;
      let result, memo;
      await expect.poll(async()=>{
        if(!result){
          const sync=await (await request.get(`/api/rooms/${room}/sync`)).json();
          result=sync.messages.find(m=>m.role==='assistant');
        }
        if(!result)return 'waiting';
        // Room sync is a paginated revision log; its first assistant record is running.
        memo=await (await request.get(`/api/memos/${result.memo_id}`)).json();
        if(memo.status==='error')throw new Error(memo.error||'AI response failed');
        return memo.status;
      },{timeout:150000}).toBe('complete');
      const answer=page.locator('.from-ai .answer-content').last();
      await scenario.check(answer);
      await expect(answer).not.toContainText(/what (?:would you like|do you want) me to do|would you like me to (?:solve|write|explain)/i);
      expect(memo.kind).toBe(scenario.kind);
      expect(memo.has_image).toBe(true);
      await testInfo.attach('answer',{body:memo.answer,contentType:'text/markdown'});
      await page.screenshot({path:`test-results/automatic-${scenario.name.replaceAll(' ','-')}.png`,fullPage:true});
    } finally {
      await fixture.close();
      if(cli&&cli.exitCode===null&&cli.signalCode===null){const done=once(cli,'exit');cli.kill('SIGTERM');await done;}
    }
  });
}

test('follow-up photos and comments use the original images from earlier messages', async ({page,browser,request},testInfo) => {
  const settings=await (await request.get('/api/settings')).json();
  expect(settings.providers.some(p=>p.id==='grok'&&p.configured)).toBe(true);
  const fixture=await browser.newPage({viewport:{width:480,height:240}});
  const photos=[];
  try {
    for(const [color,count,shape] of [['crimson',2,'circle'],['royalblue',3,'square'],['green',1,'triangle']]){
      await fixture.setContent(`<body style="margin:0;background:white;display:flex;align-items:center;justify-content:center;gap:24px;height:240px">${Array.from({length:count},()=>`<div style="width:70px;height:70px;background:${color};${shape==='circle'?'border-radius:50%;':shape==='triangle'?'clip-path:polygon(50% 0,100% 100%,0 100%);':''}"></div>`).join('')}</body>`);
      photos.push({name:`sample-${photos.length+1}.png`,mimeType:'image/png',buffer:await fixture.screenshot()});
    }
  } finally {await fixture.close();}
  const calls=[];
  page.on('websocket',ws=>ws.on('framesent',frame=>{const v=JSON.parse(String(frame.payload));if(v.path==='/api/think')calls.push(v.body);}));
  await page.goto('/');
  await expect(page.getByRole('combobox',{name:'Chat room'})).not.toHaveValue('');
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  await page.getByLabel('Chat AI provider').selectOption('grok');
  await openConversationOptions(page); await page.locator('.chat-options > summary').click();
  async function waitForAnswer(number){
    await expect.poll(()=>calls.length).toBe(number);
    const req=calls[number-1];let memoId,memo;
    await expect.poll(async()=>{
      if(!memoId){
        let after=0;
        for(;;){
          const batch=await (await request.get(`/api/rooms/${req.room_id}/sync?after=${after}`)).json();
          memoId=batch.messages.find(m=>m.messageid===req.message_id)?.memo_id;
          if(memoId||!batch.has_more)break;
          after=batch.next;
        }
      }
      if(!memoId)return 'waiting';
      memo=await (await request.get(`/api/memos/${memoId}`)).json();
      if(memo.status==='error')throw new Error(memo.error);
      return memo.status;
    },{timeout:120000}).toBe('complete');
    await testInfo.attach(`turn-${number}`,{body:memo.answer,contentType:'text/markdown'});
    return memo.answer;
  }
  await page.getByLabel('Attach to chat').setInputFiles(photos.slice(0,2));
  await expect(page.locator('.chat-attachment')).toHaveCount(2);
  await page.getByLabel('Chat message',{exact:true}).fill('The project code is APRICOT. These are our first two sample photos. For now reply only "Saved samples." Do not describe the photos yet.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  const first=await waitForAnswer(1);
  expect(first).not.toMatch(/red|blue|circle|square/i); // Later answers must actually revisit the photos.
  await page.getByLabel('Attach to chat').setInputFiles(photos.slice(2));
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await page.getByLabel('Chat message',{exact:true}).fill('Include this new photo with the previous two. Give our project code, then count the red circles, blue squares, and green triangles across all three photos. Use a short list with digits.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  const second=await waitForAnswer(2);
  expect(second).toContain('APRICOT');
  expect(second).toMatch(/red circles[^\n]*2|2[^\n]*red circles/i);
  expect(second).toMatch(/blue squares[^\n]*3|3[^\n]*blue squares/i);
  expect(second).toMatch(/green triangles?[^\n]*1|1[^\n]*green triangles?/i);
  await page.getByLabel('Chat message',{exact:true}).fill('Which color had the fewest shapes? Include our project code.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  const third=await waitForAnswer(3);
  expect(third).toContain('APRICOT');
  expect(third).toMatch(/green/i);
  await expect(page.locator('.from-user .chat-photo')).toHaveCount(3);
  await page.screenshot({path:'test-results/live-photo-context.png',fullPage:true});
});
