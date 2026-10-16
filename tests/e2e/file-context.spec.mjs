import {openConversationOptions} from './workspace-controls.mjs';
import {test, expect} from '@playwright/test';
import {documents} from './document-fixtures.mjs';

function sound() {
  const bytes = Buffer.alloc(44 + 16000);
  bytes.write('RIFF'); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
  bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(8000, 24); bytes.writeUInt32LE(16000, 28);
  bytes.writeUInt16LE(2, 32); bytes.writeUInt16LE(16, 34); bytes.write('data', 36);
  bytes.writeUInt32LE(16000, 40);
  return bytes; // Valid, one-second silent PCM WAV; demo transcription is explicit.
}

test('Add files uses Markdown, audio, and arbitrary files as context, then clears saved files and transcripts', async ({page, request}, info) => {
  const submissions = [], downloads = [];
  page.on('websocket', ws => ws.on('framesent', frame => {
    const message = JSON.parse(String(frame.payload));
    if (message.path === '/api/think') submissions.push(message.body);
  }));
  page.on('request', r => { if (/\/api\/files\/[^/]+\/download$/.test(r.url())) downloads.push(r.url()); });
  // Home opens today's thought, so a room exists even on a fresh demo backend.
  await page.goto('/'); await openConversationOptions(page);
  const rooms = page.getByRole('combobox', {name:'Chat room', exact:true});
  await expect(rooms).not.toHaveValue('');
  await page.goto('/chat'); await openConversationOptions(page);
  await expect(rooms).not.toHaveValue('');
  const previous = await rooms.inputValue();
  await page.getByRole('button', {name:'New chat', exact:true}).click();
  await openConversationOptions(page);
  await expect(rooms).not.toHaveValue(previous);
  await expect(rooms).not.toHaveValue('');
  const room = await rooms.inputValue();
  const picker = page.getByLabel('Attach to chat', {exact:true});
  await expect(page.locator('.upload-label')).toContainText('Add any file');
  expect(await picker.getAttribute('accept')).toBeNull();
  await expect(picker).toHaveAttribute('title', 'Any file · up to 8 files, 25 MB each');
  const markdown = {name:'회의 notes.md', mimeType:'text/markdown', buffer:Buffer.from('# Brief\nContext password: ORCHID-48.\nUse the blue design.')};
  await picker.setInputFiles([
    markdown,
    {name:'voice.wav', mimeType:'audio/wav', buffer:sound()},
    {name:'opaque.data', mimeType:'application/octet-stream', buffer:Buffer.from([0,255,1])},
  ]);
  await expect(page.locator('.chat-attachment')).toHaveCount(3);
  await expect(page.locator('.attachment-details')).toContainText(['Text context', 'Audio → transcript', 'Metadata only']);
  await expect(page.getByText('Files marked Metadata only include their name, size, and file signature; their contents cannot be decoded.')).toBeVisible();
  await expect(page.getByLabel('Preview voice.wav')).toHaveAttribute('preload', 'none');
  expect(downloads).toEqual([]);
  expect(submissions).toEqual([]);
  // Removing one staged item preserves the other files, and choosing it again works.
  await page.getByRole('button', {name:'Remove attachment: 회의 notes.md', exact:true}).click();
  await expect(page.locator('.chat-attachment')).toHaveCount(2);
  await picker.setInputFiles(markdown);
  await expect(page.locator('.chat-attachment')).toHaveCount(3);
  for (const mode of ['light', 'dark']) {
    if (await page.locator('html').getAttribute('data-theme') !== mode) await page.getByTestId('theme-toggle').click();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(page.viewportSize().width + 1);
    const image = info.outputPath(`file-context-${mode}.png`);
    await page.screenshot({path:image, fullPage:true});
    await info.attach(`file-context-${mode}`, {path:image, contentType:'image/png'});
  }
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  await expect(page.locator('.from-user .chat-file')).toHaveCount(3);
  await expect(page.locator('.from-user')).toContainText('What is seven plus five?');
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  expect(submissions).toHaveLength(1);
  expect(submissions[0].prompt).toBe('');
  expect(submissions[0].file_ids).toHaveLength(3);
  const batch = await (await request.get(`/api/rooms/${room}/sync`)).json();
  const message = batch.messages.find(m => m.role === 'user' && !m.deleted);
  const memo = await (await request.get(`/api/memos/${message.memo_id}`)).json();
  expect(memo.input).toContain('ORCHID-48');
  expect(memo.input).toContain('Audio transcript (demo fixture)');
  expect(memo.input).toContain('no content decoder');
  await page.reload(); await openConversationOptions(page);
  await rooms.selectOption(room);
  await expect(page.locator('.from-user')).toContainText('What is seven plus five?');
  await expect(page.locator('.from-user audio')).toHaveAttribute('preload', 'none');
  await page.getByRole('textbox', {name:'Chat message', exact:true}).fill('Use the earlier brief and voice note.');
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  await expect(page.locator('.from-ai')).toHaveCount(2);
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Clear chatroom', exact:true}).click();
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Confirm clear chatroom', exact:true}).click();
  await expect(page.locator('.chat-file, .chat-attachment, .chat-ocr')).toHaveCount(0);
  for (const id of submissions[0].file_ids) expect((await request.get(`/api/files/${id}/download`)).status()).toBe(404);
  expect((await request.get(`/api/memos/${message.memo_id}`)).status()).toBe(404);
  await page.reload();
  await expect(page.locator('.chat-file, .chat-attachment, .chat-ocr')).toHaveCount(0);
});

test('Any file accepts PDF, Office, OpenDocument, ZIP, dotfiles, and extensionless text with readable context and permanent cleanup', async ({page, request}, info) => {
  // Home opens today's thought, so a room exists even on a fresh demo backend.
  await page.goto('/'); await openConversationOptions(page);
  const rooms = page.getByRole('combobox', {name:'Chat room', exact:true});
  await expect(rooms).not.toHaveValue('');
  await page.goto('/chat'); await openConversationOptions(page);
  await expect(rooms).not.toHaveValue('');
  const previous = await rooms.inputValue();
  await page.getByRole('button', {name:'New chat', exact:true}).click();
  await openConversationOptions(page);
  await expect(rooms).not.toHaveValue(previous);
  await expect(rooms).not.toHaveValue('');
  const room = await rooms.inputValue();
  const files = documents();
  await page.getByLabel('Attach to chat', {exact:true}).setInputFiles(files);
  await expect(page.locator('.chat-attachment')).toHaveCount(8);
  await expect(page.locator('.attachment-details')).toContainText([
    'Document → text', 'Document → text', 'Document → text', 'Document → text',
    'Document → text', 'Archive listing', 'Text context', 'Text context',
  ]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(page.viewportSize().width + 1);
  const screenshot = info.outputPath('documents-staged.png');
  await page.screenshot({path:screenshot, fullPage:true});
  await info.attach('documents-staged', {path:screenshot, contentType:'image/png'});
  await page.getByRole('textbox', {name:'Chat message', exact:true}).fill('Summarize these documents.');
  await page.getByRole('button', {name:'Send message', exact:true}).click();
  await expect(page.locator('.from-user .chat-file')).toHaveCount(8);
  await expect(page.locator('.from-ai')).toHaveCount(1);
  await expect(page.getByRole('button', {name:'Stop answer', exact:true})).toHaveCount(0);
  const batch = await (await request.get(`/api/rooms/${room}/sync`)).json();
  const message = batch.messages.find(m => m.role === 'user' && !m.deleted);
  const memo = await (await request.get(`/api/memos/${message.memo_id}`)).json();
  expect(memo.status, memo.error).toBe('complete');
  for (const text of ['PDF-CONTEXT-724', 'WORD-CONTEXT-725 회의 & notes', 'SHEET-CONTEXT-726', 'B1: "42"', 'SLIDE-CONTEXT-727', 'OPEN-CONTEXT-728', 'archive-reference-729.txt', 'DOTFILE-CONTEXT-730', 'EXTENSIONLESS-CONTEXT-731']) {
    expect(memo.input).toContain(text);
  }
  expect(memo.input).not.toContain('NESTED-CONTENT-NOT-READ');
  expect(message.attachments).toHaveLength(8);
  for (const file of message.attachments) {
    const response = await request.get(`/api/files/${file.id}/download`);
    expect(response.ok()).toBeTruthy();
    expect(await response.body()).toEqual(files.find(f => f.name === file.name).buffer);
  }
  await page.reload(); await openConversationOptions(page);
  await rooms.selectOption(room);
  await expect(page.locator('.from-user .chat-file')).toHaveCount(8);
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Clear chatroom', exact:true}).click();
  await openConversationOptions(page);
  await page.getByRole('button', {name:'Confirm clear chatroom', exact:true}).click();
  await expect(page.locator('.chat-file, .chat-attachment')).toHaveCount(0);
  for (const file of message.attachments) expect((await request.get(`/api/files/${file.id}/download`)).status()).toBe(404);
  expect((await request.get(`/api/memos/${message.memo_id}`)).status()).toBe(404);
});
