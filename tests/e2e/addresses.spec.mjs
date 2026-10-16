import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Playwright skips a test's `finally` after a timeout, so the spawned backend is
// released from afterEach instead.
const cleanup=[];
test.afterEach(async()=>{while(cleanup.length){try{await cleanup.pop()();}catch{}}});
function stopChild(child){
  if(child.exitCode!==null||child.signalCode!==null)return Promise.resolve();
  const exited=once(child,'exit');child.kill('SIGTERM');
  const hammer=setTimeout(()=>child.kill('SIGKILL'),5000);
  return exited.finally(()=>clearTimeout(hammer));
}

test('Devices lists backend LAN and Tailscale addresses and copies exact commands and URLs', async ({page, context, request}) => {
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'cw-addresses-'));
  const bin=path.join(dir,'bin');fs.mkdirSync(bin);
  const status={BackendState:'Running',TailscaleIPs:['100.64.0.10'],Self:{DNSName:'studio.tailnet.ts.net.'}};
  // An isolated CLI fixture exercises backend discovery, without relying on a developer's tailnet.
  fs.writeFileSync(path.join(bin,'tailscale'),`#!/bin/sh\ncat <<'TAILSCALE_STATUS'\n${JSON.stringify(status)}\nTAILSCALE_STATUS\n`,{mode:0o700});
  const base='http://127.0.0.1:3124';
  // Address discovery only runs for a listener that accepts other interfaces, and
  // such a listener now requires APP_TOKEN or CWBBOT_ADMIN. The token keeps the
  // session wallet-less, so the capture commands stay `./bin/client <url>`.
  const token='addresses-e2e-token-0123';
  const server=spawn('./target/debug/causewaybay-server',['serve','--demo','--no-semantic'],{env:{...process.env,PATH:`${bin}:${process.env.PATH}`,CWBBOT_ADMIN:'',BIND_ADDR:'0.0.0.0:3124',DATA_DIR:path.join(dir,'data'),CAUSEWAYBAY_CONFIG_DIR:dir,APP_TOKEN:token,XAI_API_KEY:'',OPENAI_API_KEY:'',ANTHROPIC_API_KEY:'',GEMINI_API_KEY:''},stdio:'ignore'});
  cleanup.push(()=>stopChild(server));
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  const api=path=>request.get(base+path,{headers:{Authorization:`Bearer ${token}`}});
  await expect.poll(async()=> (await api('/api/health').catch(()=>null))?.status(),{timeout:30000}).toBe(200);
  await context.grantPermissions(['clipboard-read','clipboard-write'],{origin:base});
  await page.goto(base+'/devices');
  await page.getByLabel('Workspace access token').fill(token);
  await page.getByRole('button',{name:'Open workspace',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Your connected devices.'})).toBeVisible();
  await expect(page.getByLabel('Capture client command')).toHaveCount(2);
  const addresses=(await (await api('/api/dashboard')).json()).addresses;
  expect(addresses).toEqual(expect.arrayContaining([{label:'Local access',web_url:base},{label:'Tailscale DNS',web_url:'http://studio.tailnet.ts.net:3124'},{label:'Tailscale IP',web_url:'http://100.64.0.10:3124'}]));
  for(const [label,url] of [['Tailscale IP','http://100.64.0.10:3124'],['Tailscale DNS','http://studio.tailnet.ts.net:3124']]) {
    const group=page.getByRole('group',{name:label,exact:true});
    await expect(group.getByLabel('Capture client command')).toHaveText(`./bin/client ${url}`);
    await group.getByRole('button',{name:'Copy capture command',exact:true}).click();
    expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(`./bin/client ${url}`);
  }
  const groups=page.getByTestId('backend-address');
  await expect(groups).toHaveCount(addresses.length);
  for(let i=0;i<addresses.length;i++) {
    await expect(groups.nth(i).getByRole('heading')).toHaveText(addresses[i].label);
    for(const [label,value] of [['Web URL',addresses[i].web_url],['API URL',addresses[i].web_url+'/api']]) {
      await groups.nth(i).getByRole('button',{name:`Copy ${label}`,exact:true}).click();
      expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(value);
    }
  }
  await page.screenshot({path:'test-results/addresses-light.png',fullPage:true});
  await page.getByRole('button',{name:'Switch to dark mode'}).click();
  await page.screenshot({path:'test-results/addresses-dark.png',fullPage:true});
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.screenshot({path:'test-results/addresses-dark-mobile.png',fullPage:true});
  expect(errors).toEqual([]);
});
