import {expect} from '@playwright/test';

// The New thought workspace shows the AI processing mode read-only. Its Change
// button opens the Autonomous AI page, which owns the mode cards.
export async function openProcessingSettings(page, name = 'AI processing mode') {
  await openWorkspace(page);
  const settings = page.getByTestId('processing-settings');
  await expect(settings).toBeVisible();
  await settings.getByRole('button').click();
  await expect(page).toHaveURL(/\/autonomous$/);
  await page.locator('.simple-ai-settings > summary').click();
  const group = page.getByRole('group', {name});
  await expect(group).toBeVisible();
  return group;
}
export async function expectProcessingMode(page, label) {
  await expect(page.getByTestId('processing-settings')).toContainText(label);
}

export async function openWorkspace(page) {
  const details = page.locator('.everyday-workspace');
  await expect(details.locator(':scope > summary')).toBeVisible();
  if ((await details.getAttribute('open')) === null) await details.locator(':scope > summary').click();
}
export async function openConversationOptions(page) {
  // The room picker, toolbar and mode switch fold under "Conversation options"
  // once a room is open; an empty page shows them unfolded.
  const details = page.locator('.conversation-options');
  const summary = details.locator(':scope > summary');
  // A fresh page shows the empty-room toolbar unfolded for a moment before the
  // room arrives and folds it: wait for the fold, or for a page that has no room.
  // Pages without a chat (overview, library…) have nothing to unfold.
  if (!(await page.locator('.chat-page').first().waitFor({state: 'attached', timeout: 3000}).then(() => true, () => false))) return;
  // The Capture page always ends up with a room (and the fold); only other
  // pages may legitimately stay in the empty state.
  // Rooms load after the page: give the fold a grace period before trusting an
  // empty state (the Capture page always gets a room, so it waits the full time).
  const onCapturePage = new URL(page.url()).pathname === '/';
  if (!(await summary.waitFor({state: 'visible', timeout: onCapturePage ? 15000 : 8000}).then(() => true, () => false))) return;
  if ((await details.getAttribute('open')) === null) await summary.click();
}
// The sidebar's two folds: "More tools" (libraries, devices, dashboard…) and
// "Settings" (AI, assistance, admin, delete all, sign out).
async function openFold(page, selector) {
  const details = page.locator(selector);
  const summary = details.locator(':scope > summary');
  // Pages without that fold (the login card, for one) have nothing to open.
  if (!(await details.first().waitFor({state: 'attached', timeout: 3000}).then(() => true, () => false))) return;
  await expect(summary).toBeVisible();
  if ((await details.getAttribute('open')) === null) await summary.click();
}
export const openMoreTools = page => openFold(page, '.more-tools');
export const openSettings = page => openFold(page, '.everyday-settings');
export const openActivity = page => openFold(page, '.helper-history');

// Open the fold that hides a sidebar item, without following it.
export async function revealNav(page, destination) {
  const item = page.locator(`[data-nav="${destination}"]`);
  if (await item.isVisible()) return;
  const fold = item.locator('xpath=ancestor::details[1]');
  if (await fold.count() && (await fold.getAttribute('open')) === null) await fold.locator(':scope > summary').click();
}
// Sidebar destinations behind a fold ("More tools" or "Settings") open it first.
export async function openNav(page, destination) {
  const menu = page.getByRole('button', {name: 'Open navigation', exact: true});
  if (await menu.isVisible() && await menu.getAttribute('aria-expanded') !== 'true') await menu.click();
  const item = page.locator(`[data-nav="${destination}"]`);
  await revealNav(page, destination);
  await item.click();
}
