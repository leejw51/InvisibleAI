import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
const key = 'causewaybay.capture.v1:local';
async function stop(child) {
  if (child && child.exitCode === null && child.signalCode === null) {
    const exited = once(child, 'exit'); child.kill('SIGTERM'); await exited;
  }
}

test('active and default devices persist, sync across tabs, and never switch on disconnect', async ({page, context, request}) => {
  test.setTimeout(90000);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cw-device-choice-'));
  const children = new Map();
  const start = name => {
    const child = spawn('./target/debug/rust-cli', ['client','--backend','http://127.0.0.1:3099','--name',name,'--client-id',`choice-${name}`], {env:{...process.env, APP_TOKEN:'', CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(dir,name)}, stdio:'ignore'});
    children.set(name,child);
  };
  const nodes = async () => (await (await request.get('/api/clients')).json());
  const sent = []; const errors = [];
  page.on('pageerror', e=>errors.push(e.message));
  page.on('websocket', ws=>ws.on('framesent', frame=>{const v=JSON.parse(String(frame.payload));if(v.path==='/api/think')sent.push(v.body);}));
  let other;
  try {
    start('Desk'); start('Laptop');
    await expect.poll(async()=> (await nodes()).filter(c=>c.id.startsWith('choice-')&&c.online).length).toBe(2);
    await page.goto('/');
    const picker = page.getByRole('combobox',{name:'Active capture device'});
    await expect(picker).toHaveValue('choice-Desk');
    await picker.selectOption('choice-Laptop');
    await page.getByRole('button',{name:'Capture',exact:true}).click();
    await expect.poll(()=>sent.at(-1)?.client_id).toBe('choice-Laptop');
    await expect(page.locator('.from-ai').last()).toContainText('returns 5');
    await expect(picker).toBeDisabled();
    await page.reload();
    await expect(picker).toHaveValue('choice-Laptop');
    await page.getByRole('button',{name:'Devices',exact:true}).click();
    await page.locator('[data-client-id="choice-Desk"]').getByRole('button',{name:'Use Desk as default'}).click();
    await expect(page.locator('[data-client-id="choice-Desk"] .device-default')).toHaveText('Default');
    await page.getByRole('button',{name:'New thought',exact:true}).click();
    await expect(picker).toHaveValue('choice-Desk');
    await page.reload(); await expect(picker).toHaveValue('choice-Desk');
    other = await context.newPage(); await other.goto('/');
    const otherPicker = other.getByRole('combobox',{name:'Active capture device'});
    await expect(otherPicker).toHaveValue('choice-Desk');
    await picker.selectOption('choice-Laptop');
    await expect(otherPicker).toHaveValue('choice-Laptop');
    await page.reload(); await expect(picker).toHaveValue('choice-Laptop');
    await expect(page.locator('.capture-options')).toHaveCount(0);
    await page.getByRole('button',{name:'Capture',exact:true}).click();
    await expect.poll(()=>sent.length).toBe(2);
    expect(sent.at(-1).client_id).toBe('choice-Laptop'); // Manual capture keeps the selected device.
    await stop(children.get('Laptop'));
    await expect(page.getByText('Selected device is offline or unavailable. Reconnect it or choose another device.')).toBeVisible();
    await expect(picker).toHaveValue('choice-Laptop');
    await expect(page.getByRole('button',{name:'Capture',exact:true})).toBeDisabled();
    await page.reload(); await expect(picker).toHaveValue('choice-Laptop');
    await expect(page.getByRole('button',{name:'Capture',exact:true})).toBeDisabled();
    const stored=await page.evaluate(key=>JSON.parse(localStorage.getItem(key)),key);
    expect(stored).toEqual({active:'choice-Laptop',default:'choice-Desk',continuous:[]});
    start('Laptop');
    await expect(page.getByRole('button',{name:'Capture',exact:true})).toBeEnabled();
    await expect(picker).toHaveValue('choice-Laptop');
    await page.setViewportSize({width:390,height:844});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:'test-results/device-choice-mobile.png',fullPage:true});
    await page.getByRole('button',{name:'Switch to dark mode'}).click();
    await page.screenshot({path:'test-results/device-choice-dark.png',fullPage:true});
    expect(errors).toEqual([]);
  } finally {if(other)await other.close();await Promise.all([...children.values()].map(stop));}
});

test('missing saved device remains selected and cannot fall back to a demo or another computer', async ({page,request}) => {
  await request.post('/api/rooms/thought',{data:{fresh:true}});
  await page.addInitScript(({key})=>localStorage.setItem(key,JSON.stringify({active:'removed-device',default:'removed-device'})),{key});
  await page.goto('/');
  await expect(page.getByRole('combobox',{name:'Active capture device'})).toHaveValue('removed-device');
  await expect(page.getByRole('button',{name:'Capture',exact:true})).toBeDisabled();
  await expect(page.getByText('Selected device is offline or unavailable. Reconnect it or choose another device.')).toBeVisible();
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill('Text questions still work');
  await expect(page.getByRole('button',{name:'Send message'})).toBeEnabled();
});
