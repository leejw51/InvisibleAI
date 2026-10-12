// Opt-in: the real on-device Qwen through the browser. A non-demo backend,
// the real Rust MLX worker (make mlx-ai) and an installed Qwen3.5 4B answer
// two chat questions; the backend's own records (progress events, usage,
// loaded model) are what is checked, never the browser's word.
import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const baseURL = 'http://127.0.0.1:3173';
const REPO = 'mlx-community/Qwen3.5-4B-4bit';
const local = {local_only:true, apple_enabled:false, mlx_enabled:true, mlx_engine:'builtin',
  mlx_url:'http://127.0.0.1:8080/v1', speech_provider:'auto', speech_locale:'en-US',
  mlx_speech_model:'', auto_interval_seconds:60, local_interval_seconds:5, cloud_min_interval_seconds:60};
const policy = {mode:'ondevice', local_provider:'mlx', cloud_provider:'grok'};
let root, server;
test.use({baseURL});
test.setTimeout(240000);

async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, 'exit');
  child.kill('SIGTERM');
  const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
  try { await exited; } finally { clearTimeout(timer); }
}
async function progressEvents(request, memoId) {
  const text = await (await request.get(`/api/memos/${memoId}/events?after=0`)).text();
  return text.split('\n').filter(l => l.startsWith('data:')).map(l => JSON.parse(l.slice(5)))
    .filter(e => e.kind === 'progress').map(e => JSON.parse(e.text));
}
async function ask(page, request, room, question, expected) {
  const box = page.getByRole('textbox', {name:'Chat message', exact:true});
  await box.fill(question);
  await page.getByRole('button', {name:'Send message'}).click();
  const reply = page.locator('.chat-message.from-ai').last();
  await expect(reply).toContainText(expected, {timeout:120000});
  await expect(reply.getByTestId('chat-llm-log')).toContainText(`Local LLM · mlx · ${REPO}`);
  await expect(reply.getByTestId('chat-usage')).toContainText('No cloud cost');
  let assistant;
  await expect.poll(async () => {
    const sync = await (await request.get(`/api/rooms/${room}/sync?after=0`)).json();
    assistant = sync.messages.filter(m => m.role === 'assistant').at(-1);
    return assistant?.status;
  }, {timeout: 60_000}).toBe('complete');
  expect(assistant.progress).toBeNull();
  expect(assistant.text).toContain(expected);
  expect(assistant.usage.output_tokens).toBeGreaterThan(0);
  return assistant.memo_id;
}

test.beforeAll(async ({request}) => {
  test.skip(process.env.RUN_NATIVE_QWEN !== '1' || process.platform !== 'darwin',
    'Run make test-native-qwen on an Apple Silicon Mac with Qwen3.5 4B installed.');
  await fs.access('bin/invisible-mlx');
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'invisible-native-qwen-'));
  const env = {...process.env, CAUSEWAYBAY_CONFIG_DIR:path.join(root, 'backend'),
    INVISIBLE_MLX_WORKER:path.resolve('bin/invisible-mlx'),
    CWBBOT_ADMIN:'', APP_TOKEN:'', DEMO_MODE:'0', SEMANTIC_SEARCH:'off',
    XAI_API_KEY:'', OPENAI_API_KEY:'', ANTHROPIC_API_KEY:'', GEMINI_API_KEY:''};
  server = spawn('./target/debug/causewaybay-server', ['serve', '--no-semantic',
    '--bind', '127.0.0.1:3173', '--data-dir', path.join(root, 'data'), '--assets', path.resolve('dist')],
  {env, stdio:'ignore'});
  await expect.poll(async () => (await request.get('/api/health').catch(() => null))?.status(), {timeout:15000}).toBe(200);
  const models = await (await request.get('/api/local-ai/models')).json();
  const qwen = models.models.find(m => m.model === REPO);
  test.skip(!qwen.installed, 'Install Qwen3.5 4B first (Backend settings or CausewaybayMaker).');
  expect(models.worker_available).toBe(true);
  for (const [url, value] of [
    ['/api/settings', {provider:'mlx', model:REPO, local_ai:local, default_ai:policy}],
    ['/api/ai/preferences', policy],
  ]) {
    const response = await request.patch(url, {data:value});
    expect(response.ok(), await response.text()).toBeTruthy();
  }
});
test.afterAll(async () => {
  await stop(server);
  if (root) await fs.rm(root, {recursive:true, force:true});
});

test('the backend answers two chat questions with Qwen3.5 4B, loading it once', async ({page, request}) => {
  const setup = await (await request.get('/api/admin/setup/check')).json();
  expect(setup.ok, JSON.stringify(setup)).toBe(true);
  expect(setup.checks.find(c => c.id === 'answer').detail).toMatch(/^answered in/);
  await page.goto('/chat');
  await page.getByRole('button', {name:'New chat', exact:true}).click();
  await expect(page.getByRole('textbox', {name:'Chat message', exact:true})).toBeEnabled();
  const room = (await (await request.get('/api/rooms')).json())[0].roomid;
  const first = await ask(page, request, room, 'What is seven plus five? Reply with the number only.', '12');
  const second = await ask(page, request, room, 'What is two plus two? Reply with the number only.', '4');
  // The backend's records: the model was loaded into memory, then reused.
  const before = await progressEvents(request, first);
  const after = await progressEvents(request, second);
  expect(JSON.stringify(before.concat(after))).toContain('Qwen3.5 4B ready');
  expect(after.some(p => p.detail === 'already in memory'), JSON.stringify(after)).toBe(true);
  const models = await (await request.get('/api/local-ai/models')).json();
  expect(models.loaded_model).toBe('Qwen3.5 4B');
  const usage = await (await request.get('/api/usage')).json();
  expect(usage.demo).toBe(false);
  const mlx = usage.providers.find(p => p.provider === 'mlx');
  expect(mlx.requests).toBeGreaterThanOrEqual(2);
  expect(mlx.output_tokens).toBeGreaterThan(0);
  await page.screenshot({path:`test-results/native-qwen-${test.info().project.name}.png`, fullPage:true});
});
