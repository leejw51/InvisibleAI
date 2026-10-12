import {test, expect} from '@playwright/test';

// The libraries and tools sit behind one fold ("More tools"); open it when needed.
async function nav(page, route) {
  const item = page.locator(`[data-nav="${route}"]`);
  if (!(await item.isVisible())) await page.locator('.more-tools > summary').click();
  await item.click();
}

test('every main menu destination has a reloadable URL and browser history', async ({page}) => {
  await page.goto('/');
  await expect(page.getByRole('heading', {name:'Hey, what’s on your mind?', exact:true})).toBeVisible();
  await expect(page.locator('.more-tools')).not.toHaveAttribute('open');
  await expect(page.getByRole('button', {name:'Coding agent', exact:true})).toHaveCount(0);
  await page.screenshot({path:'test-results/navigation-desktop.png', fullPage:true});
  for (const route of ['chat', 'library', 'files', 'dashboard', 'images', 'devices']) {
    await nav(page, route);
    await expect(page).toHaveURL(new RegExp(`/${route}$`));
    await expect(page.locator(`[data-nav="${route}"]`)).toHaveAttribute('aria-current', 'page');
    expect((await page.reload()).status()).toBe(200);
    await expect(page.locator(`[data-nav="${route}"]`)).toHaveAttribute('aria-current', 'page');
  }
  await expect(page.getByLabel('Capture client command')).toHaveText('./bin/client http://127.0.0.1:3099');
  await page.goBack(); await expect(page).toHaveURL(/\/images$/);
  await expect(page.locator('[data-nav="images"]')).toHaveAttribute('aria-current', 'page');
  await page.goForward(); await expect(page.getByRole('heading', {name:'Your connected devices.'})).toBeVisible();
  await page.locator('.more-tools > summary').click();
  for (const route of ['overview', 'quiz']) {
    await page.locator(`[data-nav="${route}"]`).click();
    expect((await page.reload()).status()).toBe(200);
    await expect(page.locator(`[data-nav="${route}"]`)).toHaveAttribute('aria-current', 'page');
  }
  await page.getByRole('button', {name:'Search your notes'}).click();
  await expect(page).toHaveURL(/\/library$/);
  await expect(page.locator('#memo-search')).toBeFocused();
});

test('saved memo details survive reload and browser back/forward', async ({page}) => {
  await page.goto('/library');
  await page.getByRole('button', {name:'New memo', exact:true}).click();
  await page.getByLabel('Memo title').fill('Navigation memo');
  await page.getByLabel('Memo content').fill('Private memo for navigation history.');
  await page.getByRole('button', {name:'Save memo', exact:true}).click();
  await expect(page).toHaveURL(/\/memos\/[a-f\d-]{36}$/);
  const url = page.url();
  await expect(page.locator('.answer-content')).toContainText('Private memo for navigation history.');
  expect((await page.reload()).status()).toBe(200);
  await expect(page.locator('.answer-content')).toContainText('Private memo for navigation history.');
  await nav(page, 'files');
  await expect(page).toHaveURL(/\/files$/);
  await page.goBack(); await expect(page).toHaveURL(url);
  await expect(page.locator('.answer-content')).toContainText('Private memo for navigation history.');
  await page.goBack(); await expect(page).toHaveURL(/\/library$/);
  await expect(page.locator('.memo-card').filter({hasText:'Navigation memo'})).toBeVisible();
});

test('new chats leave New thought; shortcuts send only from the visible composer', async ({page}) => {
  const requests = [];
  page.on('websocket', ws => ws.on('framesent', frame => {
    const data = JSON.parse(String(frame.payload));
    if (data.path === '/api/think') requests.push(data);
  }));
  await page.goto('/files');
  await page.keyboard.press('Control+Enter');
  await page.getByRole('button', {name:'New thought', exact:true}).click();
  await expect(page.getByLabel('Chat room', {exact:true})).not.toHaveValue('');
  const thought = await page.getByLabel('Chat room', {exact:true}).inputValue();
  await page.getByRole('button', {name:'New chat', exact:true}).click();
  await expect(page).toHaveURL(/\/chat$/);
  await expect(page.getByLabel('Chat room', {exact:true})).not.toHaveValue(thought);
  await expect(page.getByLabel('Chat room', {exact:true})).not.toHaveValue('');
  const room = await page.getByLabel('Chat room', {exact:true}).inputValue();
  const composer = page.getByRole('textbox', {name:'Chat message', exact:true});
  await composer.fill('Exactly one visible message');
  await composer.press('Control+Enter');
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-user')).toContainText('Exactly one visible message');
  await expect(page.getByLabel('Chat room', {exact:true})).toHaveValue(room);
  expect(requests).toHaveLength(1);
  expect(requests[0].body).toMatchObject({capture:false, room_id:room});
  await page.getByRole('button', {name:'New thought', exact:true}).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByLabel('Chat room', {exact:true})).not.toHaveValue(thought);
  await expect(page.getByLabel('Chat room', {exact:true})).not.toHaveValue(room);
  await expect(page.locator('.chat-message')).toHaveCount(0);
});

test('Files → Use in chat attaches a room-owned copy and clear preserves the library original', async ({page, request}) => {
  const uploaded = await request.post('/api/files', {data:{name:'library-chat-flow.txt', data_base64:Buffer.from('A library file to discuss.').toString('base64')}});
  expect(uploaded.ok()).toBe(true);
  const original = await uploaded.json();
  await page.goto('/files');
  await page.locator('.file-card').filter({hasText:'library-chat-flow.txt'}).getByRole('button', {name:'Use in chat', exact:true}).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator('.chat-attachment')).toContainText('library-chat-flow.txt');
  const room = await page.getByLabel('Chat room', {exact:true}).inputValue();
  await page.getByRole('textbox', {name:'Chat message', exact:true}).fill('Explain this file');
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  const attachment = page.locator('.chat-file a').filter({hasText:'library-chat-flow.txt'}).last();
  await expect(attachment).toBeVisible();
  const download = await attachment.getAttribute('href');
  expect(download).not.toBe(`/api/files/${original.id}/download`);
  await page.getByRole('button', {name:'Clear chatroom'}).click();
  await page.getByRole('button', {name:'Confirm clear chatroom'}).click();
  await expect(page.locator('.chat-message')).toHaveCount(0);
  await expect(page.getByLabel('Chat room', {exact:true})).toHaveValue(room);
  expect((await request.get(download)).status()).toBe(404);
  expect((await request.get(`/api/files/${original.id}/download`)).ok()).toBe(true);
});

test('mobile navigation closes after selection, Escape, and opening settings', async ({page}) => {
  await page.setViewportSize({width:390, height:844});
  await page.goto('/');
  const toggle = page.getByRole('button', {name:'Open navigation'});
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await page.screenshot({path:'test-results/navigation-mobile.png', fullPage:true});
  await nav(page, 'files');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(page).toHaveURL(/\/files$/);
  await toggle.click(); await page.keyboard.press('Escape');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await toggle.click(); await page.keyboard.press('Escape');
  await page.getByRole('button', {name:'Open settings', exact:true}).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await page.getByRole('button', {name:'Close dialog'}).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
