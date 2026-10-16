import {test, expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';
import {openConversationOptions} from './workspace-controls.mjs';

isolatedBackend(test, 3188);
test.beforeEach(async ({request}) => {
  expect((await request.delete('/api/user/data')).ok()).toBeTruthy();
});

test('everyday chat starts simple and keeps the answer above the phone composer', async ({page}, info) => {
  const errors=[]; page.on('pageerror', e => errors.push(e.message));
  await page.goto('/');
  await expect(page.locator('.everyday-workspace')).not.toHaveAttribute('open');
  await expect(page.locator('.conversation-options')).not.toHaveAttribute('open');
  await expect(page.getByRole('heading', {name:'Hey, what’s on your mind?'})).toBeVisible();
  await expect(page.locator('.companion-figure')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('textbox', {name:'Chat message',exact:true}).fill('Help me plan a lovely afternoon');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.chat-message.from-ai')).toBeVisible();
  await expect(page.locator('.chat-message.from-ai')).toContainText('demo mode');
  const bounds=await page.evaluate(() => {
    const answer=document.querySelector('.chat-message.from-ai').getBoundingClientRect();
    const dock=document.querySelector('.chat-dock').getBoundingClientRect();
    return {top:answer.top,bottom:answer.bottom,dock:dock.top};
  });
  expect(bounds.top).toBeGreaterThanOrEqual(0);
  expect(bounds.bottom).toBeLessThanOrEqual(bounds.dock+2);
  await page.locator('.conversation-options > summary').click();
  await openConversationOptions(page);
  await expect(page.getByRole('button',{name:'Multi Capture',exact:true})).toBeVisible();
  await page.screenshot({path:`test-results/everyday-${info.project.name}.png`,fullPage:true});
  expect(errors).toEqual([]);
});

test('a cloud model choice reaches a fresh browser through account settings', async ({page, browser, baseURL, request}) => {
  await request.patch('/api/ai/preferences',{data:{mode:'cloudai',local_provider:'apple',cloud_provider:'grok'}});
  await page.goto('/ai');
  const model=page.getByRole('textbox',{name:'Settings AI model',exact:true});
  await model.fill('everyday-test-model');
  await model.press('Tab');
  await expect.poll(async () => (await (await request.get('/api/settings')).json()).user_models.grok).toBe('everyday-test-model');
  const fresh=await browser.newContext();
  try {
    const other=await fresh.newPage();
    await other.goto(baseURL+'/ai');
    await expect(other.getByRole('textbox',{name:'Settings AI model',exact:true})).toHaveValue('everyday-test-model');
  } finally { await fresh.close(); }
});

test('Motion is local and reduced motion leaves the companion still', async ({page}) => {
  const external=[];
  page.on('request', r => {if(/^https?:/.test(r.url()) && !r.url().startsWith('http://127.0.0.1:')) external.push(r.url());});
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.goto('/');
  await expect(page.locator('.companion-figure')).toBeVisible();
  expect(await page.locator('.companion-figure').evaluate(el => el.getAnimations().length)).toBe(0);
  expect(await page.locator('script[src="/assets/everyday-motion.js"]').count()).toBe(1);
  expect(external).toEqual([]);
});

test('the companion uses Motion and stops immediately when Reduce Motion is enabled', async ({page}) => {
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.goto('/');
  await expect.poll(() => page.locator('.companion-figure').evaluate(el => getComputedStyle(el).transform)).not.toBe('none');
  await page.emulateMedia({reducedMotion:'reduce'});
  await expect.poll(() => page.locator('.companion-figure').evaluate(el => el.getAnimations().length)).toBe(0);
  await expect(page.locator('.companion-figure')).toHaveCSS('transform','none');
  await expect(page.locator('.companion-figure')).toBeVisible();
});

test('the main companion reacts to typing and keyboard greetings, and keeps a WebGL fallback', async ({page}) => {
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.goto('/');
  const buddy=page.locator('invisible-companion');
  await expect(buddy).toHaveAttribute('data-state','idle');
  await expect(buddy).toHaveAttribute('data-render',/webgl|fallback/);
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('An idea');
  await expect(buddy).toHaveAttribute('data-state','listening');
  await page.getByRole('button',{name:'Say hello to your companion'}).press('Enter');
  await expect(buddy).toHaveAttribute('data-greeting','true');
  await page.emulateMedia({reducedMotion:'reduce'});
  await expect(buddy).not.toHaveAttribute('data-greeting');
  await expect.poll(()=>buddy.locator('img').evaluate(el=>el.getAnimations().length)).toBe(0);
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  await expect(page.locator('invisible-companion')).toHaveCount(1);
});

test('character remains usable when WebGL is unavailable', async ({page}) => {
  await page.addInitScript(()=>{
    const original=HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext=function(type,...args){return type.startsWith('webgl') ? null : original.call(this,type,...args);};
  });
  await page.goto('/');
  await expect(page.locator('invisible-companion')).toHaveAttribute('data-render','fallback');
  await expect(page.locator('.companion-figure')).toBeVisible();
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Still working');
  await expect(page.locator('invisible-companion')).toHaveAttribute('data-state','listening');
});

test('the companion animates for the entire pending request and settles when it completes', async ({page}) => {
  await page.emulateMedia({reducedMotion:'no-preference'});
  let release;
  const waiting = new Promise(resolve=>{release=resolve;});
  await page.route('**/api/think', async route=>{await waiting; await route.continue();});
  await page.goto('/');
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('A tiny adventure');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  try {
    await expect(page.locator('invisible-companion')).toHaveAttribute('data-state','thinking');
    await expect.poll(()=>page.locator('.companion-figure').evaluate(el=>getComputedStyle(el).transform)).not.toBe('none');
  } finally { release(); }
  await expect(page.locator('.chat-message.from-ai')).toContainText('demo mode');
  await expect(page.locator('invisible-companion')).toHaveAttribute('data-state','idle');
});

test('opening the app shows the waiting companion until settings arrive', async ({page}) => {
  await page.emulateMedia({reducedMotion:'no-preference'});
  let release;
  const pending=new Promise(resolve=>{release=resolve;});
  await page.route('**/api/settings',async route=>{await pending; await route.continue();});
  await page.goto('/');
  try {
    const buddy=page.locator('.loading-state .wait-buddy');
    await expect(buddy).toBeVisible();
    await expect.poll(()=>buddy.evaluate(el=>getComputedStyle(el).transform)).not.toBe('none');
    await page.emulateMedia({reducedMotion:'reduce'});
    await expect(buddy).toHaveCSS('transform','none');
  } finally { release(); }
  await expect(page.locator('invisible-companion')).toBeVisible();
  await expect(page.locator('.loading-state .wait-buddy')).toHaveCount(0);
});

test('every main destination has its own companion and a plain-language next step', async ({page}) => {
  const errors=[]; page.on('pageerror',e=>errors.push(e.message));
  for(const route of ['wiki','autonomous','coding','skills','images','videos','devices','ai','assistance','files','library','dashboard']) {
    await page.goto('/'+route);
    await expect(page.locator('.page-companion')).toHaveAttribute('data-page',route);
    await expect(page.locator('invisible-companion')).toHaveCount(1);
    await expect(page.locator('.companion-figure')).toBeVisible();
    await expect(page.locator('.page-companion-copy p')).not.toBeEmpty();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  expect(errors).toEqual([]);
});

test('a helper starts with a simple choice while technical settings stay folded', async ({page}) => {
  await page.goto('/autonomous');
  await expect(page.locator('.simple-ai-settings')).not.toHaveAttribute('open');
  await expect(page.getByRole('group',{name:'AI processing mode'})).toBeHidden();
  await expect(page.locator('.usage-credit')).toHaveCount(0);
  await page.getByRole('button',{name:'Add a helper',exact:true}).click();
  await expect(page.getByRole('textbox',{name:'Name your helper'})).not.toBeEmpty();
  await expect(page.getByRole('combobox',{name:'Which Mac?'})).toBeVisible();
  await expect(page.getByRole('button',{name:'Start helping',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Cancel',exact:true}).click();
  await page.locator('.simple-ai-settings > summary').click();
  await expect(page.getByRole('group',{name:'AI processing mode'})).toBeVisible();
});

test('actual AI location follows every page and never calls a remote service on-device', async ({page}) => {
  await page.addInitScript(() => {
    const Native = window.EventSource;
    window.EventSource = class extends EventTarget {
      constructor(url, options) {
        super();
        if (url !== '/api/activity/events') return new Native(url, options);
        window.activitySource = this;
      }
      close() {}
    };
  });
  for (const route of ['/devices','/autonomous','/wiki','/']) {
    await page.goto(route);
    await page.waitForFunction(() => !!window.activitySource);
    const update = calls => page.evaluate(calls => window.activitySource.dispatchEvent(new MessageEvent('message', {data:JSON.stringify(calls)})), calls);
    await update([{provider:'mlx',model:'9B',location:'local'}]);
    await expect(page.locator('[data-location="local"]')).toContainText('processed on your Mac');
    await update([{provider:'grok',model:'cloud',location:'cloud'}]);
    await expect(page.locator('[data-location="cloud"]')).toContainText('sent to your AI provider');
    await expect(page.locator('[data-location="local"]')).toHaveCount(0);
    await update([{provider:'mlx',model:'27B',location:'service'}]);
    await expect(page.locator('[data-location="service"]')).toBeVisible();
    await update([]);
    await expect(page.locator('.ai-location')).toHaveCount(0);
    await update([{provider:'mlx',model:'9B',location:'local'}]);
    await page.evaluate(() => window.activitySource.dispatchEvent(new Event('error')));
    await expect(page.locator('.ai-location')).toHaveCount(0);
  }
});

test('three core actions keep text and files without a manual, and failed saves preserve the draft', async ({page,request}) => {
  await page.goto('/');
  if(await page.locator('.mobile-toggle').isVisible()) await page.locator('.mobile-toggle').click();
  const nav=page.getByRole('navigation',{name:'Main navigation',exact:true});
  await expect(nav.getByRole('button')).toHaveCount(4);
  await expect(nav.getByRole('button')).toHaveText(['Capture','Save anything','Ask anything','Wallet']);
  await nav.getByRole('button',{name:'Save anything',exact:true}).click();
  await expect(page).toHaveURL(/\/save$/);
  await expect(page.getByRole('button',{name:'Save it',exact:true})).toBeDisabled();
  await page.getByRole('textbox',{name:'What would you like to keep?'}).fill('Remember the tiny garden café. Bring a book.');
  await page.getByLabel('Add any file',{exact:true}).setInputFiles({name:'garden.txt',mimeType:'text/plain',buffer:Buffer.from('The garden café has a quiet reading corner.')});
  await expect(page.locator('.anything-files')).toContainText('garden.txt');
  const oversized='x'.repeat(100001);
  await page.getByRole('textbox',{name:'What would you like to keep?'}).fill(oversized);
  await page.getByRole('button',{name:'Save it',exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('100,000');
  await expect(page.getByRole('textbox')).toHaveValue(oversized);
  await expect(page.locator('.anything-files')).toContainText('garden.txt');
  await page.getByRole('textbox',{name:'What would you like to keep?'}).fill('Remember the tiny garden café. Bring a book.');
  await page.getByRole('button',{name:'Save it',exact:true}).click();
  await expect(page.locator('.anything-saved')).toBeVisible();
  const href=await page.getByRole('link',{name:'View saved item'}).getAttribute('href');
  const id=href.split('/').pop();
  await expect.poll(async()=> (await (await request.get(`/api/memos/${id}`)).json()).answer).toContain('tiny garden');
  const memo=await (await request.get(`/api/memos/${id}`)).json();
  expect(memo.input).toContain('quiet reading corner');
  // Text kept with Save anything is listed and searchable in Notes (More tools).
  await page.getByRole('link',{name:'Open Notes'}).click();
  await expect(page).toHaveURL(/\/notes$/);
  await expect(page.getByTestId('notes-page')).toBeVisible();
  await page.locator('#memo-search').fill('garden café');
  await expect(page.locator('.memo-card').filter({hasText:'tiny garden'})).toBeVisible();
  await page.locator('#memo-search').fill('zzqx-no-such-note');
  await expect(page.locator('.memo-card')).toHaveCount(0);
  if(await page.locator('.mobile-toggle').isVisible()) await page.locator('.mobile-toggle').click();
  if(!(await page.locator('[data-nav="wiki"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.getByRole('navigation',{name:'Additional tools',exact:true}).getByRole('button',{name:'Find anything',exact:true}).click();
  await expect(page).toHaveURL(/\/wiki$/);
  if(await page.locator('.mobile-toggle').isVisible()) await page.locator('.mobile-toggle').click();
  await nav.getByRole('button',{name:'Ask anything',exact:true}).click();
  await expect(page).toHaveURL(/\/chat(\/|$)/);
  if(await page.locator('.mobile-toggle').isVisible()) await page.locator('.mobile-toggle').click();
  await nav.getByRole('button',{name:'Capture',exact:true}).click();
  await expect(page.getByRole('main').getByRole('button',{name:'Capture',exact:true})).toBeVisible();
});

test('on-device 9B and 27B choices persist to the account and a fresh browser', async ({page,browser,request}) => {
  await request.patch('/api/ai/preferences',{data:{mode:'ondevice',local_provider:'mlx',cloud_provider:'grok'}});
  await page.goto('/ai');
  const model=page.getByRole('combobox',{name:'Settings On-device model',exact:true});
  await expect(model).toBeVisible();
  await expect(model).toContainText('Recommended for 16 GB');
  await model.selectOption('mlx-community/Qwen3.8-27B-4bit');
  await expect.poll(async()=> (await (await request.get('/api/settings')).json()).user_models?.mlx).toBe('mlx-community/Qwen3.8-27B-4bit');
  const context=await browser.newContext();
  try {
    const fresh=await context.newPage();
    await fresh.goto(new URL('/ai',page.url()).href);
    await expect(fresh.getByRole('combobox',{name:'Settings On-device model',exact:true})).toHaveValue('mlx-community/Qwen3.8-27B-4bit');
    await fresh.getByRole('combobox',{name:'Settings On-device model',exact:true}).selectOption('mlx-community/Qwen3.5-9B-4bit');
    await expect.poll(async()=> (await (await request.get('/api/settings')).json()).user_models?.mlx).toBe('mlx-community/Qwen3.5-9B-4bit');
  } finally {await context.close();}
});
