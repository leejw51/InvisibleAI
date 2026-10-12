import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import {DatabaseSync} from 'node:sqlite';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Public test wallet, synthetic pixels, isolated backend and real Rust transport.
const wallet = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const local = 'http://127.0.0.1:3132';
const addresses = Object.values(os.networkInterfaces()).flat().filter(a => a.family === 'IPv4' && !a.internal);
const networkHost = process.env.CWB_E2E_NETWORK_HOST
  || addresses.find(a => /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(a.address))?.address;
const remote = `http://${networkHost || '127.0.0.1'}:3132`;
async function stop(child) {
  if (child && child.exitCode === null && child.signalCode === null) {
    const done = once(child, 'exit'); child.kill('SIGTERM'); await done;
  }
}
async function login(page, base) {
  await page.goto(base);
  await page.getByLabel('Recovery phrase').fill(phrase);
  await page.getByRole('button', {name:'Sign in', exact:true}).click();
  await expect(page.getByRole('combobox', {name:'Chat room'})).not.toHaveValue('');
}
async function decoded(image, width, height) {
  await expect(image).toBeVisible();
  await image.scrollIntoViewIfNeeded();
  if ((await image.getAttribute('src'))?.endsWith('/thumbnail')) {
    await expect(image).toHaveAttribute('loading', 'lazy');
    await expect(image).toHaveAttribute('decoding', 'async');
  }
  await expect.poll(() => image.evaluate(el => [el.complete, el.naturalWidth, el.naturalHeight])).toEqual([true, width, height]);
}

test('network capture shows thumbnails in both browsers; click opens the unchanged full image', async ({page, browser, request}, testInfo) => {
  test.setTimeout(120000);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-image-e2e-'));
  const data = path.join(dir, 'data');
  const env = {...process.env, CWBBOT_ADMIN:wallet, APP_TOKEN:'', DEMO_MODE:'1', SEMANTIC_SEARCH:'off',
    BIND_ADDR:'0.0.0.0:3132', DATA_DIR:data, CAUSEWAYBAY_CONFIG_DIR:dir,
    CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(dir, 'client'),
    XAI_API_KEY:'', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:''};
  let server, client, observerContext, db;
  try {
    // Match a Retina capture's dimensions and multi-megabyte transfer without reading a real screen.
    const fixture = await browser.newPage();
    const png = Buffer.from(await fixture.evaluate(() => {
      const canvas = document.createElement('canvas'); canvas.width = 2624; canvas.height = 1696;
      const ctx = canvas.getContext('2d'); const pixels = ctx.createImageData(canvas.width, canvas.height);
      let seed = 123456;
      for (let i = 0; i < pixels.data.length; i += 4) {
        seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
        const grey = seed >>> 24;
        pixels.data[i] = grey; pixels.data[i+1] = grey; pixels.data[i+2] = grey; pixels.data[i+3] = 255;
      }
      ctx.putImageData(pixels, 0, 0);
      ctx.fillStyle = 'white'; ctx.fillRect(80, 80, 1300, 180);
      ctx.fillStyle = 'black'; ctx.font = '80px sans-serif'; ctx.fillText('Capture test: 7 + 5 = 12', 100, 200);
      return canvas.toDataURL('image/png').split(',')[1];
    }), 'base64');
    await fixture.close();
    const fixturePath = path.join(dir, 'screen.png'); fs.writeFileSync(fixturePath, png);
    expect(png.length).toBeGreaterThan(2_000_000);
    server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {env, stdio:'ignore'});
    await expect.poll(async () => (await request.get(local+'/api/health').catch(() => null))?.status()).toBe(200);
    client = spawn('./target/debug/rust-cli', ['client', '--backend', local, '--owner-wallet', wallet,
      '--client-id', 'thumbnail-node', '--name', 'Synthetic capture Mac', '--fixture-image', fixturePath], {env, stdio:'ignore'});
    const imageRequests = [];
    page.on('request', r => { if(r.resourceType() === 'image' && r.url().includes('/api/')) imageRequests.push(r.url()); });
    const calls = [];
    page.on('websocket', ws => ws.on('framesent', frame => {
      const v = JSON.parse(String(frame.payload)); if(v.path === '/api/think') calls.push(v.body);
    }));
    await login(page, remote);
    if (networkHost) expect(await page.evaluate(() => isSecureContext)).toBe(false);
    observerContext = await browser.newContext();
    const observer = await observerContext.newPage();
    await login(observer, local);
    await expect.poll(async () => (await (await page.request.get(remote+'/api/clients')).json()).some(c => c.id === 'thumbnail-node' && c.online)).toBe(true);
    await page.getByRole('combobox', {name:'Active capture device'}).selectOption('thumbnail-node');
    await page.getByRole('button', {name:'Capture', exact:true}).click();
    await expect.poll(() => calls.length).toBe(1);
    expect(calls[0]).toMatchObject({capture:true, prompt:'', client_id:'thumbnail-node'});
    expect(calls[0].provider).not.toBe('');
    expect(calls[0].model).not.toBe('');
    db = new DatabaseSync(path.join(data, 'causewaybay.sqlite3')); db.exec('PRAGMA busy_timeout=5000;');
    let memo;
    await expect.poll(() => {
      memo = db.prepare('SELECT m.id,m.status FROM memos m JOIN chat_messages c ON c.memo_id=m.id WHERE c.messageid=?').get(calls[0].message_id);
      return memo?.status;
    }).toBe('complete');
    // Demo AI avoids paid calls. Queue the real CLI capture separately, then publish
    // the same image revision that the live pipeline publishes on capture completion.
    const captureId = randomUUID();
    db.exec('BEGIN IMMEDIATE');
    db.prepare("UPDATE memos SET status='running' WHERE id=?").run(memo.id);
    db.prepare("INSERT INTO capture_requests(id,client_id,memo_id,mode,display,created_at,requester) VALUES(?,'thumbnail-node',?,'screen',1,?,?)")
      .run(captureId, memo.id, Math.floor(Date.now()/1000), wallet);
    db.exec('COMMIT');
    await expect.poll(() => db.prepare('SELECT status FROM capture_requests WHERE id=?').get(captureId)?.status).toBe('ready');
    const imagePath = db.prepare('SELECT image_path FROM memos WHERE id=?').get(memo.id).image_path;
    expect(fs.readFileSync(path.join(data, imagePath))).toEqual(png);
    db.exec('BEGIN IMMEDIATE');
    db.prepare("UPDATE memos SET status='complete' WHERE id=?").run(memo.id);
    const message = JSON.parse(db.prepare('SELECT document FROM chat_messages WHERE messageid=?').get(calls[0].message_id).document);
    const last = db.prepare('SELECT value FROM sync_clock WHERE id=1').get().value;
    message.messagesyncid = Math.max(Date.now()*1000, last+1); message.image_path = imagePath;
    const document = JSON.stringify(message);
    db.prepare('UPDATE sync_clock SET value=? WHERE id=1').run(message.messagesyncid);
    db.prepare('UPDATE chat_messages SET document=? WHERE messageid=?').run(document, message.messageid);
    db.prepare('INSERT INTO message_changes(messagesyncid,roomid,messageid,document) VALUES(?,?,?,?)')
      .run(message.messagesyncid, message.roomid, message.messageid, document);
    db.exec('COMMIT');
    const thumbnailPath = `/api/memos/${memo.id}/thumbnail`;
    const fullPath = `/api/memos/${memo.id}/image`;
    await decoded(page.locator('.chat-image img'), 960, 620);
    await decoded(observer.locator('.chat-image img'), 960, 620);
    await expect(page.locator('.chat-image img')).toHaveAttribute('src', thumbnailPath);
    expect(imageRequests).toContain(remote+thumbnailPath);
    expect(imageRequests.some(url => url.endsWith('/image') || url.endsWith('/download'))).toBe(false);
    const preview = await page.request.get(remote+thumbnailPath);
    expect(preview.headers()['content-type']).toBe('image/jpeg');
    const thumbnail = await preview.body(); expect(thumbnail.length).toBeLessThan(png.length/5);
    const opened = page.waitForEvent('popup');
    await page.getByRole('link', {name:'Open full screenshot', exact:true}).click();
    const full = await opened;
    await expect(full).toHaveURL(remote+fullPath);
    await decoded(full.locator('img'), 2624, 1696);
    expect(await (await page.request.get(remote+fullPath)).body()).toEqual(png);
    await full.close();
    // Thumbnails must also load after rehydration from IndexedDB and server replay.
    await page.reload(); await decoded(page.locator('.chat-image img'), 960, 620);
    await page.screenshot({path:testInfo.outputPath('capture-thumbnail.png'), fullPage:true});
    await testInfo.attach('transfer-sizes', {body:JSON.stringify({origin:remote, originalBytes:png.length, thumbnailBytes:thumbnail.length}), contentType:'application/json'});
    // The saved response view also loads only a lightweight preview, not the original.
    const responseImages = [];
    observer.on('request', r => { if (r.resourceType() === 'image' && r.url().includes('/api/')) responseImages.push(r.url()); });
    await observer.goto(`${local}/memos/${memo.id}`);
    await decoded(observer.getByTestId('capture-preview').locator('img'), 960, 620);
    await expect(observer.getByTestId('capture-preview').locator('a')).toHaveAttribute('href', fullPath);
    expect(responseImages).toContain(local+thumbnailPath);
    expect(responseImages.some(url => url.endsWith('/image') || url.endsWith('/download'))).toBe(false);
    // Uploaded photos and the image library use the same bounded delivery path.
    await page.getByLabel('Attach to chat').setInputFiles({name:'photo.png', mimeType:'image/png', buffer:png});
    await decoded(page.locator('.chat-attachment img'), 960, 620);
    await expect(page.locator('.chat-attachment img')).toHaveAttribute('src', /\/thumbnail$/);
    const navigation = page.getByRole('button', {name:'Open navigation', exact:true});
    if (await navigation.isVisible()) await navigation.click();
    if (!(await page.locator('[data-nav="images"]').isVisible())) await page.locator('.more-tools > summary').click();
    await page.getByRole('button', {name:'Images', exact:true}).click();
    const card = page.locator('.image-card').filter({has:page.getByRole('heading', {name:'photo.png', exact:true})});
    await decoded(card.locator('img'), 960, 620);
    await expect(card.locator('img')).toHaveAttribute('src', /\/thumbnail$/);
    await expect(card.locator('a').first()).toHaveAttribute('href', /\/image$/);
    const openedPhoto = page.waitForEvent('popup');
    await card.locator('a').first().click();
    const photo = await openedPhoto;
    await decoded(photo.locator('img'), 2624, 1696);
    await photo.close();
    // Clearing a room removes access to both the original and its derived thumbnail.
    expect((await page.request.post(remote+`/api/rooms/${message.roomid}/clear`)).ok()).toBe(true);
    expect((await page.request.get(remote+thumbnailPath)).status()).toBe(404);
    expect((await page.request.get(remote+fullPath)).status()).toBe(404);
    expect(fs.existsSync(path.join(data, imagePath))).toBe(false);
    expect((await request.get(remote+thumbnailPath)).status()).toBe(401);
  } finally {
    db?.close(); await observerContext?.close(); await page.close();
    await stop(client); await stop(server); fs.rmSync(dir, {recursive:true, force:true});
  }
});
