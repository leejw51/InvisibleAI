#!/usr/bin/env node
// Regenerates the wallet card artwork with xAI's Grok image model.
// Usage: XAI_API_KEY=… node scripts/generate-wallet-art.mjs [testnet|mainnet|send-coin…]
// Each prompt lives beside its image in frontend/assets/*.prompt.txt. A bare
// name is a card (wallet-card-<name>); a name starting `send-` is a send-stage
// asset (wallet-<name>). A prompt line `Aspect ratio: 1:1` sets the shape.
import fs from 'node:fs';
import path from 'node:path';

const key = process.env.XAI_API_KEY;
if (!key) { console.error('XAI_API_KEY is not set'); process.exit(1); }
const model = process.env.XAI_IMAGE_MODEL || 'grok-imagine-image-quality';
const assets = path.join(path.dirname(new URL(import.meta.url).pathname), '..', 'frontend', 'assets');
const names = process.argv.slice(2).length ? process.argv.slice(2) : ['testnet', 'mainnet'];

for (const name of names) {
  const stem = name.startsWith('send-') ? `wallet-${name}` : `wallet-card-${name}`;
  const promptFile = path.join(assets, `${stem}.prompt.txt`);
  const prompt = fs.readFileSync(promptFile, 'utf8').split('\n\nGenerated with')[0].trim();
  const aspect = prompt.match(/^Aspect ratio: (\d+:\d+)$/m)?.[1] || '3:2';
  const response = await fetch('https://api.x.ai/v1/images/generations', {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${key}` },
    body: JSON.stringify({ model, prompt, n: 1, response_format: 'b64_json', aspect_ratio: aspect }),
  });
  if (!response.ok) { console.error(name, response.status, await response.text()); process.exit(1); }
  const { data } = await response.json();
  const bytes = Buffer.from(data[0].b64_json, 'base64');
  // The API answers with JPEG today; name the file after what it really is.
  const ext = bytes[0] === 0x89 ? 'png' : 'jpg';
  const out = path.join(assets, `${stem}.${ext}`);
  fs.writeFileSync(out, bytes);
  fs.writeFileSync(promptFile, `${prompt}\n\nGenerated with xAI ${model} on ${new Date().toISOString().slice(0, 10)} by scripts/generate-wallet-art.mjs.\n`);
  console.log('wrote', out);
}
