// The send stage and the safety around it: coins ranked by use, Cronos
// testnet by default, amounts typed with their denomination, a reminder of
// recipient and amount, a typed second confirmation above 20 CRO or 1 USD,
// and the game-like stage that shows both wallets before, during and after.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos, rlpDecode, unhex, big } from './evm-fixture.mjs';

const base = 'http://127.0.0.1:3128';
// Public BIP-39 vector; never use it for assets.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const recipient = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';
const CRO = 10n ** 18n;
let server;
test.beforeAll(async ({ request }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-stage-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: address, BIND_ADDR: '127.0.0.1:3128', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

test('Cronos testnet by default, coins ranked by use, a typed second confirmation, and the send stage', async ({ browser }) => {
  test.setTimeout(120000);
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'no-preference' });
  const rpc = { native: 30n * CRO, usdc: 0n, gasPrice: 5_000_000_000_000n, nativeOfAddress: { [recipient.toLowerCase()]: 2n * CRO } };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(base);
  await page.getByLabel('Recovery phrase', { exact: true }).fill(phrase);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  // Sign out is in the main navigation now, not folded under Settings.
  await expect(page.locator('nav[aria-label="Main navigation"]').getByRole('button', { name: 'Sign out' })).toBeVisible();
  await page.getByRole('button', { name: 'Wallet', exact: true }).click();
  const symbol = page.getByTestId('asset-symbol');

  await test.step('a new account opens on Cronos testnet', async () => {
    await expect(symbol).toHaveText('TCRO');
    await expect(page.getByTestId('asset-network')).toHaveText('Cronos Testnet');
    await expect(page.getByTestId('asset-balance')).toHaveText('30');
  });

  await test.step('the list is ranked by use, most used first', async () => {
    const box = page.getByRole('combobox', { name: 'Search coins and networks' });
    for (const query of ['sol devnet', 'xec', 'sol devnet', 'tcro']) { await box.fill(query); await box.press('Enter'); }
    const kept = await (await page.request.get(base + '/api/wallet')).json();
    expect(kept.uses['sol-solana-devnet']).toBe(2);
    await box.press('ArrowDown');
    const options = page.getByTestId('search-results').getByRole('option');
    await expect(options.nth(0)).toHaveAttribute('data-key', 'sol-solana-devnet');
    await box.press('Escape');
    await expect(symbol).toHaveText('TCRO');
  });

  const amount = page.getByLabel('Amount', { exact: true });
  const review = page.getByRole('button', { name: 'Review transfer' });
  const stage = page.getByTestId('send-stage');

  await test.step('an amount in another denomination is refused', async () => {
    await page.getByLabel('Recipient address').fill(recipient);
    await amount.fill('5 USDC');
    await review.click();
    await expect(page.getByTestId('wallet-page').getByRole('alert').first()).toContainText('That amount is in USDC, but this sends TCRO.');
  });

  await test.step('5 TCRO: reminder, no typed confirmation, then the stage plays to the chain\'s final numbers', async () => {
    await amount.fill('5 TCRO');
    await expect(page.getByTestId('amount-roll')).toHaveAttribute('data-value', '5');
    await review.click();
    const reminder = page.getByTestId('send-reminder');
    await expect(reminder).toContainText('5 TCRO');
    await expect(reminder).toContainText(recipient.slice(0, 6));
    await expect(reminder).toContainText('Cronos Testnet');
    await expect(page.getByTestId('double-confirm')).toHaveCount(0);
    // Before: both wallets and their balances.
    await expect(stage).toHaveAttribute('data-phase', 'ready');
    await expect(stage).toHaveAttribute('data-from-before', '30');
    await expect(stage).toHaveAttribute('data-to-before', '2');
    await expect(page.getByTestId('stage-from-balance')).toHaveText('30.00');
    await expect(page.getByTestId('stage-to-balance')).toHaveText('2.00');
    await expect(stage).toHaveAttribute('data-renderer', /webgl|2d/);
    await page.waitForTimeout(900);
    await stage.screenshot({ path: 'test-results/stage-1-ready.png' });
    // The chain moves the money when the receipt comes back.
    rpc.receiptAfter = 2;
    rpc.onReceipt = () => { rpc.native = 24_895_000_000_000_000_000n; rpc.nativeOfAddress[recipient.toLowerCase()] = 7n * CRO; };
    await page.getByTestId('wallet-confirm').click();
    await expect(stage).toHaveAttribute('data-phase', /flying|pending/);
    await page.waitForTimeout(1300);
    await stage.screenshot({ path: 'test-results/stage-2-flying.png' });
    // After: the balances the chain reports, not the ones expected.
    await expect(stage).toHaveAttribute('data-phase', 'done', { timeout: 15000 });
    await expect(stage).toHaveAttribute('data-from-after', '24.895');
    await expect(stage).toHaveAttribute('data-to-after', '7');
    await expect(stage).toHaveAttribute('data-shown-to', '7.00', { timeout: 5000 });
    await expect(page.getByTestId('stage-status')).toHaveText('Arrived!');
    await page.waitForTimeout(400);
    await stage.screenshot({ path: 'test-results/stage-3-done.png' });
    const [, , , , value] = rlpDecode(unhex(rpc.sent[0].raw));
    expect(big(value)).toBe(5n * CRO);
    await page.screenshot({ path: 'test-results/stage-4-page.png' });
    await page.getByTestId('new-transfer').click();
    await expect(stage).toHaveCount(0);
  });

  await test.step('21 TCRO (over 20 CRO) must be typed with its denomination to send', async () => {
    rpc.native = 30n * CRO;
    await page.getByRole('button', { name: 'Refresh balances' }).click();
    await expect(page.getByTestId('asset-balance')).toHaveText('30');
    await page.getByLabel('Recipient address').fill(recipient);
    await amount.fill('21');
    await review.click();
    const typed = page.getByLabel('Type the amount and coin to confirm');
    const confirm = page.getByTestId('wallet-confirm');
    await expect(page.getByTestId('double-confirm')).toContainText('21 TCRO');
    await expect(confirm).toBeDisabled();
    for (const wrong of ['21', 'TCRO', '2.1 TCRO', '21 CRO']) { await typed.fill(wrong); await expect(confirm).toBeDisabled(); }
    await typed.fill('21 tcro');
    await expect(confirm).toBeEnabled();
    await page.screenshot({ path: 'test-results/stage-5-double-confirm.png' });
    rpc.onReceipt = undefined;
    await confirm.click();
    await expect(stage).toHaveAttribute('data-phase', 'done', { timeout: 15000 });
    expect(rpc.sent).toHaveLength(2);
    expect(big(rlpDecode(unhex(rpc.sent[1].raw))[4])).toBe(21n * CRO);
  });

  await test.step('sends count toward the ranking', async () => {
    const kept = await (await page.request.get(base + '/api/wallet')).json();
    // Chosen twice at least (default open + the search back) and sent twice.
    expect(kept.uses['tcro-cronos-testnet']).toBeGreaterThanOrEqual(3);
  });
  expect(errors).toEqual([]);
  await context.close();
});
