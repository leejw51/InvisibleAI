import {test,expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';
import fs from 'node:fs';
const baseURL=isolatedBackend(test,3156);
const defaults={wiki_enabled:false,wiki_auto_update:true,wiki_group_by:'topic',wiki_include_answers:true,textedit_enabled:true,textedit_settle_ms:1200,textedit_style:'concise'};
test.beforeEach(async({request})=>{
 expect((await request.delete('/api/user/data')).ok()).toBeTruthy();
 expect((await request.patch('/api/assistance/preferences',{data:defaults})).ok()).toBeTruthy();
});
async function saveNote(request,title,content){const r=await request.post('/api/memos',{data:{title,content}});expect(r.ok()).toBeTruthy();return r.json();}

test('Wiki and TextEdit settings persist across browsers and fit light/dark mobile layouts',async({page,browser,request},info)=>{
 await page.goto('/assistance');
 await expect(page.getByRole('heading',{name:'A little help, your way.'})).toBeVisible();
 await page.getByLabel('Enable Wiki',{exact:false}).check();
 await page.getByLabel('Update Wiki automatically').uncheck();
 await page.getByLabel('Organize Wiki by').selectOption('day');
 await page.getByLabel('Include AI answers').uncheck();
 await page.getByLabel('Enable TextEdit assistance').uncheck();
 await page.getByLabel('Typing pause (seconds)').fill('2.5');
 await page.getByLabel('Answer length').selectOption('detailed');
 await page.getByRole('button',{name:'Save assistance settings'}).click();
 await expect(page.getByRole('status').filter({hasText:'Saved to your account'})).toBeVisible();
 expect(await(await request.get('/api/assistance/preferences')).json()).toMatchObject({wiki_enabled:true,wiki_auto_update:false,wiki_group_by:'day',wiki_include_answers:false,textedit_enabled:false,textedit_settle_ms:2500,textedit_style:'detailed'});
 const context=await browser.newContext();try{const second=await context.newPage();await second.goto(baseURL+'/assistance');await expect(second.getByLabel('Answer length')).toHaveValue('detailed');await expect(second.getByLabel('Enable TextEdit assistance')).not.toBeChecked();}finally{await context.close();}
 await page.reload();await expect(page.getByLabel('Typing pause (seconds)')).toHaveValue('2.5');
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 for(const label of ['Organize Wiki by','Answer length']) expect((await page.getByLabel(label).boundingBox()).height).toBeGreaterThanOrEqual(44);
 await expect(page.getByTestId('connection-activity')).toHaveCount(0);
 await page.evaluate(()=>scrollTo(0,0));await page.screenshot({path:`test-results/assistance-${info.project.name}.png`,fullPage:true});
 await page.getByRole('button',{name:'Switch to dark mode'}).click();
 await page.screenshot({path:`test-results/assistance-dark-${info.project.name}.png`,fullPage:true});
});

test('Wiki tags every message, filters by tag, searches, opens a tag document, and follows deletion',async({page,request},info)=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 const first=await saveNote(request,'Bitcoin basics','Bitcoin uses a distributed ledger. #bitcoin');
 const second=await saveNote(request,'Mining notes','Mining groups transactions into blocks. #bitcoin #mining');
 await saveNote(request,'Writing ideas','A quiet place for new ideas. #writing');
 await page.goto('/wiki');
 const cards=page.locator('.wiki-card');
 await expect(cards).toHaveCount(3);
 // Newest first, each message with its own tags and no room (kept from the library).
 await expect(cards.first()).toContainText('Writing ideas');
 // Kept notes lead with their type and category (a fact or reference is information).
 await expect(cards.first().locator('.wiki-tag')).toHaveText(['#note','#information','#writing']);
 await expect(cards.nth(1).locator('.wiki-tag')).toHaveText(['#note','#information','#bitcoin','#mining']);
 await expect(cards.first().locator('.wiki-room')).toContainText('Library');
 await expect(cards.first().locator('.wiki-local')).toContainText('Local only');
 // The rail counts messages per tag.
 const rail=page.getByRole('complementary',{name:'Tags'});
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'})).toContainText('2');
 await expect(rail.locator('.wiki-rail-types .wiki-rail-tag').filter({hasText:'#note'})).toContainText('3');
 await expect(rail.locator('.wiki-rail-categories .wiki-rail-tag').filter({hasText:'#information'})).toContainText('3');
 await expect(rail.locator('.wiki-rail-all')).toContainText('10');
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await expect(page.getByTestId('connection-activity')).toHaveCount(0);
 await page.screenshot({path:`test-results/wiki-${info.project.name}.png`,fullPage:true});
 await page.getByRole('button',{name:'Switch to dark mode'}).click();
 await page.screenshot({path:`test-results/wiki-dark-${info.project.name}.png`,fullPage:true});
 await page.getByRole('button',{name:'Switch to light mode'}).click();
 // One click on a tag filters the feed; tags that travel with it are offered.
 await rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/bitcoin$/);
 await expect(cards).toHaveCount(2);
 await expect(page.locator('.wiki-active-tag')).toHaveText('#bitcoin');
 await expect(page.locator('.wiki-related-tags .wiki-tag')).toHaveText(['#mining1']);
 await expect(rail.locator('.wiki-rail-tag.is-active')).toContainText('#bitcoin');
 // Words search inside the tag; a #tag in the search box filters too.
 const search=page.getByRole('searchbox',{name:'Search Wiki'});
 await search.fill('distributed ledger');
 await expect(cards).toHaveCount(1);
 await expect(cards.first()).toContainText('Bitcoin basics');
 await search.fill('#mining');
 await expect(cards).toHaveCount(1);
 await expect(cards.first()).toContainText('Mining notes');
 await search.fill('');
 await expect(cards).toHaveCount(2);
 // Every tag has a readable document with all of its messages.
 await page.getByTestId('wiki-open-document').click();
 await expect(page).toHaveURL(/\/wiki\/[a-f0-9]{64}$/);
 await expect(page.locator('.wiki-article')).toContainText('Mining groups transactions');
 await expect(page.locator('.wiki-article')).toContainText('#bitcoin #mining');
 await page.reload();await expect(page.locator('.wiki-article')).toContainText('Mining groups transactions');
 await page.goBack();await expect(cards).toHaveCount(2);await expect(page).toHaveURL(/\/wiki\/tag\/bitcoin$/);
 await page.goForward();await expect(page.locator('.wiki-article')).toContainText('Mining groups transactions');
 await expect(page.getByRole('link',{name:'Open original source'}).first()).toHaveAttribute('href',`/memos/${first.id}`);
 await page.getByTestId('wiki-back').click();
 await expect(page).toHaveURL(/\/wiki\/tag\/bitcoin$/);
 // Deleting a message removes it; an emptied tag leaves the rail.
 await request.delete(`/api/memos/${first.id}`);
 await expect(cards).toHaveCount(1);
 await request.delete(`/api/memos/${second.id}`);
 await expect(page.getByRole('heading',{name:'No matching messages.'})).toBeVisible();
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'})).toHaveCount(0);
 await page.getByTestId('wiki-clear-tag').click();
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(cards).toHaveCount(1);
 await expect(cards.first().locator('.wiki-tag')).toHaveText(['#note','#information','#writing']);
 // A change made elsewhere reaches the open Wiki through the notification
 // stream well before the slow fallback poll; Refresh reloads by hand.
 await saveNote(request,'Late note','Added while the Wiki was open. #late');
 await expect(cards).toHaveCount(2,{timeout:10000});
 await expect(cards.first()).toContainText('Late note');
 await page.getByTestId('wiki-refresh').click();
 await expect(cards).toHaveCount(2);
 await expect(page.getByTestId('wiki-refresh')).not.toHaveClass(/is-busy/);
});

test('chat messages are tagged automatically, link to the Wiki, and tags can be edited in place',async({page,request},info)=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 await page.goto('/chat');
 await expect(page.getByRole('heading',{name:'Keep the thought going.'})).toBeVisible();
 await page.getByRole('button',{name:'New chat',exact:true}).click();
 await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toBeEnabled();
 const prompt='Bitcoin mining difficulty adjusts every two weeks of bitcoin blocks';
 await page.getByRole('textbox',{name:'Chat message',exact:true}).fill(prompt);
 await page.getByRole('button',{name:'Send message'}).click();
 const message=page.locator('.from-user').filter({hasText:prompt});
 await expect(message).toBeVisible();
 await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
 // The on-device analysis names the tag; the chip opens that tag in the Wiki on a second tap.
 // The type leads: a question with its answer is q&a; the topic follows.
 const chips=message.getByTestId('chat-tags').locator('.chat-tag');
 await expect(chips).toHaveText(['#q&a','#bitcoin']);
 await expect(chips.first()).toHaveClass(/is-type/);
 await expect(chips.first()).toHaveAttribute('href','/wiki/tag/q%26a');
 await expect(chips.nth(1)).toHaveAttribute('href','/wiki/tag/bitcoin');
 // The generated response carries the same automatically generated tags.
 await expect(page.locator('.from-ai').getByTestId('chat-tags').locator('.chat-tag')).toHaveText(['#q&a','#bitcoin']);
 // Edit in place: remove the inferred tag, add two by typing, and save.
 await message.getByRole('button',{name:'Edit tags'}).click();
 const editor=message.getByTestId('chat-tags-editor');
 await editor.getByRole('button',{name:'Remove tag bitcoin'}).click();
 await editor.getByRole('textbox',{name:'Add tag'}).fill('Energy Use');
 await editor.getByRole('textbox',{name:'Add tag'}).press('Enter');
 await editor.getByRole('textbox',{name:'Add tag'}).fill('#mining,');
 // The type is shown in the editor but is not editable there.
 await expect(editor.locator('.chat-tag')).toHaveText(['#q&a','#energy-use','#mining']);
 await expect(editor.getByRole('button',{name:'Remove tag q&a'})).toHaveCount(0);
 await editor.getByRole('button',{name:'Save tags'}).click();
 await expect(chips).toHaveText(['#q&a','#energy-use','#mining']);
 await expect(page.locator('.from-ai').getByTestId('chat-tags').locator('.chat-tag')).toHaveText(['#q&a','#energy-use','#mining']);
 // The chat message keeps its tags after a reload: they travel with room sync.
 await page.reload();
 await expect(page.locator('.from-user').filter({hasText:prompt}).getByTestId('chat-tags').locator('.chat-tag')).toHaveText(['#q&a','#energy-use','#mining']);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await page.screenshot({path:`test-results/chat-tags-${info.project.name}.png`,fullPage:true});
 // The Wiki follows the edit: the removed tag is gone, the new ones are there,
 // and the message links back to its room.
 // One tap only arms the chip (a stray touch stays in the chat); the second opens the Wiki.
 const chip=page.locator('.from-user').filter({hasText:prompt}).locator('.chat-tag').filter({hasText:'#mining'});
 await chip.click();
 await expect(chip).toHaveClass(/is-armed/);
 await expect(chip).toHaveAttribute('title','Tap again to open in Wiki');
 await expect(page).not.toHaveURL(/\/wiki\//);
 await chip.click();
 await expect(page).toHaveURL(/\/wiki\/tag\/mining$/);
 const cards=page.locator('.wiki-card');
 await expect(cards).toHaveCount(1);
 await expect(cards.first().locator('.wiki-tag')).toHaveText(['#q&a','#energy-use','#mining']);
 // The question and its answer are one node: the card shows the answer too.
 await expect(cards.first().getByTestId('wiki-card-answer')).toContainText('demo mode');
 await expect(cards.first().locator('.wiki-room')).toHaveAttribute('href',/\/chat\?room=/);
 const rail=page.getByRole('complementary',{name:'Tags'});
 await expect(rail.locator('.wiki-rail-types .wiki-rail-tag').filter({hasText:'#q&a'})).toContainText('1');
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#energy-use'})).toContainText('1');
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'})).toHaveCount(0);
 // Tag edits are bounded on the server; a later analysis never overrides them.
 const memo=(await (await request.get('/api/wiki?tag=mining')).json()).items[0].id;
 expect((await request.patch(`/api/memos/${memo}`,{data:{tags:Array.from({length:13},(_,i)=>`t${i}`)}})).status()).toBe(400);
 // The memo keeps the order given; the Wiki lists a message's tags alphabetically.
 const saved=await (await request.patch(`/api/memos/${memo}`,{data:{tags:['#Mining','grid']}})).json();
 expect(saved.tags).toEqual(['mining','grid']);
 await request.post('/api/wiki/rebuild');
 expect((await (await request.get(`/api/wiki?tag=grid`)).json()).items[0].tags).toEqual(['q&a','grid','mining']);
});

test('trivial messages stay out of the Wiki until kept by hand',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 await page.goto('/chat');
 await page.getByRole('button',{name:'New chat',exact:true}).click();
 await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toBeEnabled();
 // Two words: a quick lookup with nothing to remember.
 await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('explain briefly');
 await page.getByRole('button',{name:'Send message'}).click();
 const message=page.locator('.from-user').filter({hasText:'explain briefly'});
 await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
 await expect(message.getByTestId('chat-wiki-skipped')).toContainText('Not in Wiki');
 // Left out of the Wiki, it still shows the tags the analysis inferred.
 await expect(message.getByTestId('chat-tags').locator('.chat-tag').first()).toBeVisible();
 expect((await (await request.get('/api/wiki')).json()).total).toBe(0);
 // Keeping it by hand files it with tags; removing it takes it back out.
 await message.getByTestId('chat-wiki-keep').click();
 await expect(message.getByTestId('chat-tags').locator('.chat-tag').first()).toHaveText('#q&a');
 await expect.poll(async()=>(await (await request.get('/api/wiki')).json()).total).toBe(1);
 await message.getByRole('button',{name:'Edit tags'}).click();
 await message.getByTestId('chat-wiki-remove').click();
 await expect(message.getByTestId('chat-wiki-skipped')).toContainText('Not in Wiki');
 await expect.poll(async()=>(await (await request.get('/api/wiki')).json()).total).toBe(0);
});

test('a plain record is kept as a note with a category, and can be asked instead',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 await page.goto('/chat');
 await page.getByRole('button',{name:'New chat',exact:true}).click();
 await expect(page.getByRole('textbox',{name:'Chat message',exact:true})).toBeEnabled();
 const line='Had dinner with mom tonight, she seemed tired. I should visit more often.';
 await page.getByRole('textbox',{name:'Chat message',exact:true}).fill(line);
 await page.getByRole('button',{name:'Send message'}).click();
 // A diary line is a record: kept as the post, tidied by the AI into a clean
 // note rather than answered, and filed as personal.
 const note=page.locator('.from-user').filter({hasText:line});
 await expect(note).toContainText('Kept as a note');
 const tidied=page.locator('.from-ai').first();
 await expect(tidied).toContainText('Tidied note');
 await expect(tidied.locator('.answer-content')).toContainText('Had dinner with mom tonight');
 await expect(tidied.locator('.answer-content li')).toHaveCount(2);
 await expect(note.getByTestId('chat-tags').locator('.chat-tag')).toContainText(['#note','#personal']);
 await expect(tidied.getByTestId('chat-tags').locator('.chat-tag')).toContainText(['#note','#personal']);
 await expect(page.locator('.from-ai')).toHaveCount(1);
 // Ask AI instead sends the same line as a question; a real answer arrives.
 await note.getByTestId('chat-ask-instead').click();
 await expect(page.locator('.from-ai')).toHaveCount(2);
 await expect(page.locator('.from-ai').nth(1).locator('.answer-content')).toContainText('demo mode');
 await expect(page.locator('.from-ai').nth(1).getByTestId('chat-tags').locator('.chat-tag').first()).toHaveText('#q&a');
 // The Wiki lists both categories first in the rail.
 await page.goto('/wiki');
 const rail=page.getByRole('complementary',{name:'Tags'});
 await expect(rail.locator('.wiki-rail-categories .wiki-rail-tag').filter({hasText:'#personal'})).toContainText('1');
 await expect(rail.locator('.wiki-rail-types .wiki-rail-tag').filter({hasText:'#q&a'})).toContainText('1');
 await expect(rail.locator('.wiki-rail-types .wiki-rail-tag').filter({hasText:'#note'})).toContainText('1');
 await rail.locator('.wiki-rail-tag').filter({hasText:'#personal'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/personal$/);
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await expect(page.locator('.wiki-card').first()).toContainText('Had dinner with mom');
 // The card's body is the tidied note; the words as written sit under Original.
 await expect(page.locator('.wiki-card').first().getByTestId('wiki-card-answer')).toContainText('Had dinner with mom tonight');
 await expect(page.locator('.wiki-card').first().locator('.wiki-card-original')).toContainText('Original');
});

test('manual Wiki builds include saved sources only on request and reorganize documents by day',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true,wiki_auto_update:false}});
 await saveNote(request,'First thought','A first thought #ideas');
 await page.goto('/wiki');await expect(page.locator('.wiki-empty')).toBeVisible();
 await page.getByRole('button',{name:'Build from saved sources'}).click();
 await expect(page.locator('.wiki-card').locator('.wiki-tag')).toHaveText(['#note','#information','#ideas']);
 await saveNote(request,'Second thought','A second thought #new');
 await page.reload();await expect(page.locator('.wiki-card')).toHaveCount(1);
 await page.getByRole('button',{name:'Build from saved sources'}).click();await expect(page.locator('.wiki-card')).toHaveCount(2);
 await page.getByRole('link',{name:'Wiki & TextEdit settings'}).click();
 await page.getByLabel('Organize Wiki by').selectOption('day');await page.getByRole('button',{name:'Save assistance settings'}).click();
 await expect(page.getByRole('status').filter({hasText:'Saved to your account'})).toBeVisible();
 // Day grouping changes the derived documents only: the feed keeps its tags,
 // and a tag has no document of its own to open.
 await page.getByRole('link',{name:'Open Wiki',exact:true}).click();await expect(page.locator('.wiki-card')).toHaveCount(2);
 await page.locator('.wiki-card .wiki-tag').filter({hasText:'#ideas'}).click();
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await expect(page.getByTestId('wiki-open-document')).toHaveCount(0);
});

test('TextEdit disabled is enforced by the backend and leaves Auto off',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{textedit_enabled:false}});
 const room=await(await request.post('/api/rooms',{data:{title:'Disabled notes'}})).json();
 const start=await request.post(`/api/rooms/${room.roomid}/notes`,{data:{client_id:'any-device'}});
 expect(start.status()).toBe(400);expect(await start.text()).toContain('Enable TextEdit assistance');
 await page.goto('/');await expect(page.getByTestId('textedit-auto-toggle')).toBeDisabled();
 await expect(page.getByText('TextEdit assistance is disabled. Enable it in assistance settings to use Auto.')).toBeVisible();
 await page.getByRole('link',{name:'Customize Wiki & TextEdit'}).click();
 await page.getByLabel('Enable TextEdit assistance').check();await page.getByRole('button',{name:'Save assistance settings'}).click();
 await expect(page.getByRole('status').filter({hasText:'Saved to your account'})).toBeVisible();
 expect((await(await request.get(`/api/rooms/${room.roomid}/notes`)).json()).active).toBe(false);
});

test('settings make unsaved changes, discard, and invalid typing pauses clear',async({page,request},info)=>{
 await page.goto('/assistance');
 const save=page.getByRole('button',{name:'Save assistance settings'});
 await expect(save).toBeDisabled();
 await page.getByLabel('Enable Wiki',{exact:true}).check();
 await expect(page.getByRole('status').filter({hasText:'Unsaved changes'})).toBeVisible();
 await page.getByRole('button',{name:'Discard',exact:true}).click();
 await expect(page.getByLabel('Enable Wiki',{exact:true})).not.toBeChecked();
 await expect(save).toBeDisabled();
 expect((await(await request.get('/api/assistance/preferences')).json()).wiki_enabled).toBe(false);
 const pause=page.getByLabel('Typing pause (seconds)');
 await pause.fill('0.1');await save.click();
 await expect(page.getByRole('alert')).toHaveText('Choose a typing pause between 0.5 and 10 seconds.');
 expect((await(await request.get('/api/assistance/preferences')).json()).textedit_settle_ms).toBe(1200);
 await pause.fill('1.2');await expect(save).toBeDisabled();
 await expect(page.getByRole('alert')).toHaveCount(0);
 await page.getByRole('switch',{name:'Enable Wiki',exact:true}).focus();
 await page.keyboard.press('Space');await expect(page.getByLabel('Enable Wiki',{exact:true})).toBeChecked();
 await page.getByRole('button',{name:'Discard',exact:true}).click();
 await page.setViewportSize({width:320,height:700});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await page.getByRole('navigation',{name:'Assistance sections'}).getByRole('link',{name:/TextEdit/}).click();
 await expect(page.getByRole('region',{name:'TextEdit settings'})).toBeFocused();
 expect((await page.getByRole('heading',{name:'Help where you write'}).boundingBox()).y).toBeLessThan(250);
 await page.getByRole('navigation',{name:'Assistance sections'}).getByRole('link',{name:/Wiki/}).click();
 await expect(page.getByRole('region',{name:'Wiki settings'})).toBeFocused();
 await save.scrollIntoViewIfNeeded();const box=await save.boundingBox();expect(box.y+box.height).toBeLessThanOrEqual(700);
 await expect(page.getByTestId('connection-activity')).toHaveCount(0);
 await page.screenshot({path:`test-results/assistance-narrow-${info.project.name}.png`,fullPage:true});
 await page.getByLabel('Enable Wiki',{exact:true}).check();
 await page.evaluate(()=>scrollTo(0,0));
 const sticky=await save.boundingBox();expect(sticky.y).toBeGreaterThanOrEqual(0);expect(sticky.y+sticky.height).toBeLessThanOrEqual(700);
 await save.click();await expect(page.getByRole('status').filter({hasText:'Saved to your account'})).toBeVisible();
 await page.goto('/wiki');await expect(page.locator('.wiki-empty')).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
});

test('Wiki search resets easily and a tag document opens as a focused reading view',async({page,request},info)=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 await saveNote(request,'A thought worth keeping','Writing something down gives you a place to begin. #writing');
 await saveNote(request,'A small daily habit','Leave a little room for a new idea. #ideas');
 await page.goto('/wiki');
 const search=page.getByRole('searchbox',{name:'Search Wiki'});
 await search.fill('no-matching-source');
 await expect(page.getByRole('heading',{name:'No matching messages.'})).toBeVisible();
 await page.getByRole('button',{name:'Show all messages'}).click();
 await expect(search).toHaveValue('');await expect(search).toBeFocused();
 await expect(page.locator('.wiki-card')).toHaveCount(2);
 await search.fill('writing');
 const clear=page.getByRole('button',{name:'Clear Wiki search'});
 expect((await clear.boundingBox()).height).toBeGreaterThanOrEqual(44);
 await clear.focus();await page.keyboard.press('Enter');
 await expect(search).toBeFocused();await expect(search).toHaveValue('');
 await search.fill('writing');await expect(page.locator('.wiki-card')).toHaveCount(1);
 await page.locator('.wiki-card .wiki-tag').filter({hasText:'#writing'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/writing$/);
 await page.getByTestId('wiki-open-document').click();
 const article=page.getByRole('article',{name:'writing',exact:true});
 await expect(article).toBeFocused();await expect(article).toContainText('1 linked source');
 expect((await article.boundingBox()).y).toBeLessThan(400);
 expect((await page.getByTestId('wiki-back').boundingBox()).y).toBeGreaterThanOrEqual(0);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await expect(page.getByTestId('connection-activity')).toHaveCount(0);
 await page.evaluate(()=>scrollTo(0,0));await page.screenshot({path:`test-results/wiki-reader-${info.project.name}.png`,fullPage:true});
 // The reader is the whole screen: Back (or Esc) returns to the search as it was.
 await page.getByTestId('wiki-back').click();
 await expect(search).toHaveValue('writing');await expect(search).toBeFocused();
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await page.getByRole('button',{name:'Switch to dark mode'}).click();
 await page.getByTestId('wiki-open-document').click();
 await expect(article).toContainText('1 linked source');
 await page.screenshot({path:`test-results/wiki-reader-dark-${info.project.name}.png`,fullPage:true});
 await page.keyboard.press('Escape');
 await expect(article).toHaveCount(0);
 await expect(search).toHaveValue('writing');
 // A search that found something is offered again after a letter or two, and
 // so are tags; one tap searches it.
 await search.fill('');
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await search.pressSequentially('writ',{delay:350});
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await search.fill('');
 await search.fill('wr');
 const suggest=page.getByTestId('wiki-suggest');
 await expect(suggest.getByRole('option')).toHaveText(['writing','#writing']);
 await suggest.getByRole('option').first().click();
 await expect(search).toHaveValue('writing');
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await expect(suggest).toHaveCount(0);
 const language=page.getByRole('combobox',{name:'Language',exact:true});
 expect((await language.boundingBox()).height).toBeGreaterThanOrEqual(44);
});

test('Wiki infers tags on-device, ranks search by messages, and the map filters by tag',async({page,request},info)=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 // No hashtags: the on-device analysis (a heuristic in demo mode) names the tag.
 await saveNote(request,'Sourdough','Sourdough starter needs flour and water daily feeding');
 await saveNote(request,'Lightning','Lightning channels settle payments off-chain #payments');
 await page.goto('/wiki');
 const rail=page.getByRole('complementary',{name:'Tags'});
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#sourdough'})).toBeVisible();
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#payments'})).toBeVisible();
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#inbox'})).toHaveCount(0);
 // Ranked search: a word that only appears in a message's text finds it.
 const search=page.getByRole('searchbox',{name:'Search Wiki'});
 await search.fill('channels');
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await expect(page.locator('.wiki-card').first()).toContainText('Lightning');
 await search.fill('');
 await expect(page.locator('.wiki-card')).toHaveCount(2);
 // The map: tag labels when WebGL draws, a plain list otherwise; both filter.
 await page.getByTestId('wiki-graph-toggle').click();
 await expect(page).toHaveURL(/\/wiki\/map$/);
 const graph=page.getByTestId('wiki-graph');
 await expect(graph).toHaveAttribute('data-renderer',/three|unavailable/);
 const api=await(await request.get('/api/wiki/graph')).json();
 expect(api.nodes.filter(n=>n.kind==='tag').map(n=>n.title).sort()).toEqual(['payments','sourdough']);
 expect(api.nodes.filter(n=>n.kind!=='tag')).toHaveLength(2);
 expect(api.edges.filter(e=>e.kind==='tag')).toHaveLength(2);
 const entry=page.locator('wiki-graph .wiki-graph-label.is-tag, .wiki-graph-fallback li button').filter({hasText:'#sourdough'}).first();
 await expect(entry).toBeVisible();
 // Map labels float over the canvas and may sit outside a phone viewport.
 await entry.evaluate(el=>el.click());
 await expect(page.locator('.wiki-active-tag')).toHaveText('#sourdough');
 await expect(page).toHaveURL(/\/wiki\/map$/);
 await expect(graph).toHaveAttribute('data-selected','tag:sourdough');
 await expect(page.locator('.wiki-graph-fallback li.is-active')).toHaveText(/sourdough/);
 if((await graph.getAttribute('data-renderer'))==='three')await expect(page.locator('.wiki-graph-label.is-selected')).toHaveText('#sourdough');
 await page.screenshot({path:`test-results/wiki-map-view-${info.project.name}.png`,fullPage:true});
 // One tap on a message node reads it over the map; Back returns to the map as it was.
 const note=api.nodes.find(n=>n.title==='Sourdough');
 await graph.evaluate((el,id)=>el.dispatchEvent(new CustomEvent('wiki-select',{detail:{id,kind:'note',url:''},bubbles:true})),note.id);
 await expect(page).toHaveURL(`/wiki/memo/${note.id}`);
 await expect(page.getByTestId('wiki-source')).toContainText('Sourdough starter needs flour');
 await page.reload();
 await expect(page.getByTestId('wiki-source')).toContainText('Sourdough starter needs flour');
 await page.keyboard.press('Escape');
 await expect(page.getByTestId('wiki-source')).toHaveCount(0);
 await expect(page).toHaveURL(/\/wiki\/map$/);
 await expect(page.locator('.wiki-active-tag')).toHaveText('#sourdough');
 // A real tap that lands a little beside the small green dot still reads the
 // note: the nearest node within a fingertip counts. Only when WebGL draws;
 // the map must be in view so the pointer lands on the canvas.
 if((await graph.getAttribute('data-renderer'))==='three'){
  await graph.scrollIntoViewIfNeeded();
  // The layout settles first, so the dot is where the test looked.
  await expect.poll(()=>graph.evaluate(el=>el.steps)).toBeGreaterThanOrEqual(240);
  const spot=await graph.evaluate((el,id)=>{const r=el.renderer.domElement.getBoundingClientRect();const n=el.nodes.find(n=>n.id===id);const v=n.p.clone().applyMatrix4(el.group.matrixWorld).project(el.camera);return {x:r.left+((v.x+1)/2)*r.width,y:r.top+((1-v.y)/2)*r.height};},note.id);
  await page.mouse.click(spot.x+9,spot.y-4);
  await expect(page).toHaveURL(`/wiki/memo/${note.id}`);
  await expect(page.getByTestId('wiki-source')).toContainText('Sourdough starter needs flour');
  await page.keyboard.press('Escape');
  await expect(page).toHaveURL(/\/wiki\/map$/);
 }
 // Zoom buttons move the camera; a tag's double tap leaves the map for its
 // messages (the event the canvas dispatches on a double tap).
 await page.getByTestId('wiki-zoom-in').click();
 await page.getByTestId('wiki-map-full').click();
 await expect(page.getByTestId('wiki-graph-panel')).toHaveClass(/is-full/);
 await page.keyboard.press('Escape');
 await expect(page.getByTestId('wiki-graph-panel')).not.toHaveClass(/is-full/);
 await graph.evaluate(el=>el.dispatchEvent(new CustomEvent('wiki-open',{detail:{id:'tag:sourdough',kind:'tag',url:''},bubbles:true})));
 await expect(page).toHaveURL(/\/wiki\/tag\/sourdough$/);
 await expect(page.getByTestId('wiki-feed')).toBeVisible();
 await expect(page.locator('.wiki-active-tag')).toHaveText('#sourdough');
 // Back to the feed keeps the tag; the bare Wiki menu entry reopens it too.
 await page.getByTestId('wiki-feed-toggle').click();
 await expect(page).toHaveURL(/\/wiki\/tag\/sourdough$/);
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 await expect(page.locator('.wiki-card').first()).toContainText('Sourdough starter needs flour');
 await page.goto('/wiki');
 await expect(page).toHaveURL(/\/wiki\/tag\/sourdough$/);
 await page.getByTestId('wiki-clear-tag').click();
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(page.locator('.wiki-card')).toHaveCount(2);
 await page.reload();
 await expect(page).toHaveURL(/\/wiki$/);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await page.screenshot({path:`test-results/wiki-map-${info.project.name}.png`,fullPage:true});
});

test('Wiki shows a picture, cloud usage and the chat room per message, and relates messages by tag',async({page,request},info)=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 await saveNote(request,'Harbour notes','Notes about the harbour. #media');
 // A chat message with a picture and a short video becomes one memo and one Wiki node.
 await page.goto('/chat');
 await expect(page.getByRole('heading',{name:'Keep the thought going.'})).toBeVisible();
 const rooms=page.getByRole('combobox',{name:'Chat room',exact:true});
 const before=await rooms.inputValue();
 await page.getByRole('button',{name:'New chat',exact:true}).click();
 await expect(rooms).not.toHaveValue(before);
 await page.getByLabel('Attach to chat',{exact:true}).setInputFiles([
  {name:'harbour.png',mimeType:'image/png',buffer:fs.readFileSync('frontend/assets/harbour.png')},
  {name:'clip.mp4',mimeType:'video/mp4',buffer:fs.readFileSync('tests/e2e/fixtures/clip.mp4')},
 ]);
 await expect(page.locator('.chat-attachment')).toHaveCount(2);
 await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('What is in this harbour picture and clip? #media');
 await page.getByRole('button',{name:'Send message',exact:true}).click();
 await expect(page.locator('.from-ai .chat-message-meta')).toContainText('Cloud LLM');
 await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
 const files=(await (await request.get('/api/files')).json());
 const video=files.find(f=>f.name==='clip.mp4');
 expect(video.mime).toBe('video/mp4');
 // The video's frame is rendered by Quick Look on the backend.
 const frame=await request.get(`/api/files/${video.id}/thumbnail`);
 expect(frame.status()).toBe(200);
 expect(frame.headers()['content-type']).toMatch(/^image\//);
 // The feed card is a photo node: thumbnail, cloud cost, the room it was said in.
 await page.goto('/wiki');
 const card=page.locator('.wiki-card.is-photo');
 await expect(card).toHaveCount(1);
 await expect(card.locator('.wiki-card-thumb img')).toHaveAttribute('src',/^\/api\/files\/[^/]+\/thumbnail$/);
 await expect(card.locator('.wiki-cloud')).toContainText('1 cloud');
 await expect(card.locator('.wiki-answered')).toBeVisible();
 await expect(card.getByTestId('wiki-room')).toHaveAttribute('href',/^\/chat\?room=/);
 // One tap on the thumbnail shows the picture over the feed; Next plays the
 // clip from the backend's seekable stream; Back leaves the feed exactly as it was.
 await card.getByTestId('wiki-card-thumb').click();
 const lightbox=page.getByTestId('wiki-lightbox');
 await expect(lightbox.getByTestId('wiki-lightbox-image')).toHaveAttribute('src',/^\/api\/files\/[^/]+\/image$/);
 await expect(lightbox).toContainText('1 / 2');
 await lightbox.getByRole('button',{name:'Next'}).click();
 await expect(lightbox.getByTestId('wiki-lightbox-video')).toHaveAttribute('src',`/api/files/${video.id}/stream`);
 await expect(lightbox.getByTestId('wiki-lightbox-video')).toHaveJSProperty('autoplay',true);
 await expect(page).toHaveURL(/\/wiki$/);
 await page.keyboard.press('Escape');
 await expect(lightbox).toHaveCount(0);
 await expect(card).toHaveCount(1);
 // One tap on the title reads the message in place, with its clip ready to play.
 await card.getByTestId('wiki-card-title').click();
 await expect(page).toHaveURL(/\/wiki\/memo\/[a-f\d-]{36}$/);
 const source=page.getByTestId('wiki-source');
 await expect(source.locator('.wiki-source-title')).toContainText('harbour picture and clip');
 await expect(source.locator(`video[src="/api/files/${video.id}/stream"]`)).toHaveCount(1);
 await expect(source.locator('img[src^="/api/files/"][src$="/image"]')).toHaveCount(1);
 await expect(source).toContainText('1 cloud');
 await expect(source.getByTestId('wiki-open-library')).toHaveAttribute('href',/^\/memos\/[a-f\d-]{36}$/);
 // A picture's tags are edited in the reader like any message's: the reader
 // shows the saved list at once and the feed follows the Wiki's own event.
 await source.getByTestId('tag-editor-edit').click();
 await source.getByLabel('Add tag',{exact:true}).fill('pier');
 await source.getByTestId('tag-editor-save').click();
 await expect(source.getByTestId('tag-editor-form')).toHaveCount(0);
 await expect(source.locator('.wiki-tag').filter({hasText:'#pier'})).toHaveCount(1);
 await expect(source.locator('.wiki-tag').filter({hasText:'#media'})).toHaveCount(1);
 expect((await page.getByTestId('wiki-back').boundingBox()).height).toBeGreaterThanOrEqual(48);
 await page.getByTestId('wiki-back').click();
 await expect(source).toHaveCount(0);
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(card).toHaveCount(1);
 await expect(card.locator('.wiki-tag').filter({hasText:'#pier'})).toHaveCount(1);
 // The map carries the same message as a photo node with its thumbnail and
 // what it can play; a tap on that node shows the picture at once.
 const graph=await(await request.get('/api/wiki/graph')).json();
 const photo=graph.nodes.find(n=>n.kind==='photo'&&/^\/api\/files\/[^/]+\/thumbnail$/.test(n.thumbnail));
 expect(photo.media.map(m=>m.kind)).toEqual(['image','video']);
 await page.getByTestId('wiki-graph-toggle').click();
 await expect(page.getByTestId('wiki-graph')).toHaveAttribute('data-graph',/"kind":"photo"/);
 await page.getByTestId('wiki-graph').evaluate((el,id)=>el.dispatchEvent(new CustomEvent('wiki-select',{detail:{id,kind:'photo',url:''},bubbles:true})),photo.id);
 await expect(lightbox.getByTestId('wiki-lightbox-image')).toBeVisible();
 // A tap inside the lightbox never reaches the map's node under it: only a
 // press that began on the canvas can pick.
 const leaked=await page.getByTestId('wiki-graph').evaluate(el=>{let n=0;el.addEventListener('wiki-select',()=>n++);const r=el.getBoundingClientRect();for(const target of [document.body,el])target.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,clientX:r.left+r.width/2,clientY:r.top+r.height/2,pointerId:7}));return n;});
 expect(leaked).toBe(0);
 // The lightbox leads to the message; Back from there keeps the map.
 await lightbox.getByTestId('wiki-lightbox-read').click();
 await expect(lightbox).toHaveCount(0);
 await expect(source.locator('.wiki-source-title')).toContainText('harbour picture and clip');
 await page.getByTestId('wiki-back').click();
 await expect(source).toHaveCount(0);
 await expect(page).toHaveURL(/\/wiki\/map$/);
 // A tag chip inside the reader filters the map and the address follows.
 await page.getByTestId('wiki-graph').evaluate((el,id)=>el.dispatchEvent(new CustomEvent('wiki-open',{detail:{id,kind:'photo',url:''},bubbles:true})),photo.id);
 await source.locator('.wiki-tag').filter({hasText:'#media'}).click();
 await expect(source).toHaveCount(0);
 await expect(page).toHaveURL(/\/wiki\/map$/);
 await expect(page.locator('.wiki-active-tag')).toHaveText('#media');
 await page.getByTestId('wiki-graph').evaluate((el,id)=>el.dispatchEvent(new CustomEvent('wiki-select',{detail:{id,kind:'photo',url:''},bubbles:true})),photo.id);
 await lightbox.getByTestId('wiki-media-close').click();
 await expect(lightbox).toHaveCount(0);
 await expect(page).toHaveURL(/\/wiki\/map$/);
 // A double tap on the node reads the message over the map; Back keeps the map.
 await page.getByTestId('wiki-graph').evaluate((el,id)=>el.dispatchEvent(new CustomEvent('wiki-open',{detail:{id,kind:'photo',url:''},bubbles:true})),photo.id);
 await expect(source.locator('.wiki-source-title')).toContainText('harbour picture and clip');
 await expect(page.getByTestId('wiki-graph')).toHaveAttribute('data-selected',photo.id);
 await page.getByTestId('wiki-back').click();
 await expect(page).toHaveURL(/\/wiki\/map$/);
 await expect(page.getByTestId('wiki-graph-panel')).toBeVisible();
 await page.getByTestId('wiki-feed-toggle').click();
 // Related: the kept note shares #media with the chat message.
 await card.getByRole('button',{name:'Related'}).click();
 const related=page.getByTestId('wiki-related');
 await expect(related).toContainText('Related to');
 await expect(related.locator('.wiki-card')).toHaveCount(1);
 await expect(related.locator('.wiki-card')).toContainText('Harbour notes');
 await page.getByRole('button',{name:'Close related messages'}).click();
 await expect(related).toHaveCount(0);
 // The tag document embeds both media files and records the cloud call.
 await card.locator('.wiki-tag').filter({hasText:'#media'}).click();
 await page.getByTestId('wiki-open-document').click();
 await expect(page).toHaveURL(/\/wiki\/[a-f0-9]{64}$/);
 const article=page.locator('.wiki-article');
 await expect(article.locator('img[src^="/api/files/"][src$="/image"]')).toHaveCount(1);
 await expect(article.locator(`img[src="/api/files/${video.id}/thumbnail"]`)).toHaveCount(1);
 await expect(article.locator('a[href^="/chat?room="]')).toHaveCount(1);
 await expect(page.getByTestId('wiki-usage')).toContainText('1 AI requests');
 const nodeUrl=page.url();
 // The clip's frame in the document plays the clip in place.
 await article.locator(`img[src="/api/files/${video.id}/thumbnail"]`).click();
 await expect(lightbox.getByTestId('wiki-lightbox-video')).toHaveAttribute('src',`/api/files/${video.id}/stream`);
 await page.keyboard.press('Escape');
 await expect(lightbox).toHaveCount(0);
 await expect(page).toHaveURL(nodeUrl);
 // Reading the source opens it over the document; Back lands on the same document.
 await page.getByRole('link',{name:'Open original source'}).last().click();
 await expect(page).toHaveURL(/\/wiki\/memo\/[a-f\d-]{36}$/);
 await expect(source).toContainText('1 cloud');
 await page.getByTestId('wiki-back').click();
 await expect(page).toHaveURL(nodeUrl);
 await expect(article).toBeVisible();
 await expect(page.getByTestId('wiki-usage')).toContainText('1 AI requests');
 // The room link opens the conversation the message came from.
 await page.getByTestId('wiki-back').click();
 await page.locator('.wiki-card.is-photo').getByTestId('wiki-room').click();
 await expect(page).toHaveURL(/\/chat\?room=/);
 await expect(page.locator('.chat-message').filter({hasText:'harbour picture and clip'}).first()).toBeVisible();
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await page.screenshot({path:`test-results/wiki-media-${info.project.name}.png`,fullPage:true});
});

test('closing an open tag brings back exactly the screen it was opened from',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 for(let i=0;i<10;i++) await saveNote(request,`Harbour walk ${i}`,`Walk ${i} along the harbour front. #walks`);
 await saveNote(request,'Mining notes','Mining groups transactions into blocks. #bitcoin #mining');
 await saveNote(request,'Bitcoin basics','Bitcoin uses a distributed ledger. #bitcoin');
 await page.goto('/wiki');
 const cards=page.locator('.wiki-card');
 await expect(cards).toHaveCount(12);
 // A search, a scroll down, then a tag from a card near the bottom.
 const search=page.getByRole('searchbox',{name:'Search Wiki'});
 await search.fill('harbour');
 await expect(cards).toHaveCount(10);
 await page.evaluate(()=>scrollTo(0,document.documentElement.scrollHeight));
 const scrolled=await page.evaluate(()=>scrollY);
 expect(scrolled).toBeGreaterThan(100);
 await cards.last().locator('.wiki-tag').filter({hasText:'#walks'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/walks$/);
 await expect(page.locator('.wiki-active-tag')).toHaveText('#walks');
 // Close: the search, its results and the scroll are as they were.
 const close=page.getByTestId('wiki-close-tag');
 expect((await close.boundingBox()).height).toBeGreaterThanOrEqual(44);
 await close.click();
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(search).toHaveValue('harbour');
 await expect(cards).toHaveCount(10);
 await expect(page.locator('.wiki-active-tag')).toHaveCount(0);
 await expect.poll(()=>page.evaluate(()=>scrollY)).toBeGreaterThan(scrolled-40);
 // A tag opened over a tag: each Close steps back one screen.
 await search.fill('');
 await expect(cards).toHaveCount(12);
 await page.getByRole('complementary',{name:'Tags'}).locator('.wiki-rail-tag').filter({hasText:'#bitcoin'}).click();
 await expect(cards).toHaveCount(2);
 await cards.filter({hasText:'Mining notes'}).locator('.wiki-tag').filter({hasText:'#mining'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/mining$/);
 await expect(cards).toHaveCount(1);
 await close.click();
 await expect(page).toHaveURL(/\/wiki\/tag\/bitcoin$/);
 await expect(cards).toHaveCount(2);
 await close.click();
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(cards).toHaveCount(12);
 // Opened on the map, Close stays on the map.
 await page.getByTestId('wiki-graph-toggle').click();
 await expect(page).toHaveURL(/\/wiki\/map$/);
 const entry=page.locator('wiki-graph .wiki-graph-label.is-tag, .wiki-graph-fallback li button').filter({hasText:'#bitcoin'}).first();
 await expect(entry).toBeVisible();
 await entry.evaluate(el=>el.click());
 await expect(page.locator('.wiki-active-tag')).toHaveText('#bitcoin');
 await close.click();
 await expect(page).toHaveURL(/\/wiki\/map$/);
 await expect(page.locator('.wiki-active-tag')).toHaveCount(0);
 await expect(page.getByTestId('wiki-graph-panel')).toBeVisible();
 // Opened from a message's tag chip, Close reopens that message.
 await page.getByTestId('wiki-feed-toggle').click();
 await cards.filter({hasText:'Bitcoin basics'}).getByTestId('wiki-card-title').click();
 const source=page.getByTestId('wiki-source');
 await expect(source.locator('.wiki-source-title')).toContainText('Bitcoin basics');
 await source.locator('.wiki-tag').filter({hasText:'#bitcoin'}).click();
 await expect(source).toHaveCount(0);
 await expect(page).toHaveURL(/\/wiki\/tag\/bitcoin$/);
 await close.click();
 await expect(page).toHaveURL(/\/wiki\/memo\/[a-f\d-]{36}$/);
 await expect(source.locator('.wiki-source-title')).toContainText('Bitcoin basics');
 await page.getByTestId('wiki-back').click();
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(cards).toHaveCount(12);
});

test('a tag can be deleted from every message, and Delete all in the main menu forgets everything',async({page,request})=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 const first=await saveNote(request,'Mining notes','Mining groups transactions into blocks. #bitcoin #mining');
 await saveNote(request,'Bitcoin basics','Bitcoin uses a distributed ledger. #bitcoin');
 await saveNote(request,'Helpers','```rust\nfn add(a: i32, b: i32) -> i32 { a + b }\n```');
 await page.goto('/wiki');
 const cards=page.locator('.wiki-card');
 await expect(cards).toHaveCount(3);
 const rail=page.getByRole('complementary',{name:'Tags'});
 // A type cannot be deleted; a real tag can, after a confirmation.
 await rail.locator('.wiki-rail-tag').filter({hasText:'#note'}).click();
 await expect(page.getByTestId('wiki-delete-tag')).toHaveCount(0);
 await page.getByTestId('wiki-clear-tag').click();
 await rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'}).click();
 await expect(page).toHaveURL(/\/wiki\/tag\/bitcoin$/);
 await expect(cards).toHaveCount(2);
 await page.getByTestId('wiki-delete-tag').click();
 await expect(page.getByTestId('wiki-delete-tag-dialog')).toContainText('#bitcoin');
 await page.getByRole('button',{name:'Cancel'}).click();
 await expect(page.getByTestId('wiki-delete-tag-dialog')).toHaveCount(0);
 await page.getByTestId('wiki-delete-tag').click();
 await page.getByTestId('wiki-delete-tag-confirm').click();
 // Back on the screen before the tag; the tag is gone from the rail and the cards.
 await expect(page).toHaveURL(/\/wiki$/);
 await expect(cards).toHaveCount(3);
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'})).toHaveCount(0);
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#mining'})).toHaveCount(1);
 await expect(cards.filter({hasText:'Mining notes'}).locator('.wiki-tag')).toHaveText(['#note','#information','#mining']);
 await page.reload();
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#bitcoin'})).toHaveCount(0);
 expect((await (await request.get(`/api/memos/${first.id}`)).json()).tags).toEqual(['mining']);
 expect((await request.delete('/api/wiki/tag/note')).status()).toBe(400);
 // Delete all from the main menu: a confirmation, then nothing of the user's is left.
 const document=(await (await request.get('/api/wiki')).json()).tags.find(t=>t.tag==='mining').document;
 expect((await request.get(`/api/wiki/${document}`)).status()).toBe(200);
 await expect.poll(async()=>(await (await request.get('/api/code')).json()).total).toBe(1);
 const toggle=page.getByRole('button',{name:'Open navigation'});
 if(await toggle.isVisible()) await toggle.click();
 await page.locator('[data-nav="delete-all"]').click();
 const dialog=page.getByTestId('delete-all-dialog');
 await expect(dialog).toContainText('This cannot be undone');
 await page.getByRole('button',{name:'Cancel'}).click();
 await expect(dialog).toHaveCount(0);
 await expect(cards).toHaveCount(3);
 if(await toggle.isVisible()) await toggle.click();
 await page.locator('[data-nav="delete-all"]').click();
 await page.getByTestId('delete-all-confirm').click();
 await expect(page.getByRole('status').filter({hasText:'Everything was deleted'})).toBeVisible();
 await expect(page.getByTestId('delete-all-dialog')).toHaveCount(0);
 const feed=await (await request.get('/api/wiki')).json();
 expect(feed.total).toBe(0);
 expect(feed.tags).toEqual([]);
 expect((await request.get(`/api/wiki/${document}`)).status()).toBe(404);
 expect((await (await request.get('/api/code')).json()).total).toBe(0);
 const memos=await (await request.get('/api/memos')).json();
 expect((Array.isArray(memos)?memos:memos.memos).length).toBe(0);
 expect((await (await request.get('/api/files')).json()).length).toBe(0);
 await page.goto('/wiki');
 await expect(page.locator('.wiki-empty')).toBeVisible();
 await expect(rail.locator('.wiki-rail-tag').filter({hasText:'#'})).toHaveCount(0);
});

test('Wiki fullscreen shows only the Wiki, survives reload, and Esc or the button brings the app back',async({page,request},info)=>{
 await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
 await saveNote(request,'Full screen note','Something to read in peace. #focus');
 await page.goto('/wiki');
 const toggle=page.getByTestId('wiki-fullscreen');
 await expect(toggle).toHaveAttribute('aria-pressed','false');
 await expect(page.locator('.topbar')).toBeVisible();
 await toggle.click();
 await expect(toggle).toHaveAttribute('aria-pressed','true');
 await expect(page.locator('body')).toHaveClass(/wiki-fullscreen/);
 await expect(page.locator('.topbar')).toBeHidden();
 await expect(page.locator('.sidebar')).toBeHidden();
 await expect(page.locator('.page-footer')).toBeHidden();
 await expect(page.getByTestId('wiki-shell')).toHaveClass(/is-fullscreen/);
 await expect(page.locator('.wiki-card')).toHaveCount(1);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
 await page.screenshot({path:`test-results/wiki-fullscreen-${info.project.name}.png`});
 // The mode is remembered for the Wiki, and only there.
 await page.reload();
 await expect(page.locator('body')).toHaveClass(/wiki-fullscreen/);
 await expect(page.locator('.topbar')).toBeHidden();
 await page.goto('/library');
 await expect(page.locator('body')).not.toHaveClass(/wiki-fullscreen/);
 await expect(page.locator('.topbar')).toBeVisible();
 await page.goto('/wiki');
 await expect(page.locator('body')).toHaveClass(/wiki-fullscreen/);
 // Esc leaves it; the button too.
 await page.keyboard.press('Escape');
 await expect(page.locator('body')).not.toHaveClass(/wiki-fullscreen/);
 await expect(page.locator('.topbar')).toBeVisible();
 await expect(toggle).toHaveAttribute('aria-pressed','false');
 await toggle.click();
 await expect(page.locator('.topbar')).toBeHidden();
 await page.getByRole('button',{name:'Exit fullscreen'}).click();
 await expect(page.locator('.topbar')).toBeVisible();
 await page.reload();
 await expect(page.locator('body')).not.toHaveClass(/wiki-fullscreen/);
});
