// Live: the Wallet page against the real public Cronos nodes. Opt in with
// RUN_LIVE_WALLET=1 (make test-wallet-live). Read-only: it reads balances and
// asks the node for a real quote, then cancels — it never confirms a transfer.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const base = 'http://127.0.0.1:3126';
// Public BIP-39 vector; never use it for assets.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
let server, dir;

test.beforeAll(async ({ request }) => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-live-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: address, BIND_ADDR: '127.0.0.1:3126', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

test('real nodes on every chain: balances load, and a Cronos testnet transfer is quoted, never sent', async ({ page }) => {
  test.setTimeout(90000);
  const sent = [];
  page.on('request', r => { if (/cronos\.org/.test(r.url()) && r.postData()?.includes('eth_sendRawTransaction')) sent.push(r.url()); });
  await page.goto(base);
  await page.getByLabel('Recovery phrase', { exact: true }).fill(phrase);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('button', { name: 'Wallet', exact: true }).click();

  const wallet = page.getByTestId('wallet-page');
  const box = page.getByRole('combobox', { name: 'Search coins and networks' });
  // One coin per network family, read from the real public node of each chain.
  for (const [query, network] of [
    ['devusdc', 'cronos-testnet'], ['usdc cronos mainnet', 'cronos-mainnet'],
    ['usdc arbitrum sepolia', 'arbitrum-sepolia'], ['eth arbitrum one', 'arbitrum-one'],
    ['sol devnet', 'solana-devnet'], ['usdc solana mainnet', 'solana-mainnet'],
    ['tada preprod', 'cardano-preprod'], ['ada mainnet', 'cardano-mainnet'],
    ['xec mainnet', 'ecash-mainnet'], ['night preview', 'midnight-preview'],
  ]) {
    await box.fill(query);
    await box.press('Enter');
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-network', network);
    // A real number from the real node, and no refusal.
    await expect(page.getByTestId('asset-balance')).toHaveText(/^\d+(\.\d+)?$/, { timeout: 45000 });
    await expect(page.getByTestId('native-balance')).toHaveText(/^\d+(\.\d+)? \S+$/);
    await expect(wallet.getByRole('alert')).toHaveCount(0);
  }

  // On testnet, ask for a quote: either the balance covers it and the node
  // quotes a fee, or the wallet refuses for the balance — both are real paths.
  await box.fill('devusdc');
  await box.press('Enter');
  await expect(page.getByTestId('asset-balance')).toHaveText(/^\d/, { timeout: 30000 });
  await page.getByLabel('Recipient address').fill('0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266');
  await page.getByLabel('Amount', { exact: true }).fill('0.01');
  await page.getByRole('button', { name: 'Review transfer' }).click();
  const review = page.getByTestId('wallet-review'), alert = wallet.getByRole('alert');
  await expect(review.or(alert)).toBeVisible({ timeout: 30000 });
  if (await review.isVisible()) {
    await expect(review).toContainText(/\d+(\.\d+)? TCRO/);
    await review.getByRole('button', { name: 'Cancel' }).click();
  } else {
    await expect(alert).toContainText(/You have|network fee/);
  }
  expect(sent).toEqual([]);
});
