import {openWorkspace} from './workspace-controls.mjs';
// Opt-in: real Rust processes, Apple Vision/Foundation Models, and TextEdit.
// Only the capture image is synthetic. No cloud credentials or demo responses.
import {test, expect} from '@playwright/test';
import {observeEvents} from './events.mjs';
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const exec = promisify(execFile);
const baseURL = 'http://127.0.0.1:3171';
const local = {local_only:true, apple_enabled:true, mlx_enabled:false,
  mlx_url:'http://127.0.0.1:8080/v1', speech_provider:'apple', speech_locale:'en-US',
  mlx_speech_model:'', auto_interval_seconds:2, local_interval_seconds:2, cloud_min_interval_seconds:2};
const policy = {mode:'ondevice', local_provider:'apple', cloud_provider:'grok'};
const clientId = 'native-notes-' + randomUUID();
let root, data, fixture, env, server, client;
test.use({baseURL});
test.setTimeout(180000);

async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, 'exit');
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
  try { await exited; } finally { clearTimeout(timer); }
}
async function startServer(request) {
  server = spawn('./target/debug/causewaybay-server', ['serve', '--no-semantic',
    '--bind', '127.0.0.1:3171', '--data-dir', data, '--assets', path.resolve('dist')],
  {env, stdio:'ignore'});
  await expect.poll(async () => (await request.get('/api/health').catch(() => null))?.status(),
    {timeout:15000}).toBe(200);
}
async function startClient(request) {
  client = spawn('./target/debug/rust-cli', ['client', '--backend', baseURL,
    '--config-dir', path.join(root, 'client'), '--client-id', clientId,
    '--name', 'Native TextEdit test Mac', '--fixture-image', fixture], {env, stdio:'ignore'});
  await expect.poll(async () => (await (await request.get('/api/clients')).json())
    .some(c => c.id === clientId && c.online), {timeout:15000}).toBe(true);
}
async function jsonPatch(request, url, value) {
  const response = await request.patch(url, {data:value});
  expect(response.ok(), await response.text()).toBeTruthy();
}
async function appleScript(source, args = []) {
  return exec('/usr/bin/osascript', ['-e', source, ...args], {timeout:30000});
}
async function textEditPermission() {
  // This suite is explicitly opt-in. Request missing consent through macOS's
  // dialog; never grant/reset/bypass it or assume Settings already lists a host.
  const source = path.join(root, 'permission.c');
  const binary = path.join(root, 'permission');
  await fs.writeFile(source, `#include <Carbon/Carbon.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(void) {
  const char *id = "com.apple.TextEdit";
  AEAddressDesc target = {typeNull, NULL};
  if (AECreateDesc(typeApplicationBundleID, id, strlen(id), &target)) return 2;
  alarm(10);
  OSStatus status = AEDeterminePermissionToAutomateTarget(&target, typeWildCard, typeWildCard, false);
  if (status == errAEEventWouldRequireUserConsent) {
    fprintf(stderr, "Waiting for the macOS TextEdit Automation consent dialog.\\n");
    alarm(60);
    status = AEDeterminePermissionToAutomateTarget(&target, typeWildCard, typeWildCard, true);
  }
  alarm(0);
  AEDisposeDesc(&target);
  printf("%d\\n", (int)status);
  return 0;
}`);
  await exec('/usr/bin/clang', [source, '-framework', 'Carbon', '-o', binary], {timeout:30000});
  return Number((await exec(binary, [], {timeout:75000})).stdout.trim());
}

test.beforeAll(async ({request, browser}) => {
  test.skip(process.env.RUN_NATIVE_NOTES !== '1' || process.platform !== 'darwin',
    'Run make test-native-notes on a Mac with Apple Intelligence enabled.');
  await fs.access('bin/invisible-apple-ai');
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'invisible-native-flow-'));
  data = path.join(root, 'data');
  fixture = path.join(root, 'capture.png');
  env = {...process.env, CAUSEWAYBAY_CONFIG_DIR:path.join(root, 'backend'),
    INVISIBLE_APPLE_WORKER:path.resolve('bin/invisible-apple-ai'),
    CWBBOT_ADMIN:'', APP_TOKEN:'', DEMO_MODE:'0', SEMANTIC_SEARCH:'off',
    XAI_API_KEY:'', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:''};
  const display = await browser.newPage({viewport:{width:1100, height:500}});
  try {
    await display.setContent('<body style="padding:70px;font:42px system-ui;background:white;color:black"><h1>Invisible AI</h1><p>What is 7 + 5?</p></body>');
    await display.screenshot({path:fixture});
  } finally { await display.close(); }
  await startServer(request);
  await jsonPatch(request, '/api/settings', {provider:'apple', model:'system', local_ai:local, default_ai:policy});
  await jsonPatch(request, '/api/ai/preferences', policy);
  await jsonPatch(request, '/api/assistance/preferences', {wiki_enabled:true, wiki_auto_update:true,
    textedit_enabled:true, textedit_settle_ms:500});
  await startClient(request);
});
test.afterAll(async () => {
  await stop(client);
  await stop(server);
  if (root) await fs.rm(root, {recursive:true, force:true});
});

test('browser Capture → real Rust CLI → Vision OCR → Apple AI → Wiki, usage and restart persistence', async ({page, request}, info) => {
  const events = [];
  let memoId;
  await observeEvents(page, (event, id) => { events.push(event); memoId = id; });
  await page.goto('/overview');
  await page.getByRole('combobox', {name:'Capture client', exact:true}).selectOption(clientId);
  await page.getByRole('button', {name:'Text mode', exact:true}).click();
  await page.getByRole('textbox', {name:'Ask a question', exact:true}).fill('Answer the arithmetic question in the captured image.');
  await page.getByRole('button', {name:'Capture', exact:true}).click();
  await expect(page.getByTestId('capture-preview').locator('img')).toBeVisible({timeout:30000});
  await expect.poll(() => events.find(e => e.kind === 'ocr_text')?.text ?? '', {timeout:30000}).toContain('7 + 5');
  await expect(page.locator('.save-status')).toHaveText('Saved to your library', {timeout:120000});
  await expect(page.locator('.answer-content')).toContainText('12');
  const memo = await (await request.get(`/api/memos/${memoId}`)).json();
  expect(memo).toMatchObject({provider:'apple', status:'complete', has_image:true});
  expect(memo.answer).toContain('12');
  expect(await fs.readFile(path.join(data, 'screenshots', `${memoId}.png`))).toEqual(await fs.readFile(fixture));
  const usage = await (await request.get('/api/usage')).json();
  expect(usage.demo).toBe(false);
  expect(usage.providers).toHaveLength(1);
  expect(usage.providers[0]).toMatchObject({provider:'apple', requests:1});
  expect(usage.providers[0].elapsed_ms).toBeGreaterThan(0);
  const denied = await request.post('/api/think', {data:{provider:'grok', prompt:'Local-only must block this'}});
  expect(denied.status()).toBe(400);
  await page.goto('/wiki');
  await page.getByRole('searchbox', {name:'Search Wiki'}).fill('7 + 5');
  await expect(page.locator('.wiki-card')).toHaveCount(1);
  await page.locator('.wiki-card .wiki-tag').first().click();
  await page.getByTestId('wiki-open-document').click();
  await expect(page.locator('.wiki-article')).toContainText('12');
  await expect(page.getByRole('link', {name:'Open original source'})).toHaveAttribute('href', `/memos/${memoId}`);
  await page.screenshot({path:info.outputPath('local-capture-wiki.png'), fullPage:true});
  await stop(client);
  await stop(server);
  await startServer(request);
  expect(await (await request.get('/api/ai/preferences')).json()).toEqual(policy);
  expect((await (await request.get(`/api/memos/${memoId}`)).json()).answer).toContain('12');
  expect((await (await request.get('/api/usage')).json()).providers).toEqual(usage.providers);
  await page.reload();
  await expect(page.locator('.wiki-article')).toContainText('12');
  await startClient(request);
});

test('browser Auto → real Rust CLI → TextEdit → Apple AI → verified note insertion and Stop', async ({page, request}, info) => {
  await exec('/usr/bin/open', ['-a', 'TextEdit'], {timeout:10000});
  const permission = await textEditPermission();
  await info.attach('macOS Automation permission', {body:Buffer.from(String(permission)), contentType:'text/plain'});
  test.skip(permission !== 0, `macOS TextEdit Automation unavailable (${permission}); enable TextEdit for the app running this test. No writing has been verified.`);
  const name = `Invisible-AI-flow-${randomUUID()}.txt`;
  const notePath = path.join(root, name);
  const original = 'What is 7 + 5?';
  await fs.writeFile(notePath, original);
  let room;
  try {
    // LaunchServices grants the sandboxed TextEdit app access to this exact
    // fixture. A dynamic POSIX-file AppleScript argument can fail that handoff.
    await exec('/usr/bin/open', ['-a', 'TextEdit', notePath], {timeout:10000});
    const readNote = async () => (await appleScript('on run argv\n set targetName to item 1 of argv\n tell application "TextEdit" to return text of (first document whose name is targetName)\nend run', [name])).stdout.trim();
    await expect.poll(() => readNote().catch(() => null), {timeout:10000}).toBe(original);
    await page.goto('/');
    await page.getByRole('combobox', {name:'Active capture device', exact:true}).selectOption(clientId);
    await openWorkspace(page); const auto = page.getByTestId('textedit-auto-toggle');
    await expect(auto).toHaveAttribute('aria-pressed', 'false');
    await auto.click();
    await expect(page.locator('.notes-progress')).toContainText('Answer added', {timeout:120000});
    room = await page.getByRole('combobox', {name:'Invisible AI session', exact:true}).inputValue();
    expect(room).toBeTruthy();
    const answered = await readNote();
    expect(answered).toMatch(/^What is 7 \+ 5\?/);
    expect(answered).toContain('12');
    expect(answered).toContain('[End Invisible AI]');
    expect(answered.match(/\[End Invisible AI\]/g)).toHaveLength(1);
    // Repeating identical input below the previous answer is a new request.
    await appleScript('on run argv\n set targetName to item 1 of argv\n set question to item 2 of argv\n tell application "TextEdit"\n set d to first document whose name is targetName\n set text of d to (text of d) & linefeed & question\n end tell\nend run', [name, original]);
    await expect.poll(async () => (await readNote()).match(/\[End Invisible AI\]/g)?.length || 0,
      {timeout:120000}).toBe(2);
    const repeated = await readNote();
    expect(repeated.split('[End Invisible AI]')[1]).toContain('12');
    await expect(page.locator('.notes-progress')).toContainText('Answer added');
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({path:info.outputPath('textedit-auto-answer.png'), fullPage:true});
    await info.attach('Disposable TextEdit note', {body:Buffer.from(repeated), contentType:'text/plain'});
    await auto.click();
    await expect(auto).toHaveAttribute('aria-pressed', 'false');
    await appleScript('on run argv\n set targetName to item 1 of argv\n tell application "TextEdit"\n set d to first document whose name is targetName\n set text of d to (text of d) & linefeed & "What is 9 + 4?"\n end tell\nend run', [name]);
    const stoppedText = await readNote();
    await page.waitForTimeout(4500); // Cross two configured polling intervals.
    expect(await readNote()).toBe(stoppedText);
    await stop(client);
    await startClient(request);
    expect((await (await request.get(`/api/rooms/${room}/notes`)).json()).active).toBe(false);
    await expect(auto).toHaveAttribute('aria-pressed', 'false');
  } finally {
    if (room) await request.delete(`/api/rooms/${room}/notes`).catch(() => {});
    await appleScript('on run argv\n set targetName to item 1 of argv\n tell application "TextEdit" to close (first document whose name is targetName) saving no\nend run', [name]).catch(() => {});
  }
});
