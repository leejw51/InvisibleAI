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
  const details = page.locator('.conversation-options');
  await expect(details.locator(':scope > summary')).toBeVisible();
  if ((await details.getAttribute('open')) === null) await details.locator(':scope > summary').click();
}
