import {test, expect} from '@playwright/test';
import fs from 'node:fs';

// Videos: a library of clips that upload, play in place from a Range-aware
// stream, filter by name, download and delete.
test('Videos page lists uploaded clips, plays them from a seekable stream, filters and deletes', async ({page, request}, info) => {
  await request.patch('/api/assistance/preferences', {data: {wiki_enabled: true, wiki_auto_update: true}});
  const data = fs.readFileSync('tests/e2e/fixtures/clip.mp4').toString('base64');
  // Other specs upload clip.mp4 too; a unique name keeps this card ours.
  const name = `clip-${Date.now()}.mp4`;
  const up = await request.post('/api/files', {data: {name, data_base64: data}});
  expect(up.ok()).toBeTruthy();
  const file = await up.json();
  await page.goto('/videos');
  await expect(page.getByRole('heading', {name: 'Every clip, ready to play.'})).toBeVisible();
  // A clip can also be added right here, by the button (or by dropping it).
  const direct = `direct-${Date.now()}.mp4`;
  await page.getByLabel('Upload video').setInputFiles({name: direct, mimeType: 'video/mp4', buffer: fs.readFileSync('tests/e2e/fixtures/clip.mp4')});
  await expect(page.getByTestId('video-card').filter({hasText: direct})).toHaveCount(1);
  // A clip added here becomes a Wiki video node with the same frame thumbnail.
  await expect.poll(async () => (await (await request.get('/api/wiki?tag=video')).json()).items.some(i => i.title === direct && /\/thumbnail$/.test(i.thumbnail || '')), {timeout: 20000}).toBe(true);
  // Its tags show on the card and are the user's to edit there; the Wiki follows.
  await page.getByRole('button', {name: 'Refresh videos', exact: true}).click();
  const kept = page.getByTestId('video-card').filter({hasText: direct});
  await expect(kept.getByTestId('tag-editor').locator('.chat-tag').first()).toHaveText('#video');
  await kept.getByTestId('tag-editor-edit').click();
  await kept.getByLabel('Add tag', {exact: true}).fill('pier');
  await kept.getByTestId('tag-editor-save').click();
  await expect(kept.getByTestId('tag-editor').locator('.chat-tag')).toContainText(['#video', '#pier']);
  await expect.poll(async () => (await (await request.get('/api/wiki?tag=pier')).json()).items.some(i => i.title === direct)).toBe(true);
  const card = page.getByTestId('video-card').filter({hasText: name});
  await expect(card).toHaveCount(1);
  await expect(card.locator('img')).toHaveAttribute('src', `/api/files/${file.id}/thumbnail`);
  await expect(card).toContainText('KB');
  // Play in place: the player streams from the backend.
  await card.getByRole('button', {name: `Play ${name}`}).click();
  await expect(card.locator('video')).toHaveAttribute('src', `/api/files/${file.id}/stream`);
  // The stream honours Range requests (Safari needs this to seek) and serves inline.
  const part = await request.get(`/api/files/${file.id}/stream`, {headers: {Range: 'bytes=0-99'}});
  expect(part.status()).toBe(206);
  expect(part.headers()['content-range']).toBe(`bytes 0-99/${file.size}`);
  expect((await part.body()).length).toBe(100);
  const whole = await request.get(`/api/files/${file.id}/stream`);
  expect(whole.status()).toBe(200);
  expect(whole.headers()['accept-ranges']).toBe('bytes');
  expect(whole.headers()['content-disposition']).toBe('inline');
  expect((await request.get(`/api/files/${file.id}/stream`, {headers: {Range: 'bytes=999999-'}})).status()).toBe(416);
  // Search narrows by name; clearing restores.
  await page.getByRole('searchbox', {name: 'Search videos'}).fill('nothing-here');
  await expect(page.getByTestId('video-card')).toHaveCount(0);
  await page.getByRole('searchbox', {name: 'Search videos'}).fill(name.slice(0, 12));
  await expect(card).toHaveCount(1);
  await page.setViewportSize({width: 390, height: 844});
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({path: `test-results/videos-${info.project.name}.png`, fullPage: true});
  // Delete asks once, then removes the clip for good.
  await card.getByRole('button', {name: 'Delete video'}).click();
  await card.getByRole('button', {name: 'Confirm delete video'}).click();
  await expect(page.getByTestId('video-card').filter({hasText: name})).toHaveCount(0);
  expect((await request.get(`/api/files/${file.id}/stream`)).status()).not.toBe(200);
});
