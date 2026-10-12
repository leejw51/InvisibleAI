import {test,expect} from '@playwright/test';

test('Dashboard reflects real totals, switches admin scope, and renders an accessible responsive diagram',async({page,request})=>{
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.setViewportSize({width:1440,height:1080});
  await page.goto('/');
  if(!(await page.locator('[data-nav="dashboard"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.getByRole('button',{name:'Dashboard',exact:true}).click();
  await expect(page).toHaveURL(/\/dashboard$/);
  await expect(page.getByRole('heading',{name:'Your workspace, at a glance.'})).toBeVisible();
  const data=await(await request.get('/api/status')).json();
  for(const [key,value] of Object.entries({memos:data.stats.memos,files:data.stats.files,rooms:data.stats.rooms}))await expect(page.getByTestId(`status-${key}`)).toHaveText(String(value));
  await expect(page.locator('cwb-status-diagram')).toHaveAttribute('data-renderer','three');
  await expect(page.locator('cwb-status-diagram canvas')).toBeVisible();
  await expect(page.getByRole('button',{name:'Download client.zip',exact:true})).toBeDisabled();
  await expect(page.getByText('Sign in with your wallet to download your client.')).toBeVisible();
  await page.screenshot({path:'test-results/dashboard-desktop.png',fullPage:true});
  await page.getByRole('button',{name:'System overview',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Everything, working together.'})).toBeVisible();
  await expect(page.getByLabel('System health')).toContainText('Semantic search');
  await expect(page.locator('cwb-status-diagram')).toHaveAttribute('data-state',/"scope":"system"/);
  await page.getByRole('button',{name:'Switch to dark mode'}).click();
  await page.screenshot({path:'test-results/dashboard-system-dark.png',fullPage:true});
  await page.getByRole('button',{name:'My workspace',exact:true}).click();
  const uploaded=await request.post('/api/files',{data:{name:'dashboard-real-total.txt',mime:'text/plain',data_base64:Buffer.from('Dashboard statistics fixture').toString('base64')}});
  expect(uploaded.ok()).toBe(true);
  await expect(page.getByTestId('status-files')).toHaveText(String(data.stats.files+1));
  const file=await uploaded.json();await request.delete(`/api/files/${file.id}`);
  await expect(page.getByTestId('status-files')).toHaveText(String(data.stats.files));
  await page.setViewportSize({width:390,height:844});
  await page.screenshot({path:'test-results/dashboard-mobile-dark.png',fullPage:true});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.getByRole('button',{name:'Switch to light mode'}).click();
  await page.screenshot({path:'test-results/dashboard-mobile.png',fullPage:true});
  await page.reload();await expect(page.getByRole('heading',{name:'Your workspace, at a glance.'})).toBeVisible();
  expect(errors).toEqual([]);
});

test('Dashboard data stays readable when WebGL is unavailable',async({page})=>{
  await page.addInitScript(()=>{
    const original=HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext=function(type,...args){return String(type).includes('webgl')?null:original.call(this,type,...args)};
  });
  await page.goto('/dashboard');
  await expect(page.locator('cwb-status-diagram')).toHaveAttribute('data-renderer','unavailable');
  await expect(page.getByTestId('status-memos')).toBeVisible();
  await expect(page.getByLabel('Workspace diagram')).toContainText('Web clients');
});
