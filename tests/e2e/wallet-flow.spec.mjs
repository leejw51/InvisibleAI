// The wallet as one user journey, start to finish: a brand-new wallet is
// created and approved, receives USDC, sends some, switches network, signs out
// and comes back with its private key — with secrets checked at every stage.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos, rlpDecode, unhex, hex, big, RPC, confirmSend } from './evm-fixture.mjs';

const base = 'http://127.0.0.1:3125';
// Public BIP-39 vector for the admin; never use it for assets.
const adminPhrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const adminAddress = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const recipient = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';
const KEY = 'invisible-ai.wallet.key';
let server, dir;

test.beforeAll(async ({ request }) => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-flow-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: adminAddress, BIND_ADDR: '127.0.0.1:3125', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

const info = async page => (await page.request.get(base + '/api/auth/info')).json();
const saved = page => page.evaluate(k => JSON.parse(localStorage.getItem(k) ?? 'null'), KEY);
const clipboard = page => page.evaluate(() => navigator.clipboard.readText());
async function noSecretOnScreen(page, secrets) {
  const text = await page.evaluate(() => document.documentElement.outerHTML + document.body.innerText + [...document.querySelectorAll('input,textarea')].map(i => i.value).join(' '));
  for (const secret of secrets) {
    expect(text).not.toContain(secret);
    expect(text.toLowerCase()).not.toContain(secret.replace(/^0x/, '').toLowerCase());
  }
}

test('new wallet → approval → receive → send → network switch → sign out → back in by private key', async ({ browser }) => {
  test.setTimeout(120000);
  const adminContext = await browser.newContext();
  const context = await browser.newContext({ permissions: ['clipboard-read', 'clipboard-write'], viewport: { width: 1360, height: 900 } });
  const rpc = { native: 0n, usdc: 0n, mainnetUsdc: 0n };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('dialog', d => d.accept());
  const bodies = [];
  page.on('request', r => { if (r.postData()) bodies.push({ url: r.url(), body: r.postData() }); });
  let phrase, address, privateKey;

  await test.step('create a new wallet: the phrase stays hidden until shown', async () => {
    await page.goto(base);
    await page.getByRole('button', { name: 'Create new recovery phrase' }).click();
    const signIn = page.getByRole('button', { name: 'Sign in', exact: true });
    await expect(signIn).toBeDisabled();
    await page.getByTestId('copy-recovery-phrase').click();
    await expect(page.getByText('Recovery phrase copied')).toBeVisible();
    phrase = await clipboard(page);
    expect(phrase.split(' ')).toHaveLength(12);
    await noSecretOnScreen(page, [phrase]);
    await page.getByLabel('I saved my recovery phrase and wallet index.').check();
    await signIn.click();
    await expect(page.getByRole('heading', { name: 'You’re on the waiting list.' })).toBeVisible();
    address = (await info(page)).address;
    const stored = await saved(page);
    expect(stored.index).toBe(0);
    expect(stored.key).toMatch(/^0x[0-9a-f]{64}$/);
    privateKey = stored.key;
    // The phrase is kept in this browser (for the non-EVM chains), and only here.
    expect(stored.mnemonic).toBe(phrase);
  });

  await test.step('an admin approves the new wallet', async () => {
    const admin = await adminContext.newPage();
    await admin.goto(base);
    await admin.getByLabel('Recovery phrase', { exact: true }).fill(adminPhrase);
    await admin.getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect.poll(async () => (await info(admin)).is_admin).toBe(true);
    expect((await adminContext.request.patch(`${base}/api/admin/users/${address}`, { data: { role: 'user' } })).status()).toBe(200);
    await expect(page.getByRole('button', { name: 'Wallet', exact: true })).toBeVisible({ timeout: 20000 });
  });

  await test.step('open the wallet on testnet: empty card, receive address copies', async () => {
    await page.getByRole('button', { name: 'Wallet', exact: true }).click();
    const card = page.getByTestId('wallet-card');
    // A new wallet opens on Cronos testnet; search finds the testnet token.
    await expect(card).toHaveAttribute('data-network', 'cronos-testnet');
    await page.getByRole('combobox', { name: 'Search coins and networks' }).fill('devusdc');
    await page.getByRole('combobox', { name: 'Search coins and networks' }).press('Enter');
    await expect(card).toHaveAttribute('data-network', 'cronos-testnet');
    await expect(page.getByTestId('asset-balance')).toHaveText('0');
    await expect(card).toHaveAttribute('data-shown-balance', '0.00');
    await page.getByRole('tab', { name: 'Receive' }).click();
    await expect(page.getByTestId('receive-address')).toHaveText(new RegExp(address, 'i'));
    await page.getByRole('button', { name: 'Copy address' }).click();
    await expect.poll(async () => (await clipboard(page)).toLowerCase()).toBe(address);
    await page.getByRole('tab', { name: 'Send' }).click();
    await page.screenshot({ path: 'test-results/flow-1-empty.png' });
  });

  await test.step('funds arrive: refresh tweens the card to the new balance', async () => {
    rpc.usdc = 100_000_000n; rpc.native = 2n * 10n ** 18n;
    await page.getByRole('button', { name: 'Refresh balances' }).click();
    await expect(page.getByTestId('asset-balance')).toHaveText('100');
    await expect(page.getByTestId('native-balance')).toHaveText('2 TCRO');
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-shown-balance', '100.00');
    await page.screenshot({ path: 'test-results/flow-2-funded.png' });
  });

  await test.step('send 12.34: review, change of mind, review again, confirm', async () => {
    await page.getByLabel('Recipient address').fill(recipient);
    await page.getByLabel('Amount', { exact: true }).fill('12.34');
    await page.getByRole('button', { name: 'Review transfer' }).click();
    const review = page.getByTestId('wallet-review');
    await expect(review).toContainText('12.34');
    await review.getByRole('button', { name: 'Cancel' }).click();
    await expect(review).toHaveCount(0);
    // The form kept what was typed.
    await expect(page.getByLabel('Amount', { exact: true })).toHaveValue('12.34');
    await page.getByRole('button', { name: 'Review transfer' }).click();
    await expect(review).toContainText(recipient);
    await page.screenshot({ path: 'test-results/flow-3-review.png' });
    rpc.receiptAfter = 2;
    rpc.onReceipt = () => { rpc.usdc = 87_660_000n; };
    await confirmSend(page);
    const row = page.getByTestId('wallet-activity');
    await expect(row).toContainText('Pending');
    await expect(row).toContainText('Confirmed', { timeout: 15000 });
    await expect(page.getByTestId('asset-balance')).toHaveText('87.66');
    expect(rpc.sent).toHaveLength(1);
    const [, , , to, , data, v] = rlpDecode(unhex(rpc.sent[0].raw));
    expect(hex(to)).toBe('0xc01efaaf7c5c61bebfaeb358e1161b537b8bc0e0');
    expect(hex(data).slice(-64)).toBe((12_340_000n).toString(16).padStart(64, '0'));
    expect([711n, 712n]).toContain(big(v));
    await page.screenshot({ path: 'test-results/flow-4-sent.png' });
  });

  await test.step('switch to mainnet and back: the activity keeps its own network', async () => {
    await page.getByTestId('main-usdc-cronos-mainnet').click();
    await expect(page.getByTestId('wallet-card')).toHaveAttribute('data-network', 'cronos-mainnet');
    await expect(page.getByTestId('asset-balance')).toHaveText('0');
    await expect(page.getByTestId('wallet-activity').getByRole('link')).toHaveAttribute('href', /explorer\.cronos\.org\/testnet\/tx\//);
    await page.getByRole('combobox', { name: 'Search coins and networks' }).fill('devusdc');
    await page.getByRole('combobox', { name: 'Search coins and networks' }).press('Enter');
    await expect(page.getByTestId('asset-balance')).toHaveText('87.66');
  });

  await test.step('account details copy the private key; sign out forgets it', async () => {
    await page.getByText('Settings', { exact: true }).click();
    await page.getByRole('button', { name: 'Account details' }).click();
    await page.getByTestId('copy-private-key').click();
    await expect.poll(() => clipboard(page)).toBe(privateKey);
    await noSecretOnScreen(page, [privateKey, phrase]);
    await page.getByRole('button', { name: 'Close account details' }).click();
    await page.getByRole('button', { name: 'Sign out' }).click();
    await expect(page.getByLabel('Recovery phrase', { exact: true })).toBeVisible();
    expect(await saved(page)).toBeNull();
    await expect(page.getByTestId('saved-wallet')).toHaveCount(0);
  });

  await test.step('sign back in with the private key: same wallet, same network', async () => {
    await page.getByRole('button', { name: 'Private key', exact: true }).click();
    await page.getByLabel('Private key', { exact: true }).fill(privateKey);
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Wallet', exact: true })).toBeVisible();
    expect((await info(page)).address).toBe(address);
    await page.getByRole('button', { name: 'Wallet', exact: true }).click();
    await expect(page.getByTestId('asset-balance')).toHaveText('87.66');
    await page.screenshot({ path: 'test-results/flow-5-back.png' });
  });

  await test.step('no secret ever reached the screen, the backend or AI', async () => {
    await noSecretOnScreen(page, [phrase, privateKey]);
    for (const { url, body } of bodies) {
      expect(body, url).not.toContain(phrase.split(' ').slice(0, 4).join(' '));
      expect(body, url).not.toContain(privateKey.slice(2));
      if (!RPC.test(url)) expect(body, url).not.toContain('eth_sendRawTransaction');
    }
    expect(errors).toEqual([]);
  });
  await context.close();
  await adminContext.close();
});
