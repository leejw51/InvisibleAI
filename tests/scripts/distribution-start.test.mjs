import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

// Exercise the actual launcher with synthetic binaries, without opening a browser
// or touching the user's config. The server deliberately ignores graceful stop.
test('desktop launcher stops both children and releases its lock with an open connection', { timeout: 12000 }, async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'invisible-launcher-'));
  let child;
  const pids = [];
  try {
    fs.mkdirSync(path.join(root, 'bin'));
    const config = path.join(root, 'config');
    fs.mkdirSync(config);
    fs.writeFileSync(path.join(config, 'capture-token'), 'synthetic-token');
    for (const name of ['server', 'client']) {
      fs.writeFileSync(path.join(root, 'bin', name), `#!${process.execPath}\nrequire('fs').writeFileSync(${JSON.stringify(path.join(root,name+'.pid'))},String(process.pid));\n${name === 'server' ? "process.on('SIGTERM',()=>{});" : ''}\nsetInterval(()=>{},100);\n`, {mode:0o755});
    }
    const source = fs.readFileSync('scripts/distribution-start.command','utf8')
      .replace('/usr/bin/curl', 'true').replace('/usr/bin/open http://127.0.0.1:3000', 'touch "$ROOT/ready"');
    const launcher = path.join(root, 'Start Invisible AI.command');
    fs.writeFileSync(launcher, source, {mode:0o755});
    child = spawn('bash', [launcher], {env:{...process.env, CAUSEWAYBAY_CONFIG_DIR:config, CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(root,'client-config')},stdio:'ignore'});
    for(let i=0;i<150 && !['ready','server.pid','client.pid'].every(name=>fs.existsSync(path.join(root,name)));i++) await new Promise(r=>setTimeout(r,30));
    assert.ok(fs.existsSync(path.join(root,'ready')), 'launcher reached ready');
    for(const name of ['server','client']) {
      const pid = Number(fs.readFileSync(path.join(root,name+'.pid'),'utf8'));
      assert.ok(Number.isInteger(pid) && pid > 1);
      pids.push(pid);
    }
    const exited = once(child,'exit');
    child.kill('SIGTERM');
    await exited;
    assert.equal(fs.existsSync(path.join(config,'desktop-launch.lock')),false);
    for(const pid of pids) assert.throws(()=>process.kill(pid,0),{code:'ESRCH'});
  } finally {
    if(child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    for(const name of ['server','client']) {
      try { const pid = Number(fs.readFileSync(path.join(root,name+'.pid'),'utf8')); if(Number.isInteger(pid) && pid > 1) process.kill(pid,'SIGKILL'); } catch {}
    }
    fs.rmSync(root,{recursive:true,force:true});
  }
});
