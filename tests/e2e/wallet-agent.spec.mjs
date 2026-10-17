// The AI's wallet function call: `wallet_send` prepares a transfer of CRO or
// USDC on Cronos under the account's hard limits; the user confirms it on the
// Wallet page, where the key is. The limits (100 CRO, 10 USDC to start) live
// on the backend per account, editable on the Wallet page and in AI settings,
// and the Send form obeys them too. The demo fixture runs the real tool.
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fakeCronos, confirmSend } from './evm-fixture.mjs';
import { openConversationOptions } from './workspace-controls.mjs';

const base = 'http://127.0.0.1:3131';
// Public BIP-39 vector; never use it for assets.
const phrase = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
const recipient = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';
const CRO = 10n ** 18n;
let server;
test.beforeAll(async ({ request }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-wallet-agent-'));
  server = spawn('./target/debug/causewaybay-server', ['serve', '--demo', '--no-semantic'], {
    env: { ...process.env, CWBBOT_ADMIN: address, BIND_ADDR: '127.0.0.1:3131', DATA_DIR: path.join(dir, 'data'), CAUSEWAYBAY_CONFIG_DIR: dir, APP_TOKEN: '', XAI_API_KEY: '', OPENAI_API_KEY: '', ANTHROPIC_API_KEY: '', GEMINI_API_KEY: '' }, stdio: 'ignore',
  });
  await expect.poll(async () => (await request.get(base + '/api/health').catch(() => null))?.status(), { timeout: 30000 }).toBe(200);
});
test.afterAll(() => { server?.kill('SIGTERM'); });

const wallet = page => page.request.get(base + '/api/wallet').then(r => r.json());
/** Open a fresh chat room and return its id. */
async function newChat(page) {
  await page.goto(base + '/chat');
  await openConversationOptions(page);
  await page.getByRole('button', { name: 'New chat', exact: true }).click();
  await openConversationOptions(page);
  await expect(page.getByRole('combobox', { name: 'Chat room' })).not.toHaveValue('');
  await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeEnabled();
  return page.getByRole('combobox', { name: 'Chat room' }).inputValue();
}
async function ask(page, prompt) {
  await page.getByRole('textbox', { name: 'Chat message', exact: true }).fill(prompt);
  await page.getByRole('button', { name: 'Send message' }).click();
  const reply = page.locator('.from-ai').last();
  await expect(reply).toHaveAttribute('data-status', 'complete', { timeout: 30000 });
  return reply;
}

test('hard limits are per account and editable in two places; the AI prepares a transfer under them that the wallet confirms', async ({ browser }) => {
  test.setTimeout(180000);
  const context = await browser.newContext({ viewport: { width: 1360, height: 960 } });
  const rpc = { native: 30n * CRO, usdc: 0n, mainnetUsdc: 50_000_000n, nativeOf: { 'cronos-mainnet': 3n * CRO }, nativeOfAddress: { [recipient.toLowerCase()]: 1n * CRO } };
  await fakeCronos(context, rpc);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(base);
  await page.getByLabel('Recovery phrase', { exact: true }).fill(phrase);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Wallet', exact: true })).toBeVisible();

  await test.step('the Wallet page shows the defaults and saves a tighter TCRO limit', async () => {
    await page.getByRole('button', { name: 'Wallet', exact: true }).click();
    const limits = page.getByTestId('wallet-limits');
    await expect(limits.getByTestId('limit-tcro-cronos-testnet')).toHaveValue('100');
    await expect(limits.getByTestId('limit-cro-cronos-mainnet')).toHaveValue('100');
    await expect(limits.getByTestId('limit-usdc-cronos-mainnet')).toHaveValue('10');
    await expect(limits.getByTestId('save-limits')).toBeDisabled();
    await limits.getByTestId('limit-tcro-cronos-testnet').fill('20');
    await limits.getByTestId('save-limits').click();
    await expect(limits.getByTestId('limits-saved')).toBeVisible();
    expect((await wallet(page)).limits).toEqual({ 'tcro-cronos-testnet': '20' });
    // A limit that is not a number is refused and nothing changes.
    await limits.getByTestId('limit-usdc-cronos-mainnet').fill('ten');
    await limits.getByTestId('save-limits').click();
    await expect(limits.getByRole('alert')).toBeVisible();
    expect((await wallet(page)).limits).toEqual({ 'tcro-cronos-testnet': '20' });
  });

  await test.step('the same panel on AI settings reads the saved limit and changes USDC', async () => {
    await page.goto(base + '/ai');
    const limits = page.getByTestId('ai-wallet-limits');
    await expect(limits.getByTestId('limit-tcro-cronos-testnet')).toHaveValue('20');
    await limits.getByTestId('limit-usdc-cronos-mainnet').fill('5');
    await limits.getByTestId('save-limits').click();
    await expect(limits.getByTestId('limits-saved')).toBeVisible();
    expect((await wallet(page)).limits).toEqual({ 'tcro-cronos-testnet': '20', 'usdc-cronos-mainnet': '5' });
  });

  await test.step('the Send form refuses a transfer over the limit before anything is signed', async () => {
    await page.goto(base + '/wallet');
    await expect(page.getByTestId('asset-symbol')).toHaveText('TCRO');
    await page.getByLabel('Recipient address').fill(recipient);
    await page.getByLabel('Amount', { exact: true }).fill('25');
    await page.getByRole('button', { name: 'Review transfer' }).click();
    await expect(page.getByTestId('wallet-page').getByRole('alert').first()).toContainText('hard limit for TCRO is 20 TCRO');
    await expect(page.getByTestId('wallet-review')).toHaveCount(0);
    expect(rpc.sent).toHaveLength(0);
  });

  let room;
  await test.step('a chat asking for more than the limit gets a refusal and no transfer', async () => {
    room = await newChat(page);
    const reply = await ask(page, `Please send 500 TCRO to ${recipient}`);
    await expect(reply).toContainText('Transfer not prepared');
    await expect(reply).toContainText('hard limit of 20 TCRO');
    await expect(page.getByTestId('chat-transfer')).toHaveCount(0);
    expect((await wallet(page)).pending).toEqual([]);
  });

  await test.step('a chat under the limit prepares the transfer: a card in the chat, the review on the Wallet page, confirmed there', async () => {
    const reply = await ask(page, `Please send 5 TCRO to ${recipient}`);
    await expect(reply).toContainText('Transfer prepared');
    const card = page.getByTestId('chat-transfer');
    await expect(card).toHaveAttribute('data-status', 'pending');
    await expect(card).toContainText('5 TCRO');
    await expect(card).toContainText('Cronos Testnet');
    const pending = (await wallet(page)).pending;
    expect(pending).toHaveLength(1);
    expect(pending[0]).toMatchObject({ coin: 'tcro-cronos-testnet', symbol: 'TCRO', to: recipient, amount: '5', status: 'pending' });
    expect(rpc.sent).toHaveLength(0);
    await page.screenshot({ path: 'test-results/wallet-agent-1-chat.png', fullPage: true });

    await card.getByTestId('review-in-wallet').click();
    await expect(page).toHaveURL(/\/wallet$/);
    await expect(page.getByTestId('agent-request')).toContainText('5 TCRO');
    // The form is filled in and the review opens by itself; the wallet's own
    // checks run, and nothing is sent until Confirm.
    await expect(page.getByLabel('Recipient address')).toHaveValue(recipient);
    await expect(page.getByLabel('Amount', { exact: true })).toHaveValue('5');
    const review = page.getByTestId('wallet-review');
    await expect(review).toContainText('5');
    await expect(review).toContainText(recipient);
    expect(rpc.sent).toHaveLength(0);
    await page.screenshot({ path: 'test-results/wallet-agent-2-review.png', fullPage: true });
    rpc.receiptAfter = 1;
    rpc.onReceipt = () => { rpc.native = 24_895_000_000_000_000_000n; };
    await confirmSend(page);
    await expect(page.getByTestId('wallet-activity')).toContainText('Confirmed', { timeout: 15000 });
    expect(rpc.sent).toHaveLength(1);
    await expect(page.getByTestId('agent-request')).toHaveCount(0);
    await expect.poll(async () => (await wallet(page)).pending).toEqual([]);
  });

  await test.step('back in the chat the card says the transfer was sent and links the explorer', async () => {
    await page.goto(base + '/chat');
    await openConversationOptions(page);
    await page.getByRole('combobox', { name: 'Chat room' }).selectOption(room);
    const card = page.getByTestId('chat-transfer');
    await expect(card).toHaveAttribute('data-status', 'sent');
    await expect(card.getByRole('link', { name: /Explorer/ })).toHaveAttribute('href', /explorer\.cronos\.org\/testnet\/tx\/0x[0-9a-f]{64}$/);
    await expect(card.getByTestId('review-in-wallet')).toHaveCount(0);
  });

  await test.step('a mainnet USDC request is prepared under its own limit and can be dismissed in the wallet', async () => {
    const refused = await ask(page, `Please send 6 USDC to ${recipient}`);
    await expect(refused).toContainText('hard limit of 5 USDC');
    const reply = await ask(page, `Please send 1 USDC to ${recipient}`);
    await expect(reply).toContainText('Cronos Mainnet');
    const card = page.getByTestId('chat-transfer').last();
    await expect(card).toContainText('1 USDC');
    await card.getByTestId('review-in-wallet').click();
    await expect(page).toHaveURL(/\/wallet$/);
    await expect(page.getByTestId('asset-symbol')).toHaveText('USDC');
    await expect(page.getByTestId('asset-network')).toHaveText('Cronos Mainnet');
    await expect(page.getByTestId('agent-request')).toContainText('1 USDC');
    await page.getByTestId('dismiss-request').click();
    await expect(page.getByTestId('agent-request')).toHaveCount(0);
    await expect.poll(async () => (await wallet(page)).pending).toEqual([]);
    expect(rpc.sent).toHaveLength(1);
    await page.goto(base + '/chat');
    await openConversationOptions(page);
    await page.getByRole('combobox', { name: 'Chat room' }).selectOption(room);
    await expect(page.getByTestId('chat-transfer').last()).toHaveAttribute('data-status', 'cancelled');
  });

  // The limits are the account's: they survive the whole flow unchanged.
  const kept = await (await context.request.get(base + '/api/wallet')).json();
  expect(kept.limits).toEqual({ 'tcro-cronos-testnet': '20', 'usdc-cronos-mainnet': '5' });
  expect(errors).toEqual([]);
  await context.close();
});
