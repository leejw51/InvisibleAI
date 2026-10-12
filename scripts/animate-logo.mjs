// Export the generated logo as a gentle, seamless GIF; no runtime app dependency.
// Requires Sharp: npm install --no-save --package-lock=false sharp
// Or set SHARP_MODULE to an existing Sharp package directory.
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const sharp = require(process.env.SHARP_MODULE || 'sharp');
const input = fileURLToPath(new URL('../frontend/assets/logo.png', import.meta.url));
const output = fileURLToPath(new URL('../frontend/assets/logo.gif', import.meta.url));
const width = 768;
const height = 512;
const count = 72;
const delay = 50;
const clear = { r: 0, g: 0, b: 0, alpha: 0 };
const metadata = await sharp(input).metadata();
if (metadata.width !== 1536 || metadata.height !== 1024 || !metadata.hasAlpha) {
  throw new Error('Expected the approved 1536 × 1024 transparent logo.png.');
}

// The generated artwork has a clear boundary at row 812 between mascot and type.
const character = await sharp(input)
  .extract({ left: 0, top: 0, width: 1536, height: 812 })
  .resize({ width: 714 })
  .png().toBuffer();
const title = await sharp(input)
  .extract({ left: 0, top: 812, width: 1536, height: 212 })
  .resize({ width })
  .png().toBuffer();
const frames = [];
for (let index = 0; index < count; index += 1) {
  const phase = index / count * Math.PI * 2;
  const angle = 2.2 * Math.sin(phase);
  const { data: figure, info } = await sharp(character)
    .rotate(angle, { background: clear })
    .png().toBuffer({ resolveWithObject: true });
  const frame = await sharp({ create: { width, height, channels: 4, background: clear } })
    .composite([
      {
        input: figure,
        left: Math.round((width - info.width) / 2 + 3 * Math.sin(phase)),
        top: Math.round(205 - info.height / 2 - 9 * Math.cos(phase)),
      },
      { input: title, left: 0, top: 406 },
    ])
    .raw().toBuffer();
  frames.push(frame);
}
await sharp(Buffer.concat(frames), {
  raw: { width, height: height * count, channels: 4, pageHeight: height },
}).gif({
  loop: 0,
  delay: Array(count).fill(delay),
  colours: 256,
  effort: 7,
  dither: 0.3,
  interFrameMaxError: 0,
  // Reuse the first frame's palette so the fixed title does not change colour.
  interPaletteMaxError: 256,
}).toFile(output);
console.log(`Saved ${output}: ${width} × ${height}, ${count} frames, ${count * delay / 1000}s loop.`);
