import {openConversationOptions} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';
// A 24x16 solid PNG; Apple Vision finds no text in it, and the picture is still kept.
const PNG='iVBORw0KGgoAAAANSUhEUgAAABgAAAAQCAIAAACDRijCAAAAGklEQVR4nGOoiDpBFcQwatCoQaMGjRpEGQIA1rZnH8lcvaMAAAAASUVORK5CYII=';

// Keeping without a chat reply: drop or attach files, press Enter or Save memo, and the
// files are kept in the chat, indexed, and organized into the Wiki in the background.
test('New thought keeps dropped files and Markdown as a Wiki source without a chat reply', async ({page, request}, info) => {
  const submissions = [];
  page.on('websocket', ws => ws.on('framesent', frame => {
    const message = JSON.parse(String(frame.payload));
    if (message.path === '/api/think' || message.path === '/api/memos') submissions.push(message);
  }));
  await request.patch('/api/assistance/preferences', {data:{wiki_enabled:true, wiki_auto_update:true}});
  await page.goto('/'); await openConversationOptions(page);
  await expect(page.getByRole('heading', {name:'Hey, what’s on your mind?', exact:true})).toBeVisible();
  const rooms = page.getByRole('combobox', {name:'Chat room', exact:true});
  await expect(rooms).not.toHaveValue('');
  // Start a fresh conversation for this file-only save.
  const previous = await rooms.inputValue();
  await page.getByRole('button', {name:'New chat', exact:true}).click(); await openConversationOptions(page);
  await expect(rooms).not.toHaveValue(previous);
  await expect(rooms).not.toHaveValue('');
  const room = await rooms.inputValue();
  // The empty chat invites a drop and explains what happens; nothing must be configured first.
  await expect(page.locator('.chat-placeholder')).toContainText('Ask a question, share a photo');
  await expect(page.locator('.chat-dock textarea')).toHaveAttribute('placeholder', 'Type, paste, or drop anything…');

  // Drop a Markdown note and a picture onto the page: both are staged like Add any file.
  const codeword = `ORCHID-${Date.now()}`;
  const transfer = await page.evaluateHandle(([text, png, name]) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], name, {type:'text/markdown'}));
    dt.items.add(new File([Uint8Array.from(atob(png), c => c.charCodeAt(0))], 'drop.png', {type:'image/png'}));
    return dt;
  }, [`# Launch brief\nThe codeword is ${codeword}. #launch`, PNG, 'launch notes.md']);
  await page.locator('.chat-page').dispatchEvent('drop', {dataTransfer: transfer});
  await expect(page.locator('.chat-attachment')).toHaveCount(2);
  await expect(page.locator('.attachment-details')).toContainText(['Text context', 'Image context']);
  expect(submissions).toEqual([]);

  // Save memo keeps files alone: a chat message with the files, no AI reply.
  const save = page.getByRole('button', {name:'Save memo', exact:true});
  await expect(save).toBeEnabled();
  await expect(save).toHaveAttribute('title', 'Save without a chat reply; your selected AI may tidy and tag the note');
  await save.click();
  await expect(page.locator('.from-user .chat-file')).toHaveCount(2);
  await expect(page.locator('.from-user')).toContainText('Kept as a note');
  await expect(page.locator('.from-ai')).toHaveCount(0);
  await expect(page.locator('.chat-attachment')).toHaveCount(0);
  expect(submissions.map(s => s.path)).toEqual(['/api/memos']);
  expect(submissions[0].body.file_ids).toHaveLength(2);
  expect(submissions[0].body.content).toBe('');

  // The memo carries the file's text, so search and the Wiki find it.
  const batch = await (await request.get(`/api/rooms/${room}/sync`)).json();
  const kept = batch.messages.find(m => m.role === 'user' && !m.deleted && m.provider === 'memo');
  const memo = await (await request.get(`/api/memos/${kept.memo_id}`)).json();
  expect(memo.title).toBe('launch notes.md, drop.png');
  expect(memo.input).toContain(codeword);
  // File-only saves retain searchable attachments, without rewriting extracted
  // file text or backend scaffolding into a note the user never typed.
  expect(memo.kind).toBe('memo');
  expect(memo.answer).toBe('');
  const found = await (await request.get(`/api/memos?q=${encodeURIComponent(codeword)}`)).json();
  expect(found.memos.map(m => m.id)).toContain(kept.memo_id);
  await expect.poll(async () => {
    const wiki = await (await request.get(`/api/wiki?q=${encodeURIComponent(codeword)}`)).json();
    return wiki.items.flatMap(i => i.tags);
  }).toContain('launch');
  if(!(await page.locator('[data-nav="wiki"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.locator('[data-nav="wiki"]').click();
  await page.getByRole('searchbox', {name:'Search Wiki'}).fill(codeword);
  await expect(page.locator('.wiki-card').filter({has:page.locator('.wiki-tag',{hasText:'#launch'})})).toHaveCount(1);

  // Typing a follow-up keeps working as before and sends the kept file as context.
  await page.goto('/'); await openConversationOptions(page);
  await page.getByRole('textbox', {name:'Chat message', exact:true}).fill('What is the codeword in the brief?');
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  await expect(page.locator('.from-ai')).toHaveCount(1);
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  const think = submissions.find(s => s.path === '/api/think');
  expect(think.body.prompt).toBe('What is the codeword in the brief?');
  const image = info.outputPath('keep-new-thought.png');
  await page.screenshot({path:image, fullPage:true});
  await info.attach('keep-new-thought', {path:image, contentType:'image/png'});
});

test('Save memo keeps a typed thought as a memo without an AI reply', async ({page, request}) => {
  await page.goto('/'); await openConversationOptions(page);
  await expect(page.getByRole('heading', {name:'Hey, what’s on your mind?', exact:true})).toBeVisible();
  const rooms = page.getByRole('combobox', {name:'Chat room', exact:true});
  await expect(rooms).not.toHaveValue('');
  const room = await rooms.inputValue();
  const text = `Keep this thought ${Date.now()} #kept`;
  await page.getByRole('textbox', {name:'Chat message', exact:true}).fill(text);
  await page.getByRole('button', {name:'Save memo', exact:true}).click();
  const kept_message = page.locator('.from-user').filter({hasText:text});
  await expect(kept_message).toHaveCount(1);
  await expect(kept_message).toContainText('Kept as a note');
  await expect(page.locator('.from-ai').filter({hasText:text})).toHaveCount(0);
  const batch = await (await request.get(`/api/rooms/${room}/sync`)).json();
  const kept = batch.messages.find(m => !m.deleted && m.provider === 'memo' && m.text === text);
  const memo = await (await request.get(`/api/memos/${kept.memo_id}`)).json();
  expect(memo.tags).toEqual(['kept']);
  expect(memo.title).toBe(text);
  await expect.poll(async () => (await (await request.get(`/api/memos/${kept.memo_id}`)).json()).kind).toBe('note');
  const tidied = await (await request.get(`/api/memos/${kept.memo_id}`)).json();
  expect(tidied.input).toBe(text);
  expect(tidied.answer).toContain(`- ${text}`);
  await expect(kept_message).toContainText(text);
  await expect(page.locator('.from-ai').filter({hasText:text})).toHaveCount(0);
});

test('the AI model menu in a chat changes the answering AI and saves it to the account', async ({page, request}) => {
  const local={local_only:false,apple_enabled:true,mlx_enabled:false,mlx_url:'http://127.0.0.1:8080/v1',speech_provider:'auto',speech_locale:'en-US',mlx_speech_model:'',auto_interval_seconds:60,local_interval_seconds:5,cloud_min_interval_seconds:60};
  await request.patch('/api/settings',{data:{local_ai:local}});
  expect((await request.patch('/api/ai/preferences',{data:{mode:'ondevice',local_provider:'apple',cloud_provider:'grok'}})).ok()).toBeTruthy();
  try {
    await page.goto('/chat');
    await openConversationOptions(page);
    await page.locator('.chat-options > summary').click();
    const menu = page.getByTestId('chat-ai-select');
    await expect(menu).toBeVisible();
    await expect(menu).toHaveValue('apple');
    await menu.selectOption('openai');
    await expect.poll(async () => (await (await request.get('/api/ai/preferences')).json())).toMatchObject({mode:'cloudai', cloud_provider:'openai'});
    await expect(menu).toHaveValue('openai');
    await page.getByRole('textbox', {name:'Chat message', exact:true}).fill('Which model answers now?');
    await page.getByRole('button', {name:'Send message', exact:true}).click();
    await expect(page.locator('.from-ai .chat-message-meta').last()).toContainText('openai');
    await menu.selectOption('apple');
    await expect.poll(async () => (await (await request.get('/api/ai/preferences')).json())).toMatchObject({mode:'ondevice', local_provider:'apple'});
    await page.reload();
    await openConversationOptions(page);
    await page.locator('.chat-options > summary').click();
    await expect(page.getByTestId('chat-ai-select')).toHaveValue('apple');
  } finally {
    await request.patch('/api/ai/preferences',{data:{mode:'cloudai',local_provider:'apple',cloud_provider:'grok'}});
    await request.patch('/api/settings',{data:{local_ai:{...local,apple_enabled:false}}});
  }
});
