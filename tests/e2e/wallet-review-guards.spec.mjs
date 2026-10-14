// Guards from the PR review: nothing is signed after a token fails its
// decimals check, a node's simulated revert stops the transfer before signing,
// and a receipt only moves the send stage of its own transaction.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos, keccak256, unhex, hex, confirmSend } from './evm-fixture.mjs';

const base = 'http://127.0.0.1:3129';
// Public BIP-39 vector; never use it for assets.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const recipient = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';
const wcro = '0x5c7f8a570d578ed84e63fdfa7b1ee72deae1ae23';
const CRO = 10n ** 18n;
let server;
test.beforeAll(async ({ request }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-guards-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: address, BIND_ADDR: '127.0.0.1:3129', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

test('a failed decimals check, a simulated revert and a stray receipt each stop short of a wrong send', async ({ browser }) => {
  test.setTimeout(120000);
  const context = await browser.newContext({ viewport: { width: 1360, height: 900 } });
  const rpc = { native: 10n * CRO, usdc: 0n, tokens: { [wcro]: { symbol: 'WCRO', name: 'Wrapped CRO', decimals: 18, balance: 3n * CRO } } };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(base);
  await page.getByLabel('Recovery phrase', { exact: true }).fill(phrase);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('button', { name: 'Wallet', exact: true }).click();
  const symbol = page.getByTestId('asset-symbol');
  const box = page.getByRole('combobox', { name: 'Search coins and networks' });
  const amount = page.getByLabel('Amount', { exact: true });
  const review = page.getByRole('button', { name: 'Review transfer' });
  const alert = page.getByTestId('wallet-page').getByRole('alert').first();
  const stage = page.getByTestId('send-stage');
  await expect(symbol).toHaveText('TCRO');

  await test.step('a token whose decimals change on refresh loses its review and cannot be sent', async () => {
    await box.fill(wcro);
    await page.getByTestId('search-add').getByRole('button', { name: 'Cronos Testnet', exact: true }).click();
    await page.getByTestId('add-token').click();
    await expect(symbol).toHaveText('WCRO');
    await expect(page.getByTestId('asset-balance')).toHaveText('3');
    await page.getByLabel('Recipient address').fill(recipient);
    await amount.fill('0.5');
    await review.click();
    await expect(page.getByTestId('wallet-review')).toBeVisible();
    rpc.tokens[wcro].decimals = 6;
    await page.getByRole('button', { name: 'Refresh balances' }).click();
    await expect(alert).toContainText('reports 6 decimals');
    await expect(page.getByTestId('wallet-review')).toHaveCount(0);
    await expect(review).toBeDisabled();
    expect(rpc.sent).toHaveLength(0);
  });

  await test.step('a revert from eth_estimateGas is shown, and nothing is signed', async () => {
    await box.fill('tcro');
    await box.press('Enter');
    await expect(symbol).toHaveText('TCRO');
    rpc.estimateError = { code: 3, message: 'execution reverted: token transfers are paused' };
    await page.getByLabel('Recipient address').fill(recipient);
    await amount.fill('1');
    await review.click();
    await expect(alert).toContainText('token transfers are paused');
    await expect(page.getByTestId('wallet-review')).toHaveCount(0);
    expect(rpc.sent).toHaveLength(0);
    rpc.estimateError = undefined;
  });

  await test.step('the first transfer\'s receipt leaves the second transfer\'s stage alone', async () => {
    const released = new Set();
    rpc.holdReceipt = hash => !released.has(hash.toLowerCase());
    const hashOf = i => hex(keccak256(unhex(rpc.sent[i].raw))).toLowerCase();
    await page.getByLabel('Recipient address').fill(recipient);
    await amount.fill('1');
    await review.click();
    await confirmSend(page);
    await expect(page.getByTestId('wallet-activity')).toHaveCount(1);
    await page.getByLabel('Recipient address').fill(recipient);
    await amount.fill('2');
    await review.click();
    await confirmSend(page);
    await expect(page.getByTestId('wallet-activity')).toHaveCount(2);
    await expect(stage).toHaveAttribute('data-phase', 'flying');
    // Only the 1 TCRO transfer confirms.
    released.add(hashOf(0));
    await expect(page.getByTestId('wallet-activity').nth(1)).toContainText('Confirmed', { timeout: 10000 });
    await page.waitForTimeout(1500);
    await expect(stage).toHaveAttribute('data-phase', 'flying');
    await expect(page.getByTestId('wallet-activity').nth(0)).toContainText('Pending');
    // Now the 2 TCRO transfer's own receipt ends its stage.
    released.add(hashOf(1));
    await expect(stage).toHaveAttribute('data-phase', 'done', { timeout: 15000 });
  });
  expect(errors).toEqual([]);
  await context.close();
});
