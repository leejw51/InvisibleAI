import {test, expect} from '@playwright/test';

const PNG = 'iVBORw0KGgoAAAANSUhEUgAAABgAAAAQCAIAAACDRijCAAAAGklEQVR4nGOoiDpBFcQwatCoQaMGjRpEGQIA1rZnH8lcvaMAAAAASUVORK5CYII=';

// Images: pictures can be added right on the page, by the button or by drop.
test('Images page uploads a picture directly, lists it, and files it in the Wiki', async ({page, request}) => {
  await request.patch('/api/assistance/preferences', {data: {wiki_enabled: true, wiki_auto_update: true}});
  const name = `direct-${Date.now()}.png`;
  await page.goto('/images');
  await expect(page.getByRole('heading', {name: 'Every captured moment.'})).toBeVisible();
  await page.getByLabel('Upload image').setInputFiles({name, mimeType: 'image/png', buffer: Buffer.from(PNG, 'base64')});
  const card = page.locator('.image-card').filter({hasText: name});
  await expect(card).toHaveCount(1);
  await expect(card).toContainText('Uploaded image');
  await expect(card.locator('img')).toHaveAttribute('src', /\/api\/files\/.+\/thumbnail$/);
  // The thumbnail is served (and cached) by the backend; the picture is a Wiki photo.
  const thumb = await card.locator('img').getAttribute('src');
  expect((await request.get(thumb)).headers()['content-type']).toBe('image/jpeg');
  await expect.poll(async () => (await (await request.get('/api/wiki?tag=photo')).json()).items.some(i => i.title === name && i.thumbnail === thumb)).toBe(true);
  // The picture's tags show on its card once it is analyzed, and they are the
  // user's to edit right there, like a message in the chat; the Wiki follows.
  await page.getByRole('button', {name: 'Refresh images', exact: true}).click();
  await expect(card.getByTestId('tag-editor').locator('.chat-tag').first()).toHaveText('#photo');
  await card.getByTestId('tag-editor-edit').click();
  await card.getByLabel('Add tag', {exact: true}).fill('harbour');
  await card.getByTestId('tag-editor-save').click();
  await expect(card.getByTestId('tag-editor-form')).toHaveCount(0);
  await expect(card.getByTestId('tag-editor').locator('.chat-tag')).toContainText(['#photo', '#harbour']);
  await expect.poll(async () => (await (await request.get('/api/wiki?tag=harbour')).json()).items.some(i => i.title === name)).toBe(true);
  // Removing the tag again is the same edit.
  await card.getByTestId('tag-editor-edit').click();
  await card.getByRole('button', {name: 'Remove tag harbour', exact: true}).click();
  await card.getByTestId('tag-editor-save').click();
  await expect(card.getByTestId('tag-editor').locator('.chat-tag').filter({hasText: '#harbour'})).toHaveCount(0);
  await expect.poll(async () => (await (await request.get('/api/wiki?tag=harbour')).json()).items.some(i => i.title === name)).toBe(false);
  // A non-image is refused with a message, nothing uploaded.
  await page.getByLabel('Upload image').setInputFiles({name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello')});
  await expect(page.getByRole('alert')).toContainText('Choose an image file');
  await expect(page.locator('.image-card').filter({hasText: 'notes.txt'})).toHaveCount(0);
  // Dropping files on the page uploads them too.
  const dropped = `dropped-${Date.now()}.png`;
  const transfer = await page.evaluateHandle(([png, file]) => {
    const dt = new DataTransfer();
    dt.items.add(new File([Uint8Array.from(atob(png), c => c.charCodeAt(0))], file, {type: 'image/png'}));
    return dt;
  }, [PNG, dropped]);
  await page.getByTestId('images-page').dispatchEvent('drop', {dataTransfer: transfer});
  await expect(page.locator('.image-card').filter({hasText: dropped})).toHaveCount(1);
  await page.setViewportSize({width: 390, height: 844});
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
});
