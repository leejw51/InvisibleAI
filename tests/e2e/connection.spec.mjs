import {test,expect} from '@playwright/test';
import {openConversationOptions} from './workspace-controls.mjs';

test('opening animation settles into a real connected confirmation',async({page},testInfo)=>{
  await page.emulateMedia({reducedMotion:'no-preference'});
  let release;
  const gate=new Promise(resolve=>release=resolve);
  await page.route('**/api/auth/info',async route=>{await gate;await route.continue();});
  try {
    await page.goto('/');
    const splash=page.getByTestId('connection-splash');
    await expect(splash).toBeVisible();
    const orb=splash.locator('.connection-orb');
    await expect(orb).toBeVisible();
    await expect.poll(()=>orb.evaluate(el=>el.complete&&el.naturalWidth>0)).toBe(true);
    // The redesign's companion sculpture floats with `companion-arriving` (styles.css overrides connection-float).
    expect(await orb.evaluate(el=>getComputedStyle(el).animationName)).toBe('companion-arriving');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await expect.poll(()=>splash.locator('.connection-stage').evaluate(el=>getComputedStyle(el).opacity)).toBe('1');
    await page.screenshot({path:`test-results/connection-opening-${testInfo.project.name}.png`,fullPage:true});
    release();
    const badge=page.getByRole('button',{name:'Backend connection',exact:true});
    await expect(badge).toHaveText('Connected');
    const activity=page.getByTestId('connection-activity');
    await expect(activity).toHaveAttribute('data-state','connected');
    await expect(activity).toContainText('Your workspace is ready.');
    await expect(activity.locator('.connection-seal')).toBeVisible();
    expect(await activity.evaluate(el=>getComputedStyle(el).pointerEvents)).toBe('none');
    await expect.poll(()=>activity.locator('.connection-seal path').evaluate(el=>getComputedStyle(el).strokeDashoffset)).toBe('0px');
    await page.screenshot({path:`test-results/connection-ready-${testInfo.project.name}.png`,fullPage:true});
    await expect(activity).toHaveCount(0,{timeout:6500});
    expect(await badge.locator('.connection-signal>span').evaluate(el=>getComputedStyle(el).animationName)).toBe('connection-heartbeat');
    await badge.click();
    await expect(page.getByRole('heading',{name:'Your connected devices.'})).toBeVisible();
  } finally {release();}
});

test('reconnecting preserves drafts and confirms recovery without repeating on navigation',async({page},testInfo)=>{
  await page.emulateMedia({reducedMotion:'no-preference'});
  let available=true, connection, interruptPoll=false, pendingPoll=false;
  await page.routeWebSocket('**/api/ws',ws=>{
    if(!available){void ws.close();return;}
    connection={ws,server:ws.connectToServer()};
    const {server}=connection;
    ws.onMessage(raw=>{
      const message=JSON.parse(String(raw));
      if(interruptPoll && message.method==='GET' && message.path?.endsWith('/capture')) {
        pendingPoll=true;
      } else server.send(raw);
    });
  });
  await page.goto('/');
  const badge=page.getByRole('button',{name:'Backend connection',exact:true});
  const activity=page.getByTestId('connection-activity');
  await expect(badge).toHaveText('Connected');
  await expect(activity).toHaveCount(0,{timeout:6500});
  const draft=page.locator('.chat-dock textarea');
  await draft.fill('Keep this thought through a reconnect.');
  // A disconnect while the periodic status request is pending used to leave
  // its error in Chat even after the global connection badge recovered.
  interruptPoll=true;
  await expect.poll(()=>pendingPoll).toBe(true);
  available=false;
  await connection.ws.close();await connection.server.close();
  await expect(badge).toHaveText('Reconnecting');
  await expect(activity).toHaveAttribute('data-state','reconnecting');
  await expect(activity).toContainText('Finding our way back…');
  await expect(activity.locator('.connection-seal')).toHaveCount(0);
  const error=page.locator('.chat-page > .error-banner[role="alert"]');
  await expect(error).toHaveText('Connection interrupted. Reconnecting to your backend…');
  await expect(draft).toHaveValue('Keep this thought through a reconnect.');
  await draft.fill('Still writing while reconnecting.');
  await expect.poll(()=>activity.evaluate(el=>getComputedStyle(el).opacity)).toBe('1');
  await page.screenshot({path:`test-results/connection-retrying-${testInfo.project.name}.png`,fullPage:true});
  interruptPoll=false;
  available=true;
  await expect(badge).toHaveText('Connected');
  await expect(activity).toContainText('Back in sync.');
  await expect(error).toHaveCount(0);
  await expect(draft).toHaveValue('Still writing while reconnecting.');
  await expect(activity).toHaveCount(0,{timeout:6500});
  await badge.click();
  await expect(page.getByRole('heading',{name:'Your connected devices.'})).toBeVisible();
  await expect(activity).toHaveCount(0);
});

test('recovery preserves unrelated chat errors',async({page})=>{
  let available=true, connection;
  await page.routeWebSocket('**/api/ws',ws=>{
    if(!available){void ws.close();return;}
    connection={ws,server:ws.connectToServer()};
  });
  await page.goto('/');
  const badge=page.getByRole('button',{name:'Backend connection',exact:true});
  await expect(badge).toHaveText('Connected');
  await expect(page.getByLabel('Attach to chat')).toBeEnabled();
  available=false;
  await connection.ws.close();await connection.server.close();
  await expect(badge).toHaveText('Reconnecting');
  await page.getByLabel('Attach to chat').setInputFiles(Array.from({length:9},(_,i)=>({
    name:`attachment-${i}.txt`,mimeType:'text/plain',buffer:Buffer.from('draft'),
  })));
  const error=page.locator('.chat-page > .error-banner[role="alert"]');
  await expect(error).toHaveText('Attach up to 8 photos or files per message');
  available=true;
  await expect(badge).toHaveText('Connected');
  await expect(error).toHaveText('Attach up to 8 photos or files per message');
});

test('an interrupted message keeps its draft and retry after recovery',async({page})=>{
  let available=true, connection, interruptSend=true;
  const submissions=[];
  await page.routeWebSocket('**/api/ws',ws=>{
    if(!available){void ws.close();return;}
    const server=ws.connectToServer();
    connection={ws,server};
    ws.onMessage(raw=>{
      const message=JSON.parse(String(raw));
      if(message.path==='/api/think') {
        submissions.push(message.body);
        if(interruptSend)return;
      }
      server.send(raw);
    });
  });
  await page.goto('/');
  const badge=page.getByRole('button',{name:'Backend connection',exact:true});
  await expect(badge).toHaveText('Connected');
  // Each browser project shares the demo server, so use a fresh conversation
  // when asserting that an interrupted submission creates exactly one message.
  await openConversationOptions(page);
  const room=page.getByRole('combobox',{name:'Chat room',exact:true});
  await expect(room).not.toHaveValue('');
  const previousRoom=await room.inputValue();
  // "New chat" opens a new conversation on Ask anything (/chat); the room
  // picker folds under "Conversation options" again once the room arrives.
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  await openConversationOptions(page);
  await expect(room).not.toHaveValue(previousRoom);
  await expect(room).not.toHaveValue('');
  const draft=page.locator('.chat-dock textarea');
  await draft.fill('Retry this thought after the connection returns.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect.poll(()=>submissions.length).toBe(1);
  available=false;
  await connection.ws.close();await connection.server.close();
  await expect(badge).toHaveText('Reconnecting');
  const error=page.locator('.chat-page > .error-banner[role="alert"]');
  await expect(error).toContainText('Connection interrupted. Reconnecting to your backend…');
  interruptSend=false;
  available=true;
  await expect(badge).toHaveText('Connected');
  await expect(error).toContainText('Message delivery could not be confirmed. Please retry.');
  await expect(error).not.toContainText('Reconnecting');
  await expect(draft).toHaveValue('Retry this thought after the connection returns.');
  expect(submissions).toHaveLength(1);
  await page.getByRole('combobox',{name:'Language',exact:true}).selectOption('ko');
  await expect(error).toContainText('메시지 전송 여부를 확인하지 못했습니다. 다시 보내 주세요.');
  await page.getByRole('button',{name:'메시지 다시 보내기',exact:true}).click();
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(draft).toHaveValue('');
  await expect(error).toHaveCount(0);
  expect(submissions).toHaveLength(2);
  expect(submissions[1].message_id).toBe(submissions[0].message_id);
});

test('connection art supports dark mode, mobile sizes, and reduced motion',async({page},testInfo)=>{
  await page.setViewportSize({width:390,height:844});
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(()=>localStorage.setItem('causewaybay.theme.v1','dark'));
  let release;
  const gate=new Promise(resolve=>release=resolve);
  await page.route('**/api/auth/info',async route=>{await gate;await route.continue();});
  try {
    await page.goto('/');
    const splash=page.getByTestId('connection-splash');
    await expect(splash).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
    for(const selector of ['.connection-orb','.connection-orbit','.connection-aura','.connection-ellipsis i','.journey-thread i']) {
      const names=await splash.locator(selector).evaluateAll(els=>els.map(el=>getComputedStyle(el).animationName));
      expect(names.every(name=>name==='none')).toBe(true);
    }
    await page.screenshot({path:`test-results/connection-dark-${testInfo.project.name}.png`,fullPage:true});
    release();
    const badge=page.getByRole('button',{name:'Backend connection',exact:true});
    await expect(badge).toHaveText('Connected');
    const activity=page.getByTestId('connection-activity');
    await expect(activity).toHaveAttribute('data-state','connected');
    expect(await activity.locator('.connection-seal path').evaluate(el=>getComputedStyle(el).strokeDashoffset)).toBe('0px');
    expect(await badge.locator('.connection-signal>span').evaluate(el=>getComputedStyle(el).animationName)).toBe('none');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`test-results/connection-dark-ready-${testInfo.project.name}.png`,fullPage:true});
  } finally {release();}
});
