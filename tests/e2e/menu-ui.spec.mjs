import {test,expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';

isolatedBackend(test,3172);

async function openMenu(page) {
  const toggle=page.getByRole('button',{name:'Open navigation',exact:true});
  if(await toggle.isVisible() && await toggle.getAttribute('aria-expanded')!=='true') await toggle.click();
}

test('main menus have distinct destinations, current states and reloadable history',async({page},info)=>{
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto('/');
  for(const [route,label] of [['autonomous','Autonomous AI'],['wiki','Wiki'],['library','Library'],['files','Files'],['images','Images'],['code','Code'],['dashboard','Dashboard'],['devices','Devices'],['assistance','Assistance settings'],['admin','Admin'],['chat','Chat']]) {
    await openMenu(page);
    const item=page.locator(`[data-nav="${route}"]`);
    // The libraries and tools sit behind one fold; the four places to go do not.
    if(!(await item.isVisible())) await page.locator(['ai','assistance','admin'].includes(route) ? '.everyday-settings > summary' : '.more-tools > summary').click();
    await item.click();
    await expect(page).toHaveURL(new RegExp(`/${route}$`));
    await expect(item).toHaveAttribute('aria-current','page');
    await expect(page.locator('.breadcrumbs strong')).toHaveText(label);
    await expect(page.locator('.sidebar-backdrop')).toHaveCount(0);
    expect((await page.reload()).status()).toBe(200);
    await expect(page.locator('.breadcrumbs strong')).toHaveText(label);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  await page.goBack();await expect(page).toHaveURL(/\/admin$/);
  await page.goForward();await expect(page).toHaveURL(/\/chat$/);
  await openMenu(page);
  await page.locator('.more-tools > summary').click();
  await page.locator('[data-nav="overview"]').click();
  await page.reload();await openMenu(page);
  await expect(page.locator('.more-tools')).toHaveAttribute('open');
  await expect(page.locator('[data-nav="overview"]')).toHaveAttribute('aria-current','page');
  await page.locator('[data-nav="autonomous"]').click();
  await page.getByTestId('language-select').selectOption('ko');
  const toggle=page.getByRole('button',{name:'메뉴 열기',exact:true});
  if(await toggle.isVisible()) await toggle.click();
  await expect(page.getByTestId('connection-activity')).toHaveCount(0);
  await page.screenshot({path:`test-results/menu-review-${info.project.name}.png`,fullPage:true});
  expect(errors).toEqual([]);
});

test('mobile sidebar search closes the menu even when already in the memo library',async({page})=>{
  await page.setViewportSize({width:390,height:844});
  await page.goto('/library');
  await page.getByRole('button',{name:'Open navigation',exact:true}).click();
  await page.getByRole('button',{name:'Search your notes',exact:false}).click();
  await expect(page.locator('.sidebar-backdrop')).toHaveCount(0);
  await expect(page.locator('#memo-search')).toBeFocused();
  await page.locator('#memo-search').fill('a useful thought');
  await expect(page.locator('#memo-search')).toHaveValue('a useful thought');
});

test('sidebar search leaves the pinned filter and searches the whole memo library',async({page,request})=>{
  const created=await request.post('/api/memos',{data:{title:'Unpinned menu review',content:'A searchable unpinned thought.'}});
  expect(created.ok()).toBe(true);
  await page.goto('/library');
  await page.getByRole('button',{name:'Pinned',exact:true}).click();
  await expect(page.locator('.memo-card').filter({hasText:'Unpinned menu review'})).toHaveCount(0);
  await openMenu(page);
  await page.getByRole('button',{name:'Search your notes',exact:false}).click();
  await expect(page).toHaveURL(/\/library$/);
  await expect(page.locator('#memo-search')).toBeFocused();
  await page.locator('#memo-search').fill('Unpinned menu review');
  await expect(page.locator('.memo-card').filter({hasText:'Unpinned menu review'})).toBeVisible();
});

test('workspace, Wiki and assistance follow the language without losing drafts or source text',async({page,request},info)=>{
  const {default:fs}=await import('node:fs');
  const catalog=JSON.parse(fs.readFileSync('crates/frontend/src/locales.json','utf8'));
  const codes=['en','ko','yue-HK','zh-CN','ja','es','cs'];
  const text=(key,index)=>index===0?key:catalog[key][index-1];
  await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true,textedit_settle_ms:1200}});
  await request.post('/api/memos',{data:{title:'Original 한국어 日本語',content:'Preserve this source exactly. #menu-review'}});
  await page.goto('/');
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('My unsent 한국어 draft');
  for(let i=0;i<codes.length;i++) {
    await page.getByTestId('language-select').selectOption(codes[i]);
    await expect(page.locator('.invisible-console h2')).toHaveText(text('Autonomous AI run',i));
    await expect(page.locator('.auto-button strong')).toHaveText(text('Start autonomous AI',i));
    await expect(page.locator('.usage-credit')).toHaveCount(0);
    await expect(page.locator('.chat-dock textarea')).toHaveValue('My unsent 한국어 draft');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  await page.getByTestId('language-select').selectOption('en');
  await page.getByTestId('language-select').selectOption('ko');
  await expect(page.getByTestId('connection-activity')).toHaveCount(0);
  await page.screenshot({path:`test-results/menu-workspace-ko-${info.project.name}.png`,fullPage:true});
  await page.getByTestId('language-select').selectOption('en');
  await page.goto('/assistance');
  await page.getByLabel('Typing pause (seconds)').fill('2.5');
  for(let i=0;i<codes.length;i++) {
    await page.getByTestId('language-select').selectOption(codes[i]);
    await expect(page.locator('#textedit-settings h2')).toHaveText(text('Help where you write',i));
    await expect(page.locator('.usage-credit')).toHaveCount(0);
    await expect(page.getByRole('button',{name:text('Save assistance settings',i),exact:true})).toBeEnabled();
    await expect(page.locator('.duration-input input')).toHaveValue('2.5');
    await expect(page.locator('.save-state')).toContainText(text('Unsaved changes',i));
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  expect((await(await request.get('/api/assistance/preferences')).json()).textedit_settle_ms).toBe(1200);
  await page.getByTestId('language-select').selectOption('ko');
  await page.getByRole('button',{name:'지원 설정 저장'}).click();
  await expect(page.locator('.save-state')).toContainText('내 계정에 저장됨');
  await expect(page.getByTestId('connection-activity')).toHaveCount(0);
  await page.screenshot({path:`test-results/menu-assistance-ko-${info.project.name}.png`,fullPage:true});
  await page.goto('/wiki');
  const search=page.locator('.wiki-search input');await search.fill('Preserve this source exactly');
  for(let i=0;i<codes.length;i++) {
    await page.getByTestId('language-select').selectOption(codes[i]);
    await expect(search).toHaveAttribute('aria-label',text('Search Wiki',i));
    await expect(search).toHaveValue('Preserve this source exactly');
    await expect(page.locator('.wiki-library-heading h2')).toHaveText(text('Search results',i));
    await expect(page.locator('.wiki-card')).toHaveCount(1);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  await page.getByTestId('language-select').selectOption('ko');
  await page.locator('.wiki-card .wiki-tag').first().click();
  await page.getByTestId('wiki-open-document').click();
  await expect(page.locator('.wiki-back')).toHaveText('뒤로');
  await expect(page.locator('.wiki-back')).toHaveAttribute('aria-label','Wiki로 돌아가기');
  await expect(page.locator('.wiki-article')).toContainText('Preserve this source exactly.');
  expect((await page.reload()).status()).toBe(200);
  await expect(page.locator('.wiki-article')).toContainText('Original 한국어 日本語');
  await expect(page.getByTestId('connection-activity')).toHaveCount(0);
  await page.screenshot({path:`test-results/menu-wiki-ko-${info.project.name}.png`,fullPage:true});
});
