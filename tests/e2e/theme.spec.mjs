import { test, expect } from '@playwright/test';

test('theme persists across reloads and tabs without losing a chat draft', async ({page, context}) => {
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto('/');
  const draft=page.getByRole('textbox',{name:'Chat message',exact:true});
  await draft.fill('Keep this draft when the theme changes');
  await page.getByRole('button',{name:'Switch to dark mode'}).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
  expect(await page.evaluate(()=>localStorage.getItem('causewaybay.theme.v1'))).toBe('dark');
  await expect(draft).toHaveValue('Keep this draft when the theme changes');
  expect(await page.evaluate(()=>getComputedStyle(document.body).backgroundColor)).toBe('rgb(24, 22, 30)');
  const other=await context.newPage();
  try {
    await other.goto('/devices');
    await expect(other.locator('html')).toHaveAttribute('data-theme','dark');
    await page.reload();await expect(page.getByRole('button',{name:'Switch to light mode'})).toBeVisible();
    await page.getByRole('button',{name:'Switch to light mode'}).click();
    await expect(other.locator('html')).toHaveAttribute('data-theme','light');
    expect(await page.evaluate(()=>localStorage.getItem('causewaybay.theme.v1'))).toBe('light');
    await page.setViewportSize({width:390,height:844});
    await expect(page.getByRole('button',{name:'Switch to dark mode'})).toBeVisible();
    await page.getByRole('button',{name:'Switch to dark mode'}).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    expect(errors).toEqual([]);
  } finally {await other.close();}
});
