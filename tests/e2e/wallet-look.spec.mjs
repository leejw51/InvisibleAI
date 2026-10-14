// Screenshots for review: the wallet on a phone, in dark mode, with the search open.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos } from './evm-fixture.mjs';

const base = 'http://127.0.0.1:3127';
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
let server;
test.beforeAll(async ({ request }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-look-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: '0x9858effd232b4033e47d90003d41ec34ecaeda94', BIND_ADDR: '127.0.0.1:3127', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

for (const [name, options] of [['phone', { viewport: { width: 1360, height: 1000 } }], ['dark', { viewport: { width: 1360, height: 1000 }, colorScheme: 'dark' }]]) {
  test(`wallet looks right: ${name}`, async ({ browser }) => {
    const context = await browser.newContext({ ...options, reducedMotion: 'no-preference' });
    await fakeCronos(context, { native: 4_250_000_000_000_000_000n, usdc: 0n, mainnetUsdc: 1_284_560_000n, sol: 1_500_000_000n });
    const page = await context.newPage();
    if (name === 'dark') await page.addInitScript(() => localStorage.setItem('causewaybay.theme.v1', 'dark'));
    await page.goto(base);
    await page.getByLabel('Recovery phrase', { exact: true }).fill(phrase);
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: 'Wallet', exact: true }).click();
    await page.getByTestId('main-usdc-cronos-mainnet').click();
    await expect(page.getByTestId('asset-balance')).toHaveText('1284.56');
    if (name === 'phone') await page.setViewportSize({ width: 390, height: 844 });
    await page.waitForTimeout(1600);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: `test-results/look-${name}.png`, fullPage: name === 'phone' });
    await page.getByRole('combobox', { name: 'Search coins and networks' }).fill('sol');
    await page.waitForTimeout(900);
    await page.screenshot({ path: `test-results/look-${name}-search.png` });
    await context.close();
  });
}
