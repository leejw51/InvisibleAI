import {test,expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';
isolatedBackend(test,3173);
test.beforeEach(async({request})=>{
 expect((await request.delete('/api/user/data')).ok()).toBeTruthy();
 expect((await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}})).ok()).toBeTruthy();
});
async function keep(request,title,content){const r=await request.post('/api/memos',{data:{title,content}});expect(r.ok()).toBeTruthy();return r.json();}

test('Code shares every kept function with its Input and Output, searches it, copies it, keeps a note, and links back to the Wiki',async({page,request,context},info)=>{
 const memo=await keep(request,'Geometry helpers','Two helpers I keep reusing. #geometry\n\n```rust\n/// Area of a rectangle.\npub fn area(width: f32, height: f32) -> f32 {\n    width * height\n}\n\nfn clamp(value: i32, low: i32, high: i32) -> i32 {\n    value.max(low).min(high)\n}\n```');
 await keep(request,'Greeting script','```python\ndef greet(name: str, times: int = 1) -> str:\n    return name * times\n```');
 await keep(request,'Convert a clip','```sh\nbrew install ffmpeg\nffmpeg -i in.mov out.mp4\n```');
 // Every function from every kept message, the newest message first.
 // Kept notes enter the Wiki once their analysis lands.
 await expect.poll(async()=>(await (await request.get('/api/code')).json()).total).toBe(4);
 const api=await(await request.get('/api/code')).json();
 expect(api.items.map(i=>i.name)).toEqual(['Convert a clip','greet','area','clamp']);
 const area=api.items.find(i=>i.name==='area');
 expect(area).toMatchObject({language:'rust',signature:'pub fn area(width: f32, height: f32) -> f32',returns:'f32',memo_id:memo.id,memo_url:`/wiki/memo/${memo.id}`,description:''});
 expect(area.params).toEqual([{name:'width',type:'f32'},{name:'height',type:'f32'}]);
 expect(area.tags).toContain('geometry');
 expect(area.code).toBe('/// Area of a rectangle.\npub fn area(width: f32, height: f32) -> f32 {\n    width * height\n}');
 expect(api.languages).toEqual([{language:'python',count:1},{language:'rust',count:2},{language:'shell',count:1}]);
 // The menu entry; cards show Input and Output; the newest message leads.
 await page.goto('/');
 const toggle=page.getByRole('button',{name:'Open navigation'});
 if(await toggle.isVisible()) await toggle.click();
 if(!(await page.locator('[data-nav="code"]').isVisible())) await page.locator('.more-tools > summary').click();
 await page.locator('[data-nav="code"]').click();
 await expect(page).toHaveURL(/\/code$/);
 await expect(page.locator('.breadcrumbs strong')).toHaveText('Code');
 const cards=page.getByTestId('code-card');
 await expect(cards).toHaveCount(4);
 await expect(page.getByTestId('code-count')).toHaveText('4 functions');
 const card=cards.filter({hasText:'area'}).first();
 await expect(card.getByTestId('code-input').locator('li')).toHaveText(['widthf32','heightf32']);
 await expect(card.getByTestId('code-output')).toContainText('f32');
 await expect(card.locator('.code-lang')).toHaveText('rust');
 await expect(card.locator('pre')).toContainText('width * height');
 await expect(card.getByTestId('code-from')).toHaveAttribute('href',`/wiki/memo/${memo.id}`);
 await expect(cards.filter({hasText:'Convert a clip'}).getByTestId('code-input')).toContainText('No parameters');
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await page.screenshot({path:`test-results/code-${info.project.name}.png`,fullPage:true});
 // Search by name, by type, by #tag; a language chip narrows the list.
 const search=page.getByRole('searchbox',{name:'Search code'});
 await search.fill('clamp');
 await expect(cards).toHaveCount(1);
 await expect(cards.first()).toContainText('clamp');
 await search.fill('#geometry');
 await expect(cards).toHaveCount(2);
 await search.fill('');
 await page.getByRole('group',{name:'Language'}).getByRole('button',{name:/python/}).click();
 await expect(cards).toHaveCount(1);
 await expect(cards.first()).toContainText('greet');
 await expect(cards.first().getByTestId('code-input').locator('li')).toHaveText(['namestr','timesint']);
 await page.getByRole('group',{name:'Language'}).getByRole('button',{name:'All'}).click();
 await expect(cards).toHaveCount(4);
 // Copy takes the whole function.
 await context.grantPermissions(['clipboard-read','clipboard-write']).catch(()=>{});
 await card.getByTestId('code-copy').click();
 await expect(page.getByRole('status').filter({hasText:'Copied'})).toBeVisible();
 // A note is the user's own, saved on blur, kept across reloads and searchable.
 const note=card.getByRole('textbox',{name:'Note for area'});
 await note.fill('Use for tile sizes');
 await note.blur();
 await expect(card.getByTestId('code-note-saved')).toBeVisible();
 await page.reload();
 await expect(page.getByRole('textbox',{name:'Note for area'})).toHaveValue('Use for tile sizes');
 await page.getByRole('searchbox',{name:'Search code'}).fill('tile sizes');
 await expect(cards).toHaveCount(1);
 expect((await (await request.get('/api/code?q=tile')).json()).items[0].description).toBe('Use for tile sizes');
 // The card leads to the message it came from, read inside the Wiki.
 await cards.first().getByTestId('code-from').click();
 await expect(page).toHaveURL(`/wiki/memo/${memo.id}`);
 await expect(page.getByTestId('wiki-source')).toContainText('Two helpers I keep reusing');
 // Deleting the message takes its functions with it.
 await request.delete(`/api/memos/${memo.id}`);
 expect((await (await request.get('/api/code')).json()).total).toBe(2);
});
