import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos, rlpDecode, unhex, hex, big, RPC } from './evm-fixture.mjs';

const base = 'http://127.0.0.1:3124';
// Public BIP-39 / Hardhat test vectors; never use these wallets for assets.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const privateKey = '0x1ab42cc412b618bdea3a599e3c9bae199ebf030895b039e9db1e30dafb12b727';
const recipient = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';
const testnetUsdc = '0xc01efaaf7c5c61bebfaeb358e1161b537b8bc0e0';
const mainnetUsdc = '0xc21223249ca28397b4b6541dffaecc539bff0c59';
const KEY = 'invisible-ai.wallet.key';
/** The saved wallet as the page stored it: `{ key, index }`, or null. */
const saved = page => page.evaluate(k => JSON.parse(localStorage.getItem(k) ?? 'null'), KEY);
let server, dir;

test.beforeAll(async ({ request }) => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-harness-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: address, BIND_ADDR: '127.0.0.1:3124', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

/** Everything a page could leak a secret through: markup, text, and request bodies. */
function watch(page) {
  const bodies = [];
  page.on('request', r => { if (r.postData()) bodies.push({ url: r.url(), body: r.postData() }); });
  return bodies;
}
async function expectNoSecretOnScreen(page, ...secrets) {
  const html = await page.evaluate(() => document.documentElement.outerHTML + document.body.innerText + [...document.querySelectorAll('input')].map(i => i.value).join(' '));
  for (const secret of secrets) {
    expect(html).not.toContain(secret);
    expect(html.toLowerCase()).not.toContain(secret.replace(/^0x/, '').toLowerCase());
  }
}
function expectNoSecretSent(bodies, ...secrets) {
  for (const { url, body } of bodies) {
    for (const secret of secrets) {
      expect(body, url).not.toContain(secret.replace(/^0x/, ''));
      expect(body, url).not.toContain(secret.split(' ').slice(0, 3).join(' '));
    }
  }
}
async function signIn(page, { method = 'phrase', secret = phrase } = {}) {
  await page.goto(base);
  if (method === 'key') {
    await page.getByRole('button', { name: 'Private key', exact: true }).click();
    await page.getByLabel('Private key', { exact: true }).fill(secret);
  } else {
    await page.getByLabel('Recovery phrase', { exact: true }).fill(secret);
  }
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect.poll(async () => (await (await page.request.get(base + '/api/auth/info')).json()).authenticated).toBe(true);
  await expect(page.getByRole('button', { name: 'Wallet', exact: true })).toBeVisible();
}
async function openWallet(page) {
  await page.getByRole('button', { name: 'Wallet', exact: true }).click();
  await expect(page.getByTestId('wallet-page')).toBeVisible();
}

test('phrase login saves only the private key; the wallet shows balances and sends USDC on testnet', async ({ browser }) => {
  const context = await browser.newContext();
  const rpc = { native: 5n * 10n ** 18n, usdc: 25_500_000n };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  const bodies = watch(page);

  await signIn(page);
  // Local storage holds the derived key, and never the phrase.
  const stored = await page.evaluate(() => JSON.stringify(localStorage));
  expect(await saved(page)).toEqual({ key: privateKey, index: 0 });
  expect(stored).not.toContain('abandon');

  await openWallet(page);
  await expect(page.getByTestId('usdc-balance')).toHaveText('25.5');
  await expect(page.getByTestId('native-balance')).toHaveText('5');
  const card = page.getByTestId('wallet-card');
  await expect(card).toHaveAttribute('data-network', 'cronos-testnet');
  await expect(card).toHaveAttribute('data-renderer', /^(webgl|2d)$/);
  await expect(card).toHaveAttribute('data-shown-usdc', '25.50');
  await expect(card).toHaveAttribute('aria-label', /devUSDC\.e/);
  expect(rpc.calls.every(c => c.url.startsWith('https://evm-t3.cronos.org'))).toBe(true);

  // Refusals happen before anything is signed.
  const to = page.getByLabel('Recipient address'), amount = page.getByLabel('Amount', { exact: true });
  const review = page.getByRole('button', { name: 'Review transfer' });
  await to.fill(address); await amount.fill('1');
  await review.click();
  await expect(page.getByTestId('wallet-page').getByRole('alert')).toContainText('your own address');
  await to.fill('0xF39Fd6e51aad88F6F4ce6aB8827279cffFb92266'); // one character's case flipped
  await review.click();
  await expect(page.getByTestId('wallet-page').getByRole('alert')).toContainText('checksum');
  await to.fill(recipient); await amount.fill('30');
  await review.click();
  await expect(page.getByTestId('wallet-page').getByRole('alert')).toContainText('You have 25.5');
  await amount.fill('1.0000001');
  await review.click();
  await expect(page.getByTestId('wallet-page').getByRole('alert')).toContainText('at most 6 decimal places');
  expect(rpc.sent).toHaveLength(0);

  // Review, then cancel: the panel fades out before it leaves the DOM.
  await amount.fill('1.25');
  await review.click();
  const panel = page.getByTestId('wallet-review');
  await expect(panel).toContainText('1.25');
  await expect(panel).toContainText(recipient);
  await expect(panel).toContainText('Cronos Testnet');
  await expect(panel).toContainText('0.3125 TCRO'); // 5000 gwei × (50000 + 25 %)
  await panel.getByRole('button', { name: 'Cancel' }).click();
  await expect(panel).toHaveClass(/is-leaving/);
  await expect(panel).toHaveCount(0);

  // Max fills the whole balance; then send 1.25 for real.
  await page.getByRole('button', { name: 'Max' }).click();
  await expect(amount).toHaveValue('25.5');
  await amount.fill('1.25');
  await review.click();
  await page.getByTestId('wallet-confirm').click();
  await expect(page.getByTestId('wallet-activity')).toContainText('Confirmed');
  await expect(page.getByTestId('wallet-activity').getByRole('link', { name: /Explorer/ })).toHaveAttribute('href', /^https:\/\/explorer\.cronos\.org\/testnet\/tx\/0x[0-9a-f]{64}$/);

  // The signed transaction is exactly an ERC-20 transfer to the USDC contract.
  expect(rpc.sent).toHaveLength(1);
  const [nonce, gasPrice, gasLimit, toField, value, data, v, r, s] = rlpDecode(unhex(rpc.sent[0].raw));
  expect(rpc.sent[0].url).toMatch(/evm-t3/);
  expect(big(nonce)).toBe(7n);
  expect(big(gasPrice)).toBe(5_000_000_000_000n);
  expect(big(gasLimit)).toBe(62_500n);
  expect(hex(toField)).toBe(testnetUsdc);
  expect(value.length).toBe(0);
  expect(hex(data)).toBe('0xa9059cbb' + recipient.slice(2).toLowerCase().padStart(64, '0') + (1_250_000n).toString(16).padStart(64, '0'));
  expect([711n, 712n]).toContain(big(v)); // EIP-155: recovery + 338 × 2 + 35
  expect(r.length).toBeGreaterThan(0); expect(s.length).toBeGreaterThan(0);

  await expectNoSecretOnScreen(page, phrase, privateKey);
  expectNoSecretSent(bodies, phrase, privateKey);
  expect(errors).toEqual([]);
  await context.close();
});

test('mainnet is a separate card: USDC contract, chain 25 and a real-funds note', async ({ browser }) => {
  const context = await browser.newContext();
  const rpc = { native: 3n * 10n ** 18n, usdc: 0n, mainnetUsdc: 1_000_000_000n };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  await signIn(page);
  await openWallet(page);
  await page.getByRole('tab', { name: 'Mainnet' }).click();
  const card = page.getByTestId('wallet-card');
  await expect(card).toHaveAttribute('data-network', 'cronos-mainnet');
  await expect(page.getByTestId('usdc-balance')).toHaveText('1000');
  await expect(card).toHaveAttribute('aria-label', /Cronos Mainnet: 1000 USDC/);
  await expect(page.getByRole('note')).toContainText('real USDC');
  const call = rpc.calls.findLast(c => c.method === 'eth_call');
  expect(call.url).toMatch(/^https:\/\/evm\.cronos\.org/);
  expect(call.params[0].to.toLowerCase()).toBe(mainnetUsdc);

  await page.getByLabel('Recipient address').fill(recipient);
  await page.getByLabel('Amount', { exact: true }).fill('2');
  await page.getByRole('button', { name: 'Review transfer' }).click();
  await page.getByTestId('wallet-confirm').click();
  await expect(page.getByTestId('wallet-activity')).toContainText('Confirmed');
  const [, , , toField, , , v] = rlpDecode(unhex(rpc.sent[0].raw));
  expect(hex(toField)).toBe(mainnetUsdc);
  expect([85n, 86n]).toContain(big(v)); // 25 × 2 + 35
  // The choice of network is remembered; the next visit opens on mainnet.
  await page.reload();
  await openWallet(page);
  await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-network', 'cronos-mainnet');
  await context.close();
});

test('a hostile gas price is refused before signing', async ({ browser }) => {
  const context = await browser.newContext();
  const rpc = { native: 1000n * 10n ** 18n, usdc: 10_000_000n, gasPrice: 10n ** 15n };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  await signIn(page);
  await openWallet(page);
  await page.getByLabel('Recipient address').fill(recipient);
  await page.getByLabel('Amount', { exact: true }).fill('1');
  await page.getByRole('button', { name: 'Review transfer' }).click();
  await expect(page.getByTestId('wallet-page').getByRole('alert')).toContainText('ceiling. Not signing.');
  await expect(page.getByTestId('wallet-review')).toHaveCount(0);
  expect(rpc.sent).toHaveLength(0);
  await context.close();
});

test('private key login, saved wallet on return, and copy-only secrets', async ({ browser }) => {
  const context = await browser.newContext({ permissions: ['clipboard-read', 'clipboard-write'] });
  await fakeCronos(context, { native: 10n ** 18n, usdc: 0n });
  const page = await context.newPage();
  const bodies = watch(page);
  page.on('dialog', d => d.accept());

  // Malformed keys are refused locally.
  await page.goto(base);
  await page.getByRole('button', { name: 'Private key', exact: true }).click();
  await page.getByLabel('Private key', { exact: true }).fill('0x1234');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.locator('.login-card').getByRole('alert')).toContainText('valid private key');
  await expect(page.getByLabel('Private key', { exact: true })).toHaveAttribute('type', 'password');

  await signIn(page, { method: 'key', secret: privateKey.slice(2) });
  expect((await (await page.request.get(base + '/api/auth/info')).json()).address).toBe(address);
  expect(await saved(page)).toEqual({ key: privateKey, index: 0 });

  // Account details: no reveal, only Copy.
  await page.getByText('Settings', { exact: true }).click();
  await page.getByRole('button', { name: 'Account details' }).click();
  await expect(page.getByRole('button', { name: /Show recovery/ })).toHaveCount(0);
  await page.getByTestId('copy-private-key').click();
  await expect(page.getByText('Private key copied')).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(privateKey);
  await expectNoSecretOnScreen(page, privateKey);
  await page.getByRole('button', { name: 'Close account details' }).click();

  // The backend session ends but the key stays: one click signs back in.
  await context.clearCookies();
  await page.reload();
  const savedCard = page.getByTestId('saved-wallet');
  await expect(savedCard).toContainText(address);
  await savedCard.getByRole('button', { name: 'Continue with saved wallet' }).click();
  await expect(page.getByRole('button', { name: 'Wallet', exact: true })).toBeVisible();

  // Signing out forgets the key.
  await page.getByText('Settings', { exact: true }).click();
  await page.getByRole('button', { name: 'Sign out' }).click();
  await expect(page.getByLabel('Recovery phrase', { exact: true })).toBeVisible();
  expect(await saved(page)).toBeNull();

  // A new recovery phrase stays hidden unless Show is pressed.
  await context.clearCookies();
  await page.evaluate(k => localStorage.removeItem(k), KEY);
  await page.goto(base);
  await page.getByRole('button', { name: 'Create new recovery phrase' }).click();
  await expect(page.getByTestId('recovery-words')).toHaveCount(0);
  const signIn2 = page.getByRole('button', { name: 'Sign in', exact: true });
  await expect(signIn2).toBeDisabled();
  await page.getByTestId('copy-recovery-phrase').click();
  await expect(page.getByText('Recovery phrase copied')).toBeVisible();
  const generated = await page.evaluate(() => navigator.clipboard.readText());
  expect(generated.split(' ')).toHaveLength(12);
  await expectNoSecretOnScreen(page, generated);
  await page.getByLabel('I saved my recovery phrase and wallet index.').check();
  await signIn2.click();
  await expect.poll(async () => (await (await page.request.get(base + '/api/auth/info')).json()).authenticated).toBe(true);
  const newKey = (await saved(page)).key;
  expect(newKey).toMatch(/^0x[0-9a-f]{64}$/);
  expect(await page.evaluate(() => JSON.stringify(localStorage))).not.toContain(generated);
  await expectNoSecretOnScreen(page, generated, newKey);

  expectNoSecretSent(bodies, privateKey, generated, newKey);
  expect(bodies.filter(b => RPC.test(b.url)).every(b => !b.body.includes('eth_sendRawTransaction'))).toBe(true);
  await context.close();
});

test('a saved wallet signs back in at the index it was derived at', async ({ browser }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(base);
  await page.getByLabel('Recovery phrase', { exact: true }).fill('test test test test test test test test test test test junk');
  await page.getByLabel('Wallet index', { exact: true }).fill('2');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect.poll(async () => (await (await page.request.get(base + '/api/auth/info')).json()).wallet_index).toBe(2);
  // Hardhat account 2 (public test vector).
  expect(await saved(page)).toEqual({ key: '0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a', index: 2 });
  await context.clearCookies();
  await page.reload();
  await page.getByRole('button', { name: 'Continue with saved wallet' }).click();
  await expect.poll(async () => (await (await page.request.get(base + '/api/auth/info')).json()).authenticated).toBe(true);
  const info = await (await page.request.get(base + '/api/auth/info')).json();
  expect(info.address).toBe('0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc');
  expect(info.wallet_index).toBe(2);
  await context.close();
});

test('on a plain-HTTP host (no Clipboard API) Copy still works and leaves nothing in the DOM', async ({ browser }) => {
  const context = await browser.newContext();
  // LAN/Tailscale hosts are insecure origins, where navigator.clipboard is undefined.
  await context.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, 'clipboard', { get: () => undefined, configurable: true });
    const copies = [];
    window.__copies = copies;
    document.addEventListener('copy', () => {
      const field = document.activeElement;
      copies.push({ tag: field?.tagName, hidden: field?.getAttribute('aria-hidden'), length: field?.value?.length ?? 0 });
    }, true);
  });
  const page = await context.newPage();
  page.on('dialog', d => d.accept());
  await signIn(page);
  expect(await page.evaluate(() => navigator.clipboard)).toBeUndefined();
  await page.getByText('Settings', { exact: true }).click();
  await page.getByRole('button', { name: 'Account details' }).click();
  await page.getByTestId('copy-private-key').click();
  await expect(page.getByText('Private key copied')).toBeVisible();
  // The copy came from the hidden fallback field, holding exactly the key…
  expect(await page.evaluate(() => window.__copies)).toEqual([{ tag: 'TEXTAREA', hidden: 'true', length: privateKey.length }]);
  // …and that field is gone, its value wiped, the moment the copy is done.
  expect(await page.locator('textarea[aria-hidden="true"]').count()).toBe(0);
  await expectNoSecretOnScreen(page, privateKey, phrase);
  await context.close();
});

test('the wallet card renders in three.js with motion: screenshots', async ({ browser }) => {
  const context = await browser.newContext({ reducedMotion: 'no-preference', viewport: { width: 1360, height: 900 } });
  await fakeCronos(context, { native: 4_250_000_000_000_000_000n, usdc: 1_284_560_000n, mainnetUsdc: 52_000_000n });
  const page = await context.newPage();
  await signIn(page);
  await openWallet(page);
  const card = page.getByTestId('wallet-card');
  await expect(card).toHaveAttribute('data-shown-usdc', '1,284.56', { timeout: 10000 });
  await page.mouse.move(420, 380);
  await page.waitForTimeout(900);
  await page.screenshot({ path: 'test-results/wallet-testnet.png' });
  await page.getByRole('tab', { name: 'Mainnet' }).click();
  await expect(card).toHaveAttribute('data-shown-usdc', '52.00', { timeout: 10000 });
  await page.waitForTimeout(1400);
  await page.screenshot({ path: 'test-results/wallet-mainnet.png' });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(600);
  await page.screenshot({ path: 'test-results/wallet-phone.png', fullPage: true });
  await context.close();
});
