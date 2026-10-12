// The erasure stamp is the backend's: it lives on each connection, so a
// browser's remembered value cannot lock it out, and a page that is told
// about a Delete all elsewhere reconnects and can keep writing.
import {test, expect} from '@playwright/test';

async function openChat(page) {
  await page.goto('/chat');
  await page.getByRole('button', {name: 'New chat', exact: true}).click();
  await expect(page.getByRole('textbox', {name: 'Chat message', exact: true})).toBeEnabled();
}
async function send(page, text) {
  await page.getByRole('textbox', {name: 'Chat message', exact: true}).fill(text);
  await page.getByRole('button', {name: 'Send message'}).click();
  await expect(page.getByText(text).first()).toBeVisible();
  await expect(page.getByText('Your workspace was erased')).toHaveCount(0);
}

test('a stale stamp remembered by the browser cannot block sending', async ({page}) => {
  await page.goto('/chat');
  await page.evaluate(() => {
    const key = 'causewaybay.settings.v1';
    const prefs = JSON.parse(localStorage.getItem(key) || '{}');
    prefs.erased = '99999999999999';
    localStorage.setItem(key, JSON.stringify(prefs));
  });
  await page.reload();
  await page.getByRole('button', {name: 'New chat', exact: true}).click();
  await send(page, 'Stamp check after switching backends');
});

test('a page that is told about Delete all elsewhere keeps working', async ({browser}) => {
  const a = await browser.newPage();
  const b = await browser.newPage();
  await openChat(a);
  await send(a, 'Before the erasure');
  await openChat(b);
  const toggle = b.getByRole('button', {name: 'Open navigation', exact: true});
  if (await toggle.isVisible()) await toggle.click();
  await b.locator('[data-nav="delete-all"]').click();
  await b.getByTestId('delete-all-confirm').click();
  await expect(b.getByTestId('delete-all-dialog')).toHaveCount(0);
  // Page A hears about it, reconnects, and can send again without a reload.
  await expect(a.getByText('Before the erasure')).toHaveCount(0, {timeout: 15_000});
  // The page reconnects on its own; a new room then opens once the connection is back.
  const box = a.getByRole('textbox', {name: 'Chat message', exact: true});
  await expect.poll(async () => {
    if (await box.isEnabled().catch(() => false)) return true;
    await a.getByRole('button', {name: 'New chat', exact: true}).click().catch(() => {});
    await a.waitForTimeout(600);
    return box.isEnabled().catch(() => false);
  }, {timeout: 25_000, intervals: [700]}).toBe(true);
  await send(a, 'After the erasure');
  // The deleting page keeps working too, without a reload.
  const boxB = b.getByRole('textbox', {name: 'Chat message', exact: true});
  await expect.poll(async () => {
    if (await boxB.isEnabled().catch(() => false)) return true;
    await b.getByRole('button', {name: 'New chat', exact: true}).click().catch(() => {});
    await b.waitForTimeout(600);
    return boxB.isEnabled().catch(() => false);
  }, {timeout: 25_000, intervals: [700]}).toBe(true);
  await send(b, 'The deleting page continues');
  await a.close();
  await b.close();
});
