import {test,expect} from '@playwright/test';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

test('signed-in Dashboard downloads a two-file client ZIP for its wallet and Tailscale IP',async({page,request})=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'cwb-personal-client-'));
  const base='http://127.0.0.1:3127', wallet='0x9858effd232b4033e47d90003d41ec34ecaeda94';
  const tools=path.join(dir,'tools');fs.mkdirSync(tools);
  fs.writeFileSync(path.join(tools,'tailscale'),'#!/bin/sh\nprintf \'%s\\n\' \'{"BackendState":"Running","TailscaleIPs":["100.64.0.20"]}\'\n',{mode:0o755});
  const server=spawn('./target/debug/causewaybay-server',['serve','--demo','--no-semantic'],{env:{...process.env,PATH:`${tools}:${process.env.PATH}`,CWBBOT_ADMIN:wallet,BIND_ADDR:'0.0.0.0:3127',CAUSEWAYBAY_CONFIG_DIR:dir,DATA_DIR:path.join(dir,'data'),APP_TOKEN:'',XAI_API_KEY:'',OPENAI_API_KEY:'',GEMINI_API_KEY:'',ANTHROPIC_API_KEY:''},stdio:'ignore'});
  try {
    await expect.poll(async()=>{try{return(await request.get(base+'/api/health')).ok()}catch{return false}}).toBe(true);
    expect((await request.get(base+'/api/download/client.zip')).status()).toBe(401);
    await page.goto(base);
    await page.getByLabel('Recovery phrase').fill('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about');
    await page.getByRole('button',{name:'Sign in',exact:true}).click();
    if(!(await page.locator('[data-nav="dashboard"]').isVisible())) await page.locator('.more-tools > summary').click();
    await page.getByRole('button',{name:'Dashboard',exact:true}).click();
    const link=page.getByRole('link',{name:'Download client.zip',exact:true});
    await expect(link).toBeVisible();
    await expect(page.getByLabel('Capture client package')).toContainText('http://100.64.0.20:3127');
    await expect(page.getByRole('button',{name:'Copy owner wallet'})).toContainText(wallet);
    const pending=page.waitForEvent('download');await link.click();const download=await pending;
    expect(download.suggestedFilename()).toBe('client.zip');
    const zip=path.join(dir,'download.zip');await download.saveAs(zip);
    expect(execFileSync('unzip',['-Z1',zip],{encoding:'utf8'}).trim().split('\n').sort()).toEqual(['client','go.sh']);
    const unpack=path.join(dir,'unpacked');execFileSync('unzip',['-q',zip,'-d',unpack]);
    for(const file of ['client','go.sh'])fs.accessSync(path.join(unpack,file),fs.constants.X_OK);
    const script=fs.readFileSync(path.join(unpack,'go.sh'),'utf8');
    expect(script).toContain(`owner_wallet='${wallet}'`);
    expect(script).toContain("backend_url='http://100.64.0.20:3127'");
    expect(script).not.toContain('abandon');
    expect(fs.readFileSync(path.join(unpack,'client')).equals(fs.readFileSync('target/debug/rust-cli'))).toBe(true);
    await page.screenshot({path:'test-results/dashboard-personal-download.png',fullPage:true});
  } finally {
    if(server.exitCode===null&&server.signalCode===null){const exited=once(server,'exit');server.kill('SIGTERM');await exited;}
    fs.rmSync(dir,{recursive:true,force:true});
  }
});
