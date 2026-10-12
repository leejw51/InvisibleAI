// On-device model picker: Apple LLM plus the Qwen MLX models, GPU-memory fit
// from the backend, and an explicit download whose progress bar eases toward
// each polled value. A loopback stand-in for Hugging Face serves small files
// slowly, so nothing real is downloaded.
import {isolatedBackend} from './invisible-fixture.mjs';
import {test, expect} from '@playwright/test';
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const REPO = 'mlx-community/Qwen3.5-4B-4bit';
const GIB = 1024 ** 3;
let hub, hubPort, cacheDir;
const files = {
  'config.json': Buffer.from('{"model_type":"qwen3_5"}'),
  'tokenizer.json': Buffer.from('{"version":"1.0"}'),
  'model.safetensors.index.json': Buffer.from('{"weight_map":{"a":"model.safetensors"}}'),
  'model.safetensors': Buffer.alloc(1_200_000, 7),
  'README.md': Buffer.from('skip me'),
};

test.beforeAll(async () => {
  cacheDir = fs.mkdtempSync(path.join(os.tmpdir(), 'empty-hf-cache-'));
  hub = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://x');
    if (url.pathname === `/api/models/${REPO}/tree/main`) {
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify(Object.entries(files).map(([p, b]) => ({type: 'file', path: p, size: b.length}))));
      return;
    }
    const m = url.pathname.match(new RegExp(`^/${REPO}/resolve/main/(.+)$`));
    if (!m || !files[m[1]]) { res.statusCode = 404; res.end(); return; }
    const body = files[m[1]];
    if (m[1] !== 'model.safetensors') { res.end(body); return; }
    // Weights trickle out in twelve slices so the bar visibly grows.
    res.statusCode = 200;
    let i = 0;
    const slice = Math.ceil(body.length / 12);
    const tick = () => {
      if (i >= body.length) { res.end(); return; }
      res.write(body.subarray(i, i + slice));
      i += slice;
      setTimeout(tick, 250);
    };
    tick();
  });
  await new Promise(r => hub.listen(0, '127.0.0.1', r));
  hubPort = hub.address().port;
});
test.afterAll(async () => { hub?.close(); fs.rmSync(cacheDir, {recursive: true, force: true}); });

isolatedBackend(test, 3177, () => ({
  INVISIBLE_HF_BASE: `http://127.0.0.1:${hubPort}`,
  HF_HUB_CACHE: cacheDir,
  INVISIBLE_MLX_WORKER: '/nonexistent/invisible-mlx',
  INVISIBLE_UNIFIED_MEMORY_BYTES: String(16 * GIB),
}));

async function openSettings(page) {
  await page.goto('/overview');
  const settingsButton = page.getByRole('button', {name: 'Open settings', exact: true});
  if (await settingsButton.isVisible()) await settingsButton.click();
  else { await page.getByRole('button', {name: 'Open navigation', exact: true}).click(); await page.keyboard.press('Escape'); await page.getByRole('button', {name: 'Open settings', exact: true}).click(); }
  await expect(page.getByTestId('ondevice-models')).toBeVisible();
}

test('the picker lists Apple LLM and the Qwen models with the backend GPU-memory verdict', async ({page, request}) => {
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  const models = await (await request.get('/api/local-ai/models')).json();
  expect(models.unified_memory_bytes).toBe(16 * GIB);
  expect(models.gpu_budget_bytes).toBe(Math.floor(16 * GIB / 3) * 2);
  expect(models.worker_available).toBe(false);
  expect(models.models.map(m => [m.id, m.installed, m.fit])).toEqual([
    ['apple', true, 'fits'], ['qwen3.5-9b', false, 'fits'], ['qwen3.5-4b', false, 'fits'], ['qwen3.8-27b', false, 'too_large'],
  ]);
  await openSettings(page);
  await expect(page.getByTestId('ondevice-memory')).toContainText('Unified memory 16 GB');
  await expect(page.getByTestId('ondevice-memory')).toContainText('GPU budget 11 GB');
  await expect(page.getByTestId('ondevice-memory')).toContainText('make mlx-ai');
  await expect(page.getByTestId('model-card-apple')).toContainText('Apple LLM');
  await expect(page.getByTestId('model-card-qwen3.8-27b')).toContainText('too large for this Mac');
  await expect(page.getByTestId('model-card-qwen3.8-27b').getByRole('button', {name: 'Download anyway'})).toBeVisible();
  await expect(page.getByTestId('model-card-qwen3.5-9b')).toContainText('fits GPU memory');
  // Choosing a card selects its provider and model for the server default.
  await expect(page.getByRole('checkbox', {name: 'Enable MLX local', exact: true})).not.toBeChecked();
  await page.getByTestId('model-card-qwen3.5-9b').getByRole('radio').click();
  await expect(page.getByRole('checkbox', {name: 'Enable MLX local', exact: true})).toBeChecked();
  await expect(page.getByRole('textbox', {name: 'Model', exact: true})).toHaveValue('mlx-community/Qwen3.5-9B-4bit');
  await expect(page.getByRole('button', {name: /MLX local/})).toHaveClass(/selected/);
  await page.getByTestId('model-card-apple').getByRole('radio').click();
  await expect(page.getByRole('checkbox', {name: 'Enable Apple local', exact: true})).toBeChecked();
  await expect(page.getByRole('textbox', {name: 'Model', exact: true})).toHaveValue('system');
  await expect(page.getByRole('button', {name: /Apple local/})).toHaveClass(/selected/);
  // Saving makes the backend check its own setup and report back: the Qwen 9B
  // is chosen but the worker is not built and the model is not installed.
  await page.getByTestId('model-card-qwen3.5-9b').getByRole('radio').click();
  await page.getByRole('group', {name: 'Default AI processing mode'}).getByRole('button', {name: /^On-device/}).click();
  await page.getByRole('button', {name: 'Save backend settings', exact: true}).click();
  const report = page.getByTestId('setup-report');
  await expect(report.locator('li[data-check="mode"]')).toContainText('On-device · mlx');
  await expect(report.locator('li[data-check="enabled"]')).not.toHaveClass(/problem/);
  await expect(report.locator('li[data-check="worker"]')).toHaveClass(/problem/);
  await expect(report.locator('li[data-check="worker"]')).toContainText('make mlx-ai');
  await expect(report.locator('li[data-check="model"]')).toContainText('Qwen3.5 9B');
  await expect(report.locator('li[data-check="model"]')).toContainText('not installed');
  await expect(report).toContainText('found problems');
  await expect(page.getByRole('status').filter({hasText: 'Backend settings saved'})).toBeVisible();
  expect(errors).toEqual([]);
});

test('an explicit download shows an easing progress bar, then the model is installed', async ({page, request}) => {
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  await openSettings(page);
  const card = page.getByTestId('model-card-qwen3.5-4b');
  await expect(card).toHaveClass(/not-installed/);
  await card.getByRole('button', {name: 'Download', exact: true}).click();
  const bar = card.getByRole('progressbar', {name: 'Downloading Qwen3.5 4B'});
  await expect(bar).toBeVisible();
  await expect(card).toContainText('model.safetensors');
  // Reported values grow; the painted width tweens and never overshoots the report.
  const seen = [];
  await expect.poll(async () => {
    // One atomic read: a pushed snapshot between two reads would compare a new width with an old target.
    const [now, target, width] = await bar.evaluate(el => [Number(el.getAttribute('aria-valuenow')), Number(el.dataset.target), parseFloat(el.querySelector('i').style.width)]);
    seen.push([now, width]);
    expect(width).toBeLessThanOrEqual(target + 0.01);
    return now;
  }, {timeout: 20_000, intervals: [120]}).toBeGreaterThan(60);
  expect(seen.filter(([n]) => n > 0 && n < 100).length).toBeGreaterThan(2);
  for (let i = 1; i < seen.length; i++) expect(seen[i][0]).toBeGreaterThanOrEqual(seen[i - 1][0]);
  // Closing the dialog mid-tween must not fire a freed animation frame; reopening resumes polling.
  await page.getByRole('button', {name: 'Close dialog'}).click();
  await expect(page.getByTestId('ondevice-models')).toHaveCount(0);
  await openSettings(page);
  await expect(card.getByRole('progressbar', {name: 'Downloading Qwen3.5 4B'}).or(card.getByText('Installed'))).toBeVisible();
  await expect(card).toContainText('Installed', {timeout: 20_000});
  await expect(bar).toHaveCount(0);
  const models = await (await request.get('/api/local-ai/models')).json();
  const qwen = models.models.find(m => m.id === 'qwen3.5-4b');
  expect(qwen.installed).toBe(true);
  expect(qwen.download).toMatchObject({finished: true, cancelled: false, error: null, total_files: 4});
  expect(qwen.download.done_bytes).toBe(qwen.download.total_bytes);
  expect(errors).toEqual([]);
  await page.screenshot({path: `test-results/ondevice-models-${test.info().project.name}.png`, fullPage: true});
});
