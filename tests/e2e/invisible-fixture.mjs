import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {expect} from '@playwright/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
// Policy changes in these suites must not affect unrelated legacy browser tests.
export function isolatedBackend(test,port,extraEnv={}){
 const baseURL=`http://127.0.0.1:${port}`;let server,dir;
 test.use({baseURL});
 test.beforeAll(async({request})=>{
  dir=fs.mkdtempSync(path.join(os.tmpdir(),'invisible-ui-'));
  server=spawn('./target/debug/causewaybay-server',['serve','--demo','--no-semantic'],{env:{...process.env,BIND_ADDR:`127.0.0.1:${port}`,DATA_DIR:dir,CAUSEWAYBAY_CONFIG_DIR:dir,CWBBOT_ADMIN:'',APP_TOKEN:'',DEMO_MODE:'1',SEMANTIC_SEARCH:'off',XAI_API_KEY:'',OPENAI_API_KEY:'',GEMINI_API_KEY:'',ANTHROPIC_API_KEY:'',...(typeof extraEnv==='function'?extraEnv():extraEnv)},stdio:'ignore'});
  await expect.poll(async()=>(await request.get(baseURL+'/api/health').catch(()=>null))?.status()).toBe(200);
 });
 test.afterAll(async()=>{if(server?.exitCode===null&&server.signalCode===null){const done=once(server,'exit');server.kill('SIGTERM');await done;}fs.rmSync(dir,{recursive:true,force:true});});
 return baseURL;
}
