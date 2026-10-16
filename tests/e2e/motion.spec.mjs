import {test,expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';
import {openWorkspace} from './workspace-controls.mjs';

isolatedBackend(test,3158);
test.use({reducedMotion:'no-preference'});

test.beforeEach(async({page,request})=>{
 expect((await request.delete('/api/user/data')).ok()).toBeTruthy();
 expect((await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true,wiki_auto_update:true}})).ok()).toBeTruthy();
 await page.addInitScript(()=>{
  window.uiMotion=[];
  document.addEventListener('animationstart',event=>{
   if(!event.animationName.startsWith('ui-'))return;
   window.uiMotion.push({target:event.target,name:event.animationName,animation:event.target.getAnimations().find(a=>a.animationName===event.animationName)});
  });
 });
});

test('cards tween through intermediate frames, stagger once, and retain identity during refresh',async({page,request})=>{
 let completedReads=0;
 await page.routeWebSocket('**/api/ws',ws=>{
  const server=ws.connectToServer(),reads=new Set();
  ws.onMessage(raw=>{const m=JSON.parse(String(raw));if(m.method==='GET'&&m.path?.startsWith('/api/wiki?'))reads.add(m.id);server.send(raw);});
  server.onMessage(raw=>{const m=JSON.parse(String(raw));if(reads.delete(m.id)&&m.status===200)completedReads++;ws.send(raw);});
 });
 for(const topic of ['bitcoin','design','writing']){
  expect((await request.post('/api/memos',{data:{title:`A note about ${topic}`,content:`An idea worth keeping. #${topic}`}})).ok()).toBeTruthy();
 }
 await page.goto('/wiki');
 await expect(page.locator('.wiki-card')).toHaveCount(3);
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.filter(e=>e.target.matches('.wiki-card')).length)).toBe(3);
 const frames=await page.evaluate(()=>{
  const entries=window.uiMotion.filter(e=>e.target.matches('.wiki-card'));
  const {animation,target}=entries[0];
  const timings=entries.map(e=>e.animation.effect.getTiming());
  animation.pause();
  const samples=[0,.25,.5,.75,1].map(progress=>{
   animation.currentTime=timings[0].delay+timings[0].duration*progress;
   const style=getComputedStyle(target);
   return {opacity:Number(style.opacity),y:parseFloat(style.translate.split(' ')[1]||'0')};
  });
  animation.finish();
  window.retainedWikiCard=document.querySelector('[aria-label="A note about writing"]');
  return {samples,delays:timings.map(t=>t.delay),iterations:timings.map(t=>t.iterations),easing:getComputedStyle(target).animationTimingFunction};
 });
 expect(frames.delays).toEqual([0,35,70]);
 expect(frames.iterations).toEqual([1,1,1]);
 expect(frames.easing).toContain('linear(');
 // Expo.InOut is slow at both ends, symmetric around the midpoint.
 for(const [i,expected] of [0,.015625,.5,.984375,1].entries()){
  expect(frames.samples[i].opacity).toBeCloseTo(expected,4);
  expect(frames.samples[i].y).toBeCloseTo(10*(1-expected),3);
 }
 // A real backend refresh (the toolbar button; live changes arrive the same way)
 // must not replay entrance effects.
 const initialReads=completedReads;
 await page.getByTestId('wiki-refresh').click();
 await expect.poll(()=>completedReads,{timeout:8000}).toBeGreaterThan(initialReads);
 expect(await page.evaluate(()=>window.uiMotion.filter(e=>e.target.matches('.wiki-card')).length)).toBe(3);
 expect(await page.evaluate(()=>document.querySelector('[aria-label="A note about writing"]')===window.retainedWikiCard)).toBe(true);
 // A user-requested filter can reveal a new result set; it must remain usable.
 await page.getByRole('searchbox',{name:'Search Wiki'}).fill('writing');
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await page.locator('.wiki-card .wiki-tag').filter({hasText:'#writing'}).click();
 await page.getByTestId('wiki-open-document').click();
 await expect(page.getByRole('article',{name:'writing',exact:true})).toBeFocused();
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.some(e=>e.target.matches('.wiki-article')&&e.name==='ui-reveal'))).toBe(true);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});

test('controls ease into their states and confirm only a successful save',async({page,request})=>{
 let failSave=true;
 await page.routeWebSocket('**/api/ws',ws=>{
  const server=ws.connectToServer();
  ws.onMessage(raw=>{
   const m=JSON.parse(String(raw));
   if(failSave&&m.method==='PATCH'&&m.path==='/api/assistance/preferences')ws.send(JSON.stringify({type:'response',id:m.id,status:500,data:{error:'Test save failed'}}));
   else server.send(raw);
  });
 });
 await page.goto('/assistance');
 const toggle=page.getByRole('switch',{name:'Enable Wiki',exact:true});
 await toggle.uncheck();
 await expect(page.getByRole('status').filter({hasText:'Unsaved changes'})).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.some(e=>e.target.matches('.assistance-save')&&e.name==='ui-reveal'))).toBe(true);
 expect(await page.locator('#wiki-settings .setting-switch .switch-track').evaluate(el=>getComputedStyle(el,'::after').transitionTimingFunction)).toContain('linear(');
 const save=page.getByRole('button',{name:'Save assistance settings'});
 await save.click();await expect(page.getByRole('alert')).toBeVisible();
 await expect(page.locator('.assistance-save')).not.toHaveClass(/is-saved/);
 expect(await page.evaluate(()=>window.uiMotion.some(e=>e.name==='ui-confirm'))).toBe(false);
 expect((await(await request.get('/api/assistance/preferences')).json()).wiki_enabled).toBe(true);
 failSave=false;
 await save.click();
 await expect(page.getByRole('status').filter({hasText:'Saved to your account'})).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.some(e=>e.name==='ui-confirm'))).toBe(true);
 expect((await(await request.get('/api/assistance/preferences')).json()).wiki_enabled).toBe(false);
 await page.getByText('How your Wiki works',{exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.some(e=>e.name==='ui-detail'))).toBe(true);
 await page.goto('/');
 await openWorkspace(page);
 await expect(page.locator('.invisible-console')).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.some(e=>e.target.matches('.invisible-console')&&e.name==='ui-reveal'))).toBe(true);
});

test('reduced motion settles active tweens and keeps controls immediately usable',async({page})=>{
 await page.goto('/assistance');
 await expect.poll(()=>page.evaluate(()=>window.uiMotion.some(e=>e.target.matches('.assistance-card')))).toBe(true);
 // Scrub an actual entrance to its midpoint, then change the OS preference.
 await page.evaluate(()=>{
  const {animation}=window.uiMotion.find(e=>e.target.matches('.assistance-card'));
  animation.pause();const t=animation.effect.getTiming();animation.currentTime=t.delay+t.duration/2;
 });
 await page.emulateMedia({reducedMotion:'reduce'});
 const card=page.locator('#wiki-settings');
 await expect(card).toHaveCSS('opacity','1');await expect(card).toHaveCSS('translate','none');
 expect(await card.evaluate(el=>el.getAnimations().length)).toBe(0);
 const toggle=page.getByRole('switch',{name:'Enable Wiki',exact:true});
 await toggle.focus();await page.keyboard.press('Space');await expect(toggle).not.toBeChecked();
 expect(await page.locator('#wiki-settings .setting-switch .switch-track').evaluate(el=>getComputedStyle(el,'::after').transitionDuration)).toBe('0s');
 await page.getByRole('button',{name:'Save assistance settings'}).click();
 await expect(page.getByRole('status').filter({hasText:'Saved to your account'})).toBeVisible();
 expect(await page.locator('.assistance-page').evaluate(el=>el.getAnimations({subtree:true}).length)).toBe(0);
 await page.reload();
 await expect(page.getByRole('switch',{name:'Enable Wiki',exact:true})).not.toBeChecked();
 expect(await page.evaluate(()=>window.uiMotion.length)).toBe(0);
 await page.getByRole('link',{name:'Open Wiki',exact:true}).click();
 await expect(page.locator('.wiki-empty')).toBeVisible();
 expect(await page.locator('.wiki-page').evaluate(el=>el.getAnimations({subtree:true}).length)).toBe(0);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});

test('Wiki views fade out before removal and fade in with the Expo curve',async({page,request})=>{
 await request.post('/api/memos',{data:{title:'Writing a thought',content:'A quiet place for an idea. #writing'}});
 await page.addInitScript(()=>{
  window.fadeFrames=[];
  document.addEventListener('animationstart',event=>{
   if(!event.target.matches('.wiki-layer .fade-swap'))return;
   const animation=event.target.getAnimations().find(a=>a.animationName===event.animationName);
   if(!animation)return;
   const time=animation.currentTime, timing=animation.effect.getTiming();
   animation.pause();
   const values=[.25,.5,.75].map(t=>{
    animation.currentTime=timing.duration*t;
    return Number(getComputedStyle(event.target).opacity);
   });
   window.fadeFrames.push({name:event.animationName,values,inert:event.target.inert,collection:!!event.target.querySelector('.wiki-collection'),article:!!event.target.querySelector('.wiki-article')});
   animation.currentTime=time;animation.play();
  });
 });
 await page.goto('/wiki');
 await page.locator('.wiki-card .wiki-tag').filter({hasText:'#writing'}).click();
 await page.getByTestId('wiki-open-document').click();
 await expect(page.getByRole('article',{name:'writing',exact:true})).toBeFocused();
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveAttribute('data-phase','idle');
 const forward=await page.evaluate(()=>window.fadeFrames.slice(0,2));
 expect(forward.map(f=>f.name)).toEqual(['ui-fade-out','ui-fade-in']);
 expect(forward[0]).toMatchObject({inert:true,collection:false,article:false});
 expect(forward[1]).toMatchObject({inert:false,collection:false,article:true});
 for(const [i,value] of [.015625,.5,.984375].entries()){
  expect(forward[0].values[i]).toBeCloseTo(1-value,4);
  expect(forward[1].values[i]).toBeCloseTo(value,4);
 }
 await page.getByTestId('wiki-back').click();
 await expect(page.getByRole('searchbox',{name:'Search Wiki'})).toBeFocused();
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveAttribute('data-phase','idle');
 const backward=await page.evaluate(()=>window.fadeFrames.slice(2));
 expect(backward.map(f=>f.name)).toEqual(['ui-fade-out','ui-fade-in']);
 expect(backward[0]).toMatchObject({inert:true,article:true});
 await expect(page.locator('.wiki-article')).toHaveCount(0);
});

test('interrupted fades settle immediately and history cannot restore a stale view',async({page,request})=>{
 await request.post('/api/memos',{data:{title:'A small idea',content:'Keep the next idea. #ideas'}});
 await page.addInitScript(()=>{
  document.addEventListener('animationstart',event=>{
   if(event.animationName==='ui-fade-out')event.target.getAnimations().find(a=>a.animationName===event.animationName)?.pause();
  });
 });
 await page.goto('/wiki');
 await page.locator('.wiki-card .wiki-tag').filter({hasText:'#ideas'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/ideas$/);
 await page.getByTestId('wiki-open-document').click();
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveAttribute('data-phase','leaving');
 await page.emulateMedia({reducedMotion:'reduce'});
 await expect(page.getByRole('article',{name:'ideas',exact:true})).toBeFocused();
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveAttribute('data-phase','idle');
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveCSS('opacity','1');
 expect(await page.locator('.wiki-layer .fade-swap').evaluate(el=>el.inert)).toBe(false);
 expect(await page.locator('.wiki-layer .fade-swap').evaluate(el=>el.getAnimations({subtree:true}).length)).toBe(0);
 await page.getByTestId('wiki-back').click();
 await expect(page.getByRole('searchbox',{name:'Search Wiki'})).toBeFocused();
 await page.emulateMedia({reducedMotion:'no-preference'});
 await page.getByTestId('wiki-open-document').click();
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveAttribute('data-phase','leaving');
 await page.goBack();
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 // Beyond the cancelled component's fallback timer, the latest route stays put.
 await page.waitForTimeout(400);
 await expect(page).toHaveURL(/\/wiki\/tag\/ideas$/);
 await expect(page.locator('.wiki-article')).toHaveCount(0);
 await expect(page.locator('.wiki-layer .fade-swap')).toHaveCSS('opacity','1');
});
