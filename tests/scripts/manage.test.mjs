import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync, spawnSync} from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

test('start-server replaces changed and legacy builds but preserves a current process', t => {
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'cwb-manage-test-'));
  const config=path.join(root,'config');
  const binary=path.join(root,'target/debug/causewaybay-server');
  const pidFile=path.join(config,'run/server.pid');
  const buildFile=path.join(config,'run/server.build');
  fs.mkdirSync(path.join(root,'scripts'),{recursive:true});
  fs.mkdirSync(path.dirname(binary),{recursive:true});
  fs.copyFileSync(path.resolve('scripts/manage.sh'),path.join(root,'scripts/manage.sh'));
  fs.copyFileSync(path.resolve('scripts/spawn-detached.mjs'),path.join(root,'scripts/spawn-detached.mjs'));
  const env={...process.env,FORCE_COLOR:'1',CAUSEWAYBAY_CONFIG_DIR:config,CAUSEWAYBAY_CLIENT_CONFIG_DIR:path.join(root,'client')};
  const run=action=>execFileSync('bash',[path.join(root,'scripts/manage.sh'),action],{env,encoding:'utf8',timeout:15000});
  // A harmless long-running fixture exercises the real process manager without
  // touching the developer's server, credentials, wallet data, or network ports.
  const writeBuild=version=>{
    const staged=binary+'.next';
    fs.writeFileSync(staged,`#!/usr/bin/env node\nconsole.log('Tailwind CSS fixture ${version}');\nsetInterval(()=>{},1000);\nprocess.on('SIGTERM',()=>process.exit(0));\n`,{mode:0o755});
    fs.renameSync(staged,binary);
  };
  t.after(()=>{try {run('stop');} finally {fs.rmSync(root,{recursive:true,force:true});}});
  writeBuild('one');
  assert.match(run('start-server'),/fixture one/);
  const firstPid=fs.readFileSync(pidFile,'utf8');
  assert.match(firstPid,/^\d+\n$/,'PID files stay plain decimal even in a color-enabled terminal');
  const firstBuild=fs.readFileSync(buildFile,'utf8');
  assert.equal(Number(execFileSync('ps',['-p',firstPid.trim(),'-o','pgid='],{encoding:'utf8'}).trim()),Number(firstPid), 'backend has its own process group and outlives the launcher');
  const sentinel=path.join(config,'account-data');
  fs.writeFileSync(sentinel,'keep existing account data');
  assert.match(run('start-server'),/already running with the current build/);
  assert.equal(fs.readFileSync(pidFile,'utf8'),firstPid);
  // A timestamp-only touch must not disconnect a working backend.
  fs.utimesSync(binary,new Date(),new Date());
  assert.match(run('start-server'),/already running with the current build/);
  assert.equal(fs.readFileSync(pidFile,'utf8'),firstPid);
  writeBuild('two');
  assert.match(run('start-server'),/Backend build changed; restarting/);
  const secondPid=fs.readFileSync(pidFile,'utf8');
  assert.notEqual(secondPid,firstPid);
  assert.notEqual(fs.readFileSync(buildFile,'utf8'),firstBuild);
  assert.match(fs.readFileSync(path.join(config,'logs/backend.log'),'utf8'),/fixture two/);
  assert.equal(fs.readFileSync(sentinel,'utf8'),'keep existing account data');
  // Existing installations have no build record: upgrade them once, then settle.
  fs.unlinkSync(buildFile);
  assert.match(run('start-server'),/Backend build changed; restarting/);
  const upgradedPid=fs.readFileSync(pidFile,'utf8');
  assert.notEqual(upgradedPid,secondPid);
  assert.match(run('start-server'),/already running with the current build/);
  assert.equal(fs.readFileSync(pidFile,'utf8'),upgradedPid);
  assert.equal(fs.existsSync(path.join(root,'client/run/client.pid')),false);
});

test('start-client loads updated builds, detaches, and preserves saved client data', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-client-manage-'));
  const client = path.join(root, 'client');
  const binary = path.join(root, 'target/debug/causewaybay');
  const pidFile = path.join(client, 'run/client.pid');
  fs.mkdirSync(path.join(root, 'scripts'), {recursive:true});
  fs.mkdirSync(path.dirname(binary), {recursive:true});
  for (const file of ['manage.sh', 'spawn-detached.mjs']) {
    fs.copyFileSync(path.resolve('scripts', file), path.join(root, 'scripts', file));
  }
  const env = {...process.env, FORCE_COLOR:'1', CAUSEWAYBAY_CONFIG_DIR:path.join(root,'backend'), CAUSEWAYBAY_CLIENT_CONFIG_DIR:client};
  const run = action => execFileSync('bash', [path.join(root,'scripts/manage.sh'), action], {env, encoding:'utf8', timeout:15000});
  const writeBuild = version => {
    fs.writeFileSync(binary+'.next', `#!/usr/bin/env node\n// ${version}\nconsole.log('Connected. Waiting for capture or notes commands from the Rust backend.');\nsetInterval(()=>{},1000);\nprocess.on('SIGTERM',()=>process.exit(0));\n`, {mode:0o755});
    fs.renameSync(binary+'.next', binary);
  };
  t.after(() => { try { run('stop-client'); } finally { fs.rmSync(root, {recursive:true, force:true}); } });
  writeBuild('one');
  assert.match(run('start-client'), /Capture client started/);
  const first = fs.readFileSync(pidFile, 'utf8');
  assert.match(first, /^\d+\n$/);
  assert.equal(Number(execFileSync('ps',['-p',first.trim(),'-o','pgid='],{encoding:'utf8'})), Number(first));
  const saved = path.join(client,'client.jsonl');
  fs.writeFileSync(saved, 'saved connection sentinel\n');
  assert.match(run('start-client'), /already running with the current build/);
  assert.equal(fs.readFileSync(pidFile,'utf8'), first);
  writeBuild('two');
  assert.match(run('start-client'), /build changed; restarting/);
  const second = fs.readFileSync(pidFile,'utf8');
  assert.notEqual(second, first);
  assert.equal(fs.readFileSync(saved,'utf8'), 'saved connection sentinel\n');
  fs.unlinkSync(path.join(client,'run/client.build'));
  assert.match(run('start-client'), /build changed; restarting/);
  assert.notEqual(fs.readFileSync(pidFile,'utf8'), second);
  assert.match(run('start-client'), /already running with the current build/);
  assert.equal(fs.existsSync(path.join(root,'backend/run/server.pid')), false);
});

function startupFixture(t, {server, client}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(),'cwb-startup-status-'));
  const config = path.join(root,'backend'), clientConfig = path.join(root,'client');
  fs.mkdirSync(path.join(root,'scripts'),{recursive:true});
  fs.mkdirSync(path.join(root,'target/debug'),{recursive:true});
  for (const name of ['manage.sh','spawn-detached.mjs']) fs.copyFileSync(path.resolve('scripts',name),path.join(root,'scripts',name));
  for (const [name, body] of [['causewaybay-server',server],['causewaybay',client]]) {
    if (body !== undefined) fs.writeFileSync(path.join(root,'target/debug',name), '#!/usr/bin/env node\n'+body, {mode:0o755});
  }
  const env = {...process.env,CAUSEWAYBAY_CONFIG_DIR:config,CAUSEWAYBAY_CLIENT_CONFIG_DIR:clientConfig};
  const run = action => spawnSync('bash',[path.join(root,'scripts/manage.sh'),action],{env,encoding:'utf8',timeout:20000});
  t.after(()=>{run('stop');fs.rmSync(root,{recursive:true,force:true});});
  return {run, config, clientConfig, env};
}
const keepRunning = "setInterval(()=>{},1000);process.on('SIGTERM',()=>process.exit(0));";

test('combined startup reports connection, permission and URL, including repeat starts', t => {
  const f = startupFixture(t, {
    server:"console.log('Web URL http://127.0.0.1:3000\\nTailwind CSS fixture');"+keepRunning,
    client:"console.log('Capture access: private-owner-sentinel\\nScreen capture permission: granted\\nConnected. Waiting for capture');"+keepRunning,
  });
  for (let i=0;i<2;i++) {
    const result = f.run('start');
    assert.equal(result.status,0,result.stderr);
    assert.match(result.stdout,/Web URL http:\/\/127\.0\.0\.1:3000/);
    assert.match(result.stdout,/Screen capture permission: granted/);
    assert.match(result.stdout,/Capture client connected to backend/);
    assert.doesNotMatch(result.stdout,/private-owner-sentinel/);
  }
});

test('a client that exits immediately makes startup fail', t => {
  const f = startupFixture(t,{client:"console.error('Error: invalid backend URL');process.exit(1);"});
  const result = f.run('start-client');
  assert.equal(result.status,1);
  assert.match(result.stderr,/Capture client exited during startup/);
  assert.match(result.stderr,/invalid backend URL/);
  assert.equal(fs.existsSync(path.join(f.clientConfig,'run/client.pid')),false);
});

test('a disconnected client keeps retrying but startup never calls it connected', t => {
  const f = startupFixture(t,{client:"console.log('Screen capture permission: required on this Mac; press Capture to request access');console.error('Connection failed: backend unavailable');"+keepRunning});
  const result = f.run('start-client');
  assert.equal(result.status,0,result.stderr);
  assert.match(result.stdout,/Screen capture permission: required/);
  assert.match(result.stdout,/not connected yet; it will retry automatically/);
  assert.match(result.stdout,/backend unavailable/);
  assert.doesNotMatch(result.stdout,/Capture client connected to backend/);
});

test('a backend that never becomes ready makes combined startup fail without starting the client', t => {
  const f = startupFixture(t,{server:keepRunning,client:keepRunning});
  const result = f.run('start');
  assert.equal(result.status,1);
  assert.match(result.stderr,/Backend did not become ready/);
  assert.equal(fs.existsSync(path.join(f.clientConfig,'run/client.pid')),false);
});

test('an explicit owner enrolls a running shared client and persists across normal launches', t => {
  const f = startupFixture(t, {client:`
const fs = require('node:fs'), path = require('node:path');
const file = path.join(process.env.CAUSEWAYBAY_CLIENT_CONFIG_DIR, 'client.jsonl');
let config = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file,'utf8').trim().split('\\n').at(-1)).config : {client_id:'same-mac',owner_wallet:null};
const owner = process.argv.indexOf('--owner-wallet');
if (owner !== -1) config.owner_wallet = process.argv[owner+1].toLowerCase();
fs.appendFileSync(file, JSON.stringify({config})+'\\n');
console.log('TextEdit Auto: '+(config.owner_wallet ? 'account enrolled' : 'shared capture only'));
console.log('Connected. Waiting for capture');
${keepRunning}`});
  // Pass environment changes through the fixture instead of real user config.
  assert.equal(f.run('start-client').status, 0);
  const pid = () => fs.readFileSync(path.join(f.clientConfig,'run/client.pid'),'utf8');
  const sharedPid = pid();
  f.env.OWNER_WALLET = '0x1111111111111111111111111111111111111111';
  const result = f.run('start-client');
  assert.equal(result.status,0,result.stderr);
  assert.match(result.stdout,/account changed; restarting/);
  assert.match(result.stdout,/TextEdit Auto: account enrolled/);
  assert.notEqual(pid(),sharedPid);
  const enrolledPid = pid();
  assert.match(f.run('start-client').stdout,/already running/);
  assert.equal(pid(),enrolledPid);
  delete f.env.OWNER_WALLET;
  assert.match(f.run('start-client').stdout,/already running/);
  assert.equal(f.run('stop-client').status,0);
  assert.equal(f.run('start-client').status,0);
  const saved = JSON.parse(fs.readFileSync(path.join(f.clientConfig,'client.jsonl'),'utf8').trim().split('\n').at(-1)).config;
  assert.equal(saved.client_id,'same-mac');
  assert.equal(saved.owner_wallet,'0x1111111111111111111111111111111111111111');
});
