// Opt-in: the core Invisible AI moment, on-device. A busy screen hides one
// arithmetic question; the user presses Capture and types nothing. The real
// Rust client captures, Apple Vision reads, the on-device model answers with
// the result first, and the chat marks it Solved and celebrates once.
// The 27B model is held to all of that; the 9B model to a basic local answer.
import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const base = 'http://127.0.0.1:3193';
const wallet = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const policy = {mode:'ondevice', local_provider:'mlx', cloud_provider:'grok'};
const models = [
  {repo:'mlx-community/Qwen3.8-27B-4bit', full:true},
  {repo:'mlx-community/Qwen3.5-9B-4bit', full:false},
];
let server, client, dir;
test.use({baseURL: base});
test.setTimeout(600000);

async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, 'exit');
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
  try { await exited; } finally { clearTimeout(timer); }
}
test.afterEach(async () => { await stop(client); await stop(server); if (dir) fs.rmSync(dir, {recursive:true, force:true}); });

for (const {repo, full} of models) test(`Capture with nothing typed: ${repo} ${full ? 'solves the screen’s math problem and celebrates' : 'answers on-device'}`, async ({page}) => {
  test.skip(process.env.RUN_NATIVE_QWEN !== '1' || process.platform !== 'darwin',
    'Run with RUN_NATIVE_QWEN=1 on an Apple Silicon Mac with an MLX Qwen model installed.');
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'invisible-solve-'));
  const env = {...process.env, CAUSEWAYBAY_CONFIG_DIR:dir, CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(dir, 'client'),
    DATA_DIR:path.join(dir, 'data'), BIND_ADDR:'127.0.0.1:3193', CWBBOT_ADMIN:wallet, APP_TOKEN:'', DEMO_MODE:'0',
    SEMANTIC_SEARCH:'off', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:'', XAI_API_KEY:''};
  server = spawn('./target/debug/causewaybay-server', ['serve', '--no-semantic', '--assets', path.resolve('dist')], {env, stdio:'ignore'});
  await expect.poll(async () => (await page.request.get(base + '/api/health').catch(() => null))?.status(), {timeout:60000}).toBe(200);

  // A screen like the user's: tabs, server status, ids, and one question.
  const screen = path.join(dir, 'screen.png');
  const shot = await page.context().newPage();
  await shot.setViewportSize({width:1400, height:900});
  await shot.setContent('<body style="font:18px system-ui;padding:30px"><h2>Invisible AI · Chat</h2><p>Open tabs: Grok - Imagine, Invisible AI dashboard, example/repo</p><p>Local development server 127.0.0.1:3000 is running. Status: 721 · Invisible AI</p><p>Articles: How to Build a Phone Talking App, Apple Vision Pro overview</p><div style="font-size:64px;margin:60px 0">98 - 4 = ?</div><p>ID: b91b89e9 · Settings · More tools</p></body>');
  await shot.screenshot({path:screen});
  await shot.close();

  await page.goto('/');
  await page.getByLabel('Recovery phrase').fill(phrase);
  await page.getByRole('button', {name:'Sign in', exact:true}).click();
  await expect(page.locator('#connection-label')).toHaveText('Connected');
  client = spawn('./target/debug/causewaybay', ['client', '--backend', base, '--owner-wallet', wallet, '--name', 'Solve Mac', '--fixture-image', screen], {env, stdio:'ignore'});
  await expect.poll(async () => (await (await page.request.get(base + '/api/clients')).json()).some?.(c => c.online)).toBeTruthy();
  expect((await page.request.patch(base + '/api/settings', {data:{provider:'mlx', default_ai:policy, local_ai:{local_only:false, apple_enabled:false, mlx_enabled:true}}})).ok()).toBeTruthy();
  expect((await page.request.patch(base + '/api/ai/preferences', {data:policy})).ok()).toBeTruthy();
  expect((await page.request.patch(base + '/api/ai/models', {data:{provider:'mlx', model:repo}})).ok()).toBeTruthy();
  await page.reload();
  await expect(page.locator('#connection-label')).toHaveText('Connected');

  await expect(page.getByRole('textbox').last()).toHaveValue('');
  await page.locator('button.secondary-button[aria-label="Capture"]').click();
  const reply = page.locator('.chat-message.from-ai').last();
  await expect(reply.getByTestId('chat-llm-log')).toContainText(`Local LLM · mlx · ${repo}`, {timeout:540000});
  await expect(reply.getByTestId('chat-usage')).toContainText('No cloud cost');
  if (!full) return;
  // The answer leads with the result instead of describing the screen.
  const answer = (await reply.locator('.answer-content').innerText()).trim();
  expect(answer.split('\n')[0]).toContain('94');
  await expect(reply.getByTestId('chat-solved')).toContainText('98 - 4 = 94');
  await expect(reply).toHaveAttribute('data-celebrate', '1');
  const party = page.locator('.solve-card');
  await expect(party).toBeVisible();
  await expect(party).toContainText('98 - 4 = 94');
  await expect(party.locator('.solve-score')).toHaveText('+1');
  await expect(party).toHaveCount(0, {timeout:6000});
  // A reload shows the badge but does not celebrate again.
  await page.reload();
  await expect(page.locator('.chat-message.from-ai').last().getByTestId('chat-solved')).toBeVisible();
  await expect(page.locator('.chat-message.from-ai').last()).not.toHaveAttribute('data-celebrate', /.*/);
});
