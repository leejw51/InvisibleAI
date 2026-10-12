import {openConversationOptions} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';

async function newChat(page) {
  await page.goto('/'); await openConversationOptions(page);
  const selector = page.getByRole('combobox', {name: 'Chat room', exact: true});
  await expect(selector).not.toHaveValue('');
  const previous = await selector.inputValue();
  await page.getByRole('button', {name: 'New chat', exact: true}).click();
  await openConversationOptions(page);
  await expect(selector).not.toHaveValue(previous);
  await expect(selector).not.toHaveValue('');
  await expect(page.getByRole('button', {name: 'Multi Capture', exact: true})).toBeEnabled();
  return selector.inputValue();
}

async function liveMessages(request, room) {
  const response = await request.get(`/api/rooms/${room}/sync`);
  expect(response.ok()).toBe(true);
  return (await response.json()).messages.filter(message => !message.deleted);
}

for (const action of ['clear', 'all']) {
  test(`offline staged Multi Capture photos disappear after another session performs ${action}`, async ({page, browser, request}, testInfo) => {
    const otherContext = await browser.newContext();
    const other = await otherContext.newPage();
    try {
      const room = await newChat(page);
      const prompt = page.getByRole('textbox', {name: 'Chat message', exact: true});
      await prompt.fill('Private draft that was never sent');
      const capture = page.getByRole('button', {name: 'Multi Capture', exact: true});
      for (let count = 1; count <= 2; count++) {
        await capture.click();
        await expect(page.locator('.chat-attachment')).toHaveCount(count);
      }
      await expect(page.getByRole('button', {name: 'Send message', exact: true})).toBeEnabled();
      expect(await liveMessages(request, room)).toEqual([]);
      const files = await page.locator('.chat-attachment a').evaluateAll(links => links.map(link => new URL(link.href).pathname.split('/')[3]));
      expect(new Set(files).size).toBe(2);
      for (const id of files) expect((await request.get(`/api/files/${id}/download`)).status()).toBe(200);
      await testInfo.attach('unsent-captures', {body: await page.screenshot({fullPage: true}), contentType: 'image/png'});

      await other.goto('/chat'); await openConversationOptions(other);
      await other.getByRole('combobox', {name: 'Chat room', exact: true}).selectOption(room);
      await expect(other.locator('.chat-page').getByRole('button', {name: 'Capture', exact: true})).toBeEnabled();
      await page.context().setOffline(true);
      if (action === 'clear') {
        await other.getByRole('button', {name: 'Clear chatroom', exact: true}).click();
        await other.getByRole('button', {name: 'Confirm clear chatroom', exact: true}).click();
      } else {
        const toggle = other.getByRole('button', {name: 'Open navigation', exact: true});
        if (await toggle.isVisible() && await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
        await other.locator('.everyday-settings > summary').click();
        await other.locator('[data-nav="delete-all"]').click();
        await other.getByTestId('delete-all-confirm').click();
      }
      for (const id of files) {
        await expect.poll(async () => (await request.get(`/api/files/${id}/download`)).status()).toBe(404);
        expect((await request.get(`/api/files/${id}/thumbnail`)).status()).toBe(404);
      }
      await page.context().setOffline(false);
      await expect(page.locator('.chat-attachment')).toHaveCount(0);
      await expect(page.locator('.chat-message')).toHaveCount(0);
      const selector = page.getByRole('combobox', {name: 'Chat room', exact: true});
      if (action === 'clear') {
        await expect(selector).toHaveValue(room);
        await expect(prompt).toHaveValue('');
      } else {
        await expect(selector.locator(`option[value="${room}"]`)).toHaveCount(0);
        await page.goto('/'); await openConversationOptions(page);
        await expect(selector).not.toHaveValue('');
        await expect(selector).not.toHaveValue(room);
      }
      // A fresh capture must work after replay without including the erased draft/photos.
      await page.locator('.chat-page').getByRole('button', {name: 'Capture', exact: true}).click();
      await expect(page.locator('.from-user')).toHaveCount(1);
      await expect(page.locator('.from-ai')).toHaveCount(1);
      await expect(page.getByRole('button', {name: 'Stop answer', exact: true})).toHaveCount(0);
      const fresh = await liveMessages(request, await selector.inputValue());
      expect(fresh).toHaveLength(2);
      expect(JSON.stringify(fresh)).not.toContain('Private draft that was never sent');
      for (const id of files) expect(JSON.stringify(fresh)).not.toContain(id);
      await page.reload();
      await expect(page.locator('.from-user')).toHaveCount(1);
      await expect(page.locator('.chat-attachment')).toHaveCount(0);
      await testInfo.attach('after-erasure-and-fresh-capture', {body: await page.screenshot({fullPage: true}), contentType: 'image/png'});
    } finally {
      await page.context().setOffline(false);
      await otherContext.close();
    }
  });
}

test('Multi Capture enforces eight photos, disables empty Send message, and sends exactly one response', async ({page, request}, testInfo) => {
  const room = await newChat(page);
  const capture = page.getByRole('button', {name: 'Multi Capture', exact: true});
  const done = page.getByRole('button', {name: 'Send message', exact: true});
  const submitted = [];
  page.on('websocket', socket => socket.on('framesent', frame => {
    try {
      const message = JSON.parse(String(frame.payload));
      if (message.path === '/api/think') submitted.push(message.body);
    } catch { /* Ignore transport control frames. */ }
  }));
  // Attach to the already-open WebSocket through a reload before interacting.
  await page.reload(); await openConversationOptions(page);
  await page.getByRole('combobox', {name: 'Chat room', exact: true}).selectOption(room);
  for (let count = 1; count <= 8; count++) {
    await capture.click();
    await expect(page.locator('.chat-attachment')).toHaveCount(count);
  }
  await expect(capture).toBeDisabled();
  await expect(page.locator('.chat-page').getByRole('button', {name: 'Capture', exact: true})).toBeDisabled();
  await expect(done).toBeEnabled();
  expect(await liveMessages(request, room)).toEqual([]);
  expect(submitted).toEqual([]);
  await testInfo.attach('eight-staged-captures', {body: await page.screenshot({fullPage: true}), contentType: 'image/png'});
  for (let count = 7; count >= 0; count--) {
    await page.getByRole('button', {name: /^Remove attachment:/}).first().click();
    await expect(page.locator('.chat-attachment')).toHaveCount(count);
  }
  await expect(done).toBeDisabled();
  await expect(capture).toBeEnabled();
  await capture.click();
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await done.click();
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-ai')).toHaveCount(1);
  await expect(page.getByRole('button', {name: 'Stop answer', exact: true})).toHaveCount(0);
  expect(submitted).toHaveLength(1);
  expect(submitted[0].capture).toBe(false);
  expect(submitted[0].file_ids).toHaveLength(1);
  expect(await liveMessages(request, room)).toHaveLength(2);
  await expect(done).toBeDisabled();
});
