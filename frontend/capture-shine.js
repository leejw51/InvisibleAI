// The Capture button's shine: a light sweep while it waits, a soft breathing
// glow, and a ring with sparkles on press. Motion only writes a CSS variable
// and body-level particles, never the children Yew renders.
import { animate } from 'motion';

const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const watched = new WeakSet();
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const ready = button => button.isConnected && !button.disabled && !reduced.matches && button.classList.contains('primary-button');

async function sweep(button) {
  while (button.isConnected) {
    if (ready(button) && document.visibilityState === 'visible') {
      // A light sweep should glide; an exponential curve crosses the middle in a blink.
      await animate(button, { '--shine-x': ['-140%', '140%'] }, { duration: 1.4, ease: [0.45, 0, 0.25, 1] });
      button.style.setProperty('--shine-x', '-140%');
    }
    await sleep(2400 + Math.random() * 900);
  }
}
function breathe(button) {
  const glow = animate(button, { '--capture-glow': [0.18, 0.42, 0.18] }, { duration: 2.8, ease: 'easeInOut', repeat: Infinity });
  const sync = () => (ready(button) ? glow.play() : glow.pause());
  new MutationObserver(sync).observe(button, { attributes: true, attributeFilter: ['disabled', 'class'] });
  reduced.addEventListener('change', sync);
  sync();
}
function burst(button) {
  if (reduced.matches) return;
  const rect = button.getBoundingClientRect();
  const x = rect.left + rect.width / 2, y = rect.top + rect.height / 2;
  const ring = document.createElement('span');
  ring.className = 'capture-ring';
  Object.assign(ring.style, { left: `${rect.left}px`, top: `${rect.top}px`, width: `${rect.width}px`, height: `${rect.height}px` });
  document.body.append(ring);
  animate(ring, { scale: [1, 1.35], opacity: [0.7, 0] }, { duration: 0.6, ease: [0.16, 1, 0.3, 1] }).then(() => ring.remove());
  const count = 12;
  for (let i = 0; i < count; i++) {
    const spark = document.createElement('span');
    spark.className = 'capture-spark';
    spark.textContent = i % 3 ? '' : '✦';
    Object.assign(spark.style, { left: `${x}px`, top: `${y}px` });
    document.body.append(spark);
    const angle = (i / count) * Math.PI * 2 + Math.random() * 0.5;
    const reach = 46 + Math.random() * 34;
    animate(spark,
      { x: [0, Math.cos(angle) * reach], y: [0, Math.sin(angle) * reach - 10], scale: [0.3, 1.1, 0], opacity: [1, 1, 0], rotate: [0, 120 * (Math.random() - 0.5)] },
      { duration: 0.75 + Math.random() * 0.25, ease: [0.16, 1, 0.3, 1] }
    ).then(() => spark.remove());
  }
}
function attach(button) {
  if (watched.has(button)) return;
  watched.add(button);
  button.style.setProperty('--shine-x', '-140%');
  sweep(button);
  breathe(button);
  // pointerdown fires before Yew disables the button for the capture.
  button.addEventListener('pointerdown', event => { if (event.isPrimary && ready(button)) burst(button); });
  button.addEventListener('keydown', event => { if ((event.key === 'Enter' || event.key === ' ') && ready(button)) burst(button); });
}
function scan(root) {
  if (!(root instanceof Element)) return;
  if (root.matches('.dock-capture')) attach(root);
  root.querySelectorAll('.dock-capture').forEach(attach);
}
new MutationObserver(records => { for (const r of records) r.addedNodes.forEach(scan); })
  .observe(document.body, { childList: true, subtree: true });
scan(document.body);
// Enter on an empty message captures too: give it the same burst.
document.addEventListener('keydown', event => {
  const box = event.target;
  if (event.key !== 'Enter' || event.shiftKey || event.isComposing || !box?.matches?.('.chat-dock textarea') || box.value.trim()) return;
  const button = document.querySelector('.dock-capture.primary-button');
  if (button && ready(button)) burst(button);
}, true);
