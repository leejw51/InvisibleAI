// The web download supplies the current wallet; CLI builds can set OWNER_WALLET.
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import {execFileSync} from 'node:child_process';
const [stage, output] = process.argv.slice(2);
if (!stage || !output) throw new Error('Expected staging directory and ZIP output');
const owner = (process.env.OWNER_WALLET || '').trim().toLowerCase();
if (owner && !/^0x[0-9a-f]{40}$/.test(owner)) throw new Error('OWNER_WALLET must be an Ethereum wallet address, never a recovery phrase');
let url = (process.env.PACKAGE_BACKEND_URL || '').trim();
if (!url) {
  for (const command of ['tailscale','/Applications/Tailscale.app/Contents/MacOS/Tailscale']) {
    try {
      const status = JSON.parse(execFileSync(command,['status','--json'],{encoding:'utf8',timeout:4000,stdio:['ignore','pipe','ignore']}));
      if (status.BackendState !== 'Running') continue;
      const ip = (status.TailscaleIPs || status.Self?.TailscaleIPs || []).find(ip=>net.isIP(ip)===4);
      if (ip) { url = `http://${ip}:${(process.env.BIND_ADDR || '0.0.0.0:3000').split(':').at(-1)}`; break; }
    } catch { /* Generic bundle prompts on first launch; Dashboard packages are prefilled. */ }
  }
}
if (url) {
  const parsed=new URL(url), host=parsed.hostname.replace(/^\[|\]$/g,'');
  const parts=host.split('.').map(Number);
  const tailscale = net.isIP(host) === 4 && parts[0]===100 && parts[1]>=64 && parts[1]<=127 || host.toLowerCase().startsWith('fd7a:115c:a1e0:');
  if (!tailscale || !['http:','https:'].includes(parsed.protocol) || parsed.username || parsed.password || parsed.search || parsed.hash || parsed.pathname!=='/') throw new Error('PACKAGE_BACKEND_URL must be an HTTP(S) Tailscale IP URL');
  url=parsed.origin;
}
const quote=value=>`'${value.replaceAll("'", "'\"'\"'")}'`;
const script=fs.readFileSync('scripts/client-go.sh','utf8').replace('@@BACKEND_URL@@',quote(url)).replace('@@OWNER_WALLET@@',quote(owner)).replace('@@TOKEN_REQUIRED@@','true');
fs.mkdirSync(stage,{recursive:true});
fs.copyFileSync('bin/client',path.join(stage,'client'));
fs.chmodSync(path.join(stage,'client'),0o755);
fs.writeFileSync(path.join(stage,'go.sh'),script,{mode:0o755});
fs.chmodSync(path.join(stage,'go.sh'),0o755);
for (const [source,name] of [['packaging/client.env.example','.env.example'],['packaging/QUICK.md','QUICK.md'],['packaging/README.md','README.md']]) {
  fs.copyFileSync(source,path.join(stage,name));
  fs.chmodSync(path.join(stage,name),0o644);
}
// Entries sit in one folder named after the staging directory, so unzip never spills.
// Naming them explicitly keeps stale files in the staging directory out of the archive.
const folder=path.basename(path.resolve(stage));
const entries=['client','go.sh','.env.example','QUICK.md','README.md'].map(file=>`${folder}/${file}`);
// A fresh archive prevents deleted or renamed files from surviving a later build.
const temporary=path.resolve(stage,'client.zip');
fs.rmSync(temporary,{force:true});
execFileSync('/usr/bin/zip',['-q','-X',temporary,...entries],{cwd:path.dirname(path.resolve(stage)),env:{...process.env,COPYFILE_DISABLE:'1'}});
fs.renameSync(temporary,output);
console.log(`Capture client: ${output}`);
if (!owner || !url) console.log('Dashboard downloads fill in the current wallet and Tailscale IP automatically. This CLI bundle prompts for missing values.');
