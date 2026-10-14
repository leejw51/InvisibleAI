// Every coin on every network: the two main buttons, the search box, balances
// read from each chain's own endpoint (all faked here), receive addresses
// derived from the phrase, EVM-only sending, and the public choices kept on
// the backend across a sign-out that wipes this browser.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos, fakeKoios, rlpDecode, unhex, hex, big, confirmSend } from './evm-fixture.mjs';

const base = 'http://127.0.0.1:3126';
// Public BIP-39 vector; never use it for assets.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const privateKey = '0x1ab42cc412b618bdea3a599e3c9bae199ebf030895b039e9db1e30dafb12b727';
const recipient = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';
// The addresses the Causewaybay Wallet's vectors give this phrase at index 0.
const SOLANA = 'HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk';
const CARDANO_TEST = 'addr_test1qq8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7sh927ysx5sftuw0dlft05dz3c7revpf7jx0xnlcjz3g69mqkt5dmn';
const ECASH = 'ecash:qrwzys2q6xq98vwz0kjn6ulu5m6yljr5fyc909kalg';
let server, dir, koios;

test.beforeAll(async ({ request }) => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-multichain-'));
  koios = await fakeKoios({ ada: 12_345_678n });
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: address, BIND_ADDR: '127.0.0.1:3126', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, CAUSEWAYBAY_RPC_CARDANO_PREPROD: koios.url, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); koios?.close(); });

async function signIn(page, { key = false } = {}) {
  await page.goto(base);
  if (key) {
    await page.getByRole('button', { name: 'Private key', exact: true }).click();
    await page.getByLabel('Private key', { exact: true }).fill(privateKey);
  } else await page.getByLabel('Recovery phrase', { exact: true }).fill(phrase);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Wallet', exact: true })).toBeVisible();
}

test('two main buttons, one search box for every chain, EVM sends, choices kept on the backend', async ({ browser }) => {
  test.setTimeout(120000);
  const context = await browser.newContext({ viewport: { width: 1360, height: 1000 }, reducedMotion: 'no-preference' });
  const rpc = {
    native: 4_250_000_000_000_000_000n, usdc: 0n, mainnetUsdc: 52_500_000n,
    nativeOf: { 'arbitrum-sepolia': 2n * 10n ** 17n }, arbUsdc: { 'arbitrum-sepolia': 9_000_000n },
    // Arbitrum charges about 0.01 gwei; Cronos thousands.
    gasPriceOf: { 'arbitrum-sepolia': 10_000_000n },
    sol: 1_500_000_000n, xec: 300_000n, night: 2_500_000n,
  };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  const bodies = [];
  page.on('request', r => { if (r.postData()) bodies.push({ url: r.url(), body: r.postData() }); });
  await signIn(page);
  await page.getByRole('button', { name: 'Wallet', exact: true }).click();
  const symbol = page.getByTestId('asset-symbol'), network = page.getByTestId('asset-network'), held = page.getByTestId('asset-balance');

  await test.step('opens on Cronos testnet; USDC is one click away, with CRO for fees', async () => {
    await expect(symbol).toHaveText('TCRO');
    await expect(network).toHaveText('Cronos Testnet');
    await page.getByTestId('main-usdc-cronos-mainnet').click();
    await expect(symbol).toHaveText('USDC');
    await expect(network).toHaveText('Cronos Mainnet');
    await expect(page.getByTestId('main-usdc-cronos-mainnet')).toHaveAttribute('aria-pressed', 'true');
    await expect(held).toHaveText('52.5');
    await expect(page.getByTestId('native-balance')).toHaveText('4.25 CRO');
    await expect(page.getByTestId('asset-contract')).toHaveText('0xc21223249CA28397B4B6541dfFaEcC539BfF0c59');
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-shown-balance', '52.50', { timeout: 10000 });
    await page.screenshot({ path: 'test-results/multichain-1-usdc.png' });
  });

  await test.step('CRONOS MAINNET is the network coin', async () => {
    await page.getByTestId('main-cro-cronos-mainnet').click();
    await expect(symbol).toHaveText('CRO');
    await expect(held).toHaveText('4.25');
    await expect(page.getByTestId('fee-balance')).toContainText('paid from this balance');
  });

  await test.step('search finds every chain; Solana, Cardano, eCash and Midnight read their own nodes', async () => {
    const box = page.getByRole('combobox', { name: 'Search coins and networks' });
    await box.click();
    await expect(page.getByTestId('search-results').getByRole('option')).toHaveCount(27);
    await page.screenshot({ path: 'test-results/multichain-2-search.png' });
    await box.fill('sol devnet');
    await expect(page.getByTestId('search-results').getByRole('option')).toHaveCount(2);
    await box.press('Enter');
    await expect(symbol).toHaveText('SOL');
    await expect(network).toHaveText('Solana Devnet');
    await expect(held).toHaveText('1.5');
    await expect(page.getByRole('tab', { name: 'Send' })).toBeDisabled();
    await expect(page.getByTestId('receive-address')).toHaveText(SOLANA);
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-network', 'solana-devnet');
    await expect(page.getByTestId('address-unverified')).toHaveCount(0);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: 'test-results/multichain-3-solana.png' });

    await box.fill('tada preprod');
    await box.press('Enter');
    await expect(network).toHaveText('Cardano Preprod');
    await expect(held).toHaveText('12.345678');
    await expect(page.getByTestId('receive-address')).toHaveText(CARDANO_TEST);
    // Read by the backend relay, for exactly this public address.
    expect(koios.state.calls.at(-1)).toEqual({ path: '/address_info', address: CARDANO_TEST });

    await box.fill('xec');
    await box.press('Enter');
    await expect(held).toHaveText('3000');
    await expect(page.getByTestId('receive-address')).toHaveText(ECASH);

    await box.fill('night preview');
    await box.press('Enter');
    await expect(held).toHaveText('2.5');
    await expect(page.getByTestId('receive-address')).toHaveText(/^mn_addr_preview1/);
    await page.waitForTimeout(1500);
    await page.screenshot({ path: 'test-results/multichain-4-midnight.png' });
  });

  await test.step('Arbitrum Sepolia USDC sends to its own contract with chain id 421614', async () => {
    const box = page.getByRole('combobox', { name: 'Search coins and networks' });
    await box.fill('usdc arbitrum sepolia');
    await box.press('Enter');
    await expect(network).toHaveText('Arbitrum Sepolia');
    await expect(held).toHaveText('9');
    await expect(page.getByTestId('native-balance')).toHaveText('0.2 ETH');
    await page.getByLabel('Amount', { exact: true }).fill('2.5');
    await expect(page.getByTestId('amount-roll')).toHaveAttribute('data-value', '2.5');
    await page.getByLabel('Recipient address').fill(recipient);
    await page.getByRole('button', { name: 'Review transfer' }).click();
    await expect(page.getByTestId('review-symbol')).toHaveText('USDC');
    await confirmSend(page);
    await expect(page.getByTestId('wallet-activity').first()).toContainText('Confirmed', { timeout: 15000 });
    // Once the stage has finished and paused, the page glides up to the card
    // and the amount flies into it.
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-arrived', '1', { timeout: 15000 });
    await expect(page.getByTestId('wallet-card')).toBeInViewport();
    await expect(page.locator('.wallet-flight')).toHaveCount(1);
    await page.waitForTimeout(500);
    await page.screenshot({ path: 'test-results/multichain-5-sending.png' });
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-sent', '1');
    const sent = rpc.sent.at(-1);
    expect(sent.url).toContain('sepolia-rollup.arbitrum.io');
    const [, , , to, value, data, v] = rlpDecode(unhex(sent.raw));
    expect(hex(to)).toBe('0x75faf114eafb1bdbe2f0316df893fd58ce46aa4d');
    expect(value.length).toBe(0);
    expect(hex(data).slice(-64)).toBe((2_500_000n).toString(16).padStart(64, '0'));
    expect([421614n * 2n + 35n, 421614n * 2n + 36n]).toContain(big(v));
  });

  await test.step('public choices survive a sign-out that wipes the browser; keys never left it', async () => {
    const kept = await (await page.request.get(base + '/api/wallet')).json();
    expect(kept.active).toBe('usdc-arbitrum-sepolia');
    expect(kept.recent.slice(0, 3)).toEqual(['usdc-arbitrum-sepolia', 'night-midnight-preview', 'xec-ecash-mainnet']);
    expect(kept.addresses['solana-mainnet']).toBe(SOLANA);
    expect(kept.addresses['ecash-mainnet']).toBe(ECASH);
    for (const { url, body } of bodies) {
      expect(body, url).not.toContain('abandon abandon');
      expect(body, url).not.toContain(privateKey.slice(2));
    }
    // This browser keeps both secrets, as asked: the key and the phrase.
    const stored = JSON.parse(await page.evaluate(() => localStorage.getItem('invisible-ai.wallet.key')));
    expect(stored).toEqual({ key: privateKey, index: 0, mnemonic: phrase });

    await page.getByText('Settings', { exact: true }).click();
    await page.getByRole('button', { name: 'Sign out' }).click();
    await expect(page.getByLabel('Recovery phrase', { exact: true })).toBeVisible();
    await page.evaluate(() => localStorage.clear());
    // Back with only the private key: no phrase here, yet every address is known.
    await signIn(page, { key: true });
    await page.getByRole('button', { name: 'Wallet', exact: true }).click();
    await expect(symbol).toHaveText('USDC');
    await expect(network).toHaveText('Arbitrum Sepolia');
    const box = page.getByRole('combobox', { name: 'Search coins and networks' });
    await box.fill('sol mainnet');
    await box.press('Enter');
    await expect(page.getByTestId('receive-address')).toHaveText(SOLANA);
    // Recorded, not re-derived here: said so plainly.
    await expect(page.getByTestId('address-unverified')).toBeVisible();
  });
  expect(errors).toEqual([]);
  await context.close();
});
