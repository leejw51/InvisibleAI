import {test, expect} from '@playwright/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {openConversationOptions} from './workspace-controls.mjs';

// Videos above the 25 MB single-request limit go up through the resumable
// chunked protocol (offset-checked PATCH appends of raw Blob slices) on the
// Videos page and in chat.
const MB = 1024 * 1024;
const CHUNK = 500 * 1024;
function bigVideo(name, size) {
  const clip = fs.readFileSync('tests/e2e/fixtures/clip.mp4');
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-video-')), name);
  const bytes = Buffer.alloc(size, 7);
  clip.copy(bytes);
  fs.writeFileSync(file, bytes);
  return {file, bytes};
}
function watchUploads(page) {
  const seen = {slices: 0, inFlight: 0, maxInFlight: 0, base64: 0};
  page.on('request', r => {
    if (r.method() === 'PATCH' && /\/api\/uploads\/[^/?]+\?offset=\d+&?$/.test(r.url())) {
      seen.slices++; seen.inFlight++; seen.maxInFlight = Math.max(seen.maxInFlight, seen.inFlight);
      expect(r.headers()['content-type']).toBe('application/octet-stream');
    }
    if (r.method() === 'POST' && r.url().endsWith('/api/files')) seen.base64++;
  });
  const settle = r => { if (r.method() === 'PATCH' && /\/api\/uploads\//.test(r.url())) seen.inFlight--; };
  page.on('requestfinished', settle);
  page.on('requestfailed', settle);
  return seen;
}

test('a 60 MB video uploads in chunks on the Videos page and streams back intact', async ({page, request}, info) => {
  test.setTimeout(120000);
  const name = `big-${info.project.name}-${Date.now()}.mp4`;
  const {file, bytes} = bigVideo(name, 60 * MB);
  const seen = watchUploads(page);
  await page.goto('/videos');
  await page.getByLabel('Upload video').setInputFiles(file);
  const card = page.getByTestId('video-card').filter({hasText: name});
  await expect(card).toHaveCount(1, {timeout: 60000});
  await expect(card).toContainText('MB');
  expect(seen.slices).toBe(Math.ceil(bytes.length / CHUNK));
  // One chunk in flight at a time: the offset is the backend's.
  expect(seen.maxInFlight).toBe(1);
  expect(seen.base64).toBe(0);
  const stored = (await (await request.get('/api/files')).json()).find(f => f.name === name);
  expect(stored.size).toBe(bytes.length);
  expect(stored.mime).toBe('video/mp4');
  // The stored bytes match exactly, read back through the Range-aware stream.
  for (const [start, end] of [[0, 99], [8 * MB - 50, 8 * MB + 49], [bytes.length - 100, bytes.length - 1]]) {
    const part = await request.get(`/api/files/${stored.id}/stream`, {headers: {Range: `bytes=${start}-${end}`}});
    expect(part.status()).toBe(206);
    expect(Buffer.compare(await part.body(), bytes.subarray(start, end + 1))).toBe(0);
  }
  const download = await request.get(`/api/files/${stored.id}/download`);
  expect(download.headers()['content-disposition']).toContain('attachment');
  expect(Number(download.headers()['content-length'])).toBe(bytes.length);
  await card.getByRole('button', {name: 'Delete video'}).click();
  await card.getByRole('button', {name: 'Confirm delete video'}).click();
  await expect(card).toHaveCount(0);
  fs.rmSync(path.dirname(file), {recursive: true, force: true});
});

test('a large video attached in chat shows progress and becomes an attachment', async ({page}, info) => {
  test.setTimeout(120000);
  const name = `chat-${info.project.name}-${Date.now()}.mov`;
  const {file, bytes} = bigVideo(name, 30 * MB);
  const seen = watchUploads(page);
  await page.goto('/chat');
  await openConversationOptions(page);
  // New chat unmounts the conversation-options fold while the room is created
  // and remounts it closed: a label query still reads the hidden picker.
  const rooms = page.getByLabel('Chat room', {exact: true});
  const before = await rooms.inputValue();
  await page.getByRole('button', {name: 'New chat', exact: true}).click();
  await expect(rooms).not.toHaveValue(before);
  await expect(rooms).toHaveValue(/^[a-f\d-]{36}$/);
  // The file input lives inside the fold too.
  await openConversationOptions(page);
  // Hold the final step so the progress bar is observable. (Chunks themselves
  // are never intercepted: WebKit's request interception drops Blob bodies.)
  let release;
  const held = new Promise(r => { release = r; });
  await page.route('**/api/uploads/*/finish', async route => { await held; await route.continue(); });
  await page.getByLabel('Attach to chat', {exact: true}).setInputFiles(file);
  const progress = page.getByRole('progressbar', {name: 'Uploading video'});
  await expect(progress).toBeVisible();
  await expect(progress).toHaveAttribute('aria-valuenow', '100', {timeout: 60000});
  release();
  await expect(page.locator('.chat-attachment').filter({hasText: name})).toHaveCount(1, {timeout: 60000});
  await expect(progress).toHaveCount(0);
  expect(seen.slices).toBe(Math.ceil(bytes.length / CHUNK));
  expect(seen.base64).toBe(0);
  fs.rmSync(path.dirname(file), {recursive: true, force: true});
});

test('a video over 1 GB is refused before anything is sent', async ({page}) => {
  await page.goto('/videos');
  const refused = await page.evaluate(async () => {
    const response = await fetch('/api/uploads', {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({name: 'huge.mp4', size: 1024 ** 3 + 1})});
    return [response.status, (await response.json()).error];
  });
  expect(refused).toEqual([400, 'Videos must be 1 GB or smaller']);
});

test('re-picking an interrupted video resumes from where the backend stopped', async ({page, request}, info) => {
  const crypto = await import('node:crypto');
  const name = `resume-${info.project.name}-${Date.now()}.mp4`;
  const {file, bytes} = bigVideo(name, 3 * MB);
  const sha256 = crypto.createHash('sha256').update(bytes).digest('hex');
  // An earlier attempt got two chunks onto the backend before the page went away.
  const started = await (await request.post('/api/uploads', {data: {name, size: bytes.length, sha256, wiki: true}})).json();
  for (const offset of [0, CHUNK]) {
    const ack = await request.patch(`/api/uploads/${started.id}?offset=${offset}`, {headers: {'content-type': 'application/octet-stream'}, data: bytes.subarray(offset, offset + CHUNK)});
    expect(ack.ok()).toBeTruthy();
  }
  await page.goto('/videos');
  await page.evaluate(([key, id]) => localStorage.setItem(key, id), [`causewaybay.upload.resume.videos:${sha256}:${bytes.length}`, started.id]);
  const offsets = [];
  page.on('request', r => { const m = r.method() === 'PATCH' && r.url().match(/offset=(\d+)/); if (m) offsets.push(Number(m[1])); });
  await page.getByLabel('Upload video').setInputFiles(file);
  const card = page.getByTestId('video-card').filter({hasText: name});
  await expect(card).toHaveCount(1, {timeout: 60000});
  // Only the missing chunks were sent, and no new session was opened.
  expect(offsets[0]).toBe(2 * CHUNK);
  expect(offsets).toHaveLength(Math.ceil(bytes.length / CHUNK) - 2);
  const stored = (await (await request.get('/api/files')).json()).find(f => f.name === name);
  const whole = await request.get(`/api/files/${stored.id}/download`);
  expect(Buffer.compare(await whole.body(), bytes)).toBe(0);
  expect(await page.evaluate(id => Object.values(localStorage).includes(id), started.id)).toBe(false);
  fs.rmSync(path.dirname(file), {recursive: true, force: true});
});
