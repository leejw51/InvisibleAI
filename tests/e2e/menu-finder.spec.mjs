// The sidebar finder locates any menu by its name or its tags, in English or
// the chosen language; Delete all sits in the main navigation and sweeps the
// browser's own copies; the Devices page offers a one-paste install from source.
import {test, expect} from '@playwright/test';

const port = process.env.PW_PORT || '3099';
const finder = page => page.getByTestId('nav-search');
const results = page => page.getByTestId('nav-results');

test('the finder matches menus by name or tag, shows the tags, and opens the match', async ({page}) => {
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  await page.goto('/');
  await expect(finder(page)).toHaveAttribute('placeholder', 'Find a menu…');
  await expect(results(page)).toHaveCount(0);
  await finder(page).fill('wallet');
  const found = results(page).locator('[data-nav]');
  await expect(found.first()).toHaveAttribute('data-nav', 'wallet');
  await expect(results(page).locator('[data-nav="wallet"] .nav-tags')).toHaveText('crypto, balance, send, receive, Cronos, USDC');
  // The folds are replaced by one flat list while searching.
  await expect(page.locator('details.more-tools')).toHaveCount(0);
  // A tag finds a menu whose name says nothing about it.
  await finder(page).fill('screenshot');
  await expect(found).toHaveCount(1);
  await expect(found.first()).toHaveAttribute('data-nav', 'home');
  await finder(page).fill('client.zip');
  await expect(found.first()).toHaveAttribute('data-nav', 'dashboard');
  // Every word must match somewhere.
  await finder(page).fill('capture computer');
  await expect(found).toHaveCount(1);
  await expect(found.first()).toHaveAttribute('data-nav', 'devices');
  await finder(page).fill('zzzz-nothing');
  await expect(results(page)).toContainText('No menu matches. Try another word.');
  await expect(found).toHaveCount(0);
  // Enter opens the first match and clears the finder; the folds return.
  await finder(page).fill('devices');
  await finder(page).press('Enter');
  await expect(page.getByRole('heading', {name: 'Your connected devices.'})).toBeVisible();
  await expect(finder(page)).toHaveValue('');
  await expect(page.locator('details.more-tools')).toHaveCount(1);
  // Clicking a result navigates too, and Escape clears without navigating.
  await finder(page).fill('ask');
  await results(page).locator('[data-nav="chat"]').click();
  await expect(page).toHaveURL(/\/chat$/);
  await finder(page).fill('wiki');
  await finder(page).press('Escape');
  await expect(finder(page)).toHaveValue('');
  await expect(page).toHaveURL(/\/chat$/);
  expect(errors).toEqual([]);
});

test('the finder understands the chosen language and the English words', async ({page}) => {
  await page.goto('/');
  await page.getByTestId('language-select').selectOption('ko');
  await expect(finder(page)).toHaveAttribute('placeholder', '메뉴 찾기…');
  await finder(page).fill('지갑');
  await expect(results(page).locator('[data-nav="wallet"]')).toBeVisible();
  await finder(page).fill('스크린샷');
  await expect(results(page).locator('[data-nav="home"]')).toBeVisible();
  await finder(page).fill('wallet');
  await expect(results(page).locator('[data-nav="wallet"]')).toBeVisible();
  await finder(page).fill('지워');
  await expect(results(page).locator('[data-nav="delete-all"]')).toHaveText(/다 지워/);
});

test('Delete all lives in the main navigation, asks first, and sweeps this browser', async ({page}) => {
  await page.goto('/');
  const item = page.locator('nav[aria-label="Main navigation"] [data-nav="delete-all"]');
  await expect(item).toBeVisible();
  await expect(item).toHaveText('Delete all');
  await expect(page.locator('details.everyday-settings [data-nav="delete-all"]')).toHaveCount(0);
  await item.click();
  await expect(page.getByTestId('delete-all-dialog')).toContainText('Only your wallet key stays in this browser');
  await page.getByRole('button', {name: 'Cancel', exact: true}).click();
  await expect(page.getByTestId('delete-all-dialog')).toHaveCount(0);
  await page.evaluate(() => {
    localStorage.setItem('causewaybay.capture.v1:local', '{"device":"mac"}');
    localStorage.setItem('causewaybay.wiki.view.v2', 'graph');
    localStorage.setItem('causewaybay.coding.prompt.p1', 'draft prompt');
    localStorage.setItem('causewaybay.upload.resume.abc', 'u1');
    localStorage.setItem('causewaybay.history.v1', '1');
    localStorage.setItem('causewaybay.theme.v1', 'dark');
    localStorage.setItem('causewaybay.language.v1', 'en');
    localStorage.setItem('invisible-ai.wallet.key', '{"key":"keep-me"}');
    sessionStorage.setItem('causewaybay.upload.resume.session', 'u2');
  });
  await item.click();
  await page.getByTestId('delete-all-confirm').click();
  await expect(page.getByTestId('delete-all-dialog')).toHaveCount(0);
  await expect(page.getByText('Everything was deleted')).toBeVisible();
  await expect.poll(() => page.evaluate(() => ({
    gone: ['causewaybay.capture.v1:local', 'causewaybay.wiki.view.v2', 'causewaybay.coding.prompt.p1', 'causewaybay.upload.resume.abc', 'causewaybay.history.v1'].map(k => localStorage.getItem(k)),
    session: sessionStorage.getItem('causewaybay.upload.resume.session'),
    kept: [localStorage.getItem('causewaybay.theme.v1'), localStorage.getItem('invisible-ai.wallet.key'), localStorage.getItem('causewaybay.language.v1')],
    erased: JSON.parse(localStorage.getItem('causewaybay.settings.v1') || '{}').erased,
  }))).toEqual({gone: [null, null, null, null, null], session: null, kept: ['dark', '{"key":"keep-me"}', 'en'], erased: expect.stringMatching(/^\d+$/)});
});

test('Devices offers a one-paste install from source with the backend URL filled in', async ({page, context}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/devices');
  const group = page.getByRole('group', {name: 'Install and run from source', exact: true});
  await expect(group).toContainText('No client yet? One paste installs Rust');
  await expect(page.getByTestId('capture-install-meta')).toHaveText(`Backend http://127.0.0.1:${port} · Shared capture (sign in with a wallet to enroll this Mac)`);
  const script = await group.getByLabel('Install and run command').innerText();
  const lines = script.split('\n');
  expect(lines[0]).toBe("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y &&");
  expect(lines.slice(0, -1).every(line => line.endsWith(' &&'))).toBe(true);
  expect(script).toContain('git clone https://github.com/leejw51/InvisibleAI.git');
  expect(script).toContain('cargo build --release -p causewaybay-cli --bin rust-cli');
  expect(lines.at(-1)).toBe(`./bin/client http://127.0.0.1:${port}`);
  // The plain run command keeps its own label and count for the existing specs.
  await expect(page.getByLabel('Capture client command')).toHaveCount(1);
  await group.getByRole('button', {name: 'Copy install command', exact: true}).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(script);
  await expect(group.getByRole('link', {name: /Dashboard has client.zip/})).toHaveAttribute('href', '/dashboard');
});

test('Capture opens with the connect command and a Copy button at the top', async ({page, context}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/');
  const strip = page.getByTestId('connect-strip');
  await expect(strip).toContainText('Connect a Mac');
  await expect(strip.getByLabel('Connect command', {exact: true})).toHaveText(`./bin/client http://127.0.0.1:${port}`);
  // The strip is the first thing in the page's main area.
  expect(await page.evaluate(() => document.querySelector('main').firstElementChild.dataset.testid)).toBe('connect-strip');
  await strip.getByRole('button', {name: 'Copy connect command', exact: true}).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`./bin/client http://127.0.0.1:${port}`);
  await expect(strip.getByRole('link', {name: /No client yet/})).toHaveAttribute('href', '/devices#device-setup');
  await page.goto('/chat');
  await expect(page.getByTestId('connect-strip')).toHaveCount(0);
});
