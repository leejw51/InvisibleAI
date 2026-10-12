// Motion's vanilla API works alongside Yew without owning its DOM or content.
// Bundled locally: opening the app never contacts a CDN.
import { animate } from 'motion';
import './companion.js';
import './waiting-companion.js';
import './celebrate.js';

const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const seen = new WeakSet();
const running = new Map();
const spring = { type: 'spring', stiffness: 360, damping: 32, mass: 0.7 };
const selector = '.chat-message, .modal, .page-intro';

function move(element, frames, options = spring) {
  if (reduced.matches || !element.isConnected) return;
  running.get(element)?.stop();
  const control = animate(element, frames, options);
  running.set(element, control);
  control.then(() => { if (running.get(element) === control) running.delete(element); });
}
function enter(root) {
  if (!(root instanceof Element)) return;
  const elements = [...(root.matches(selector) ? [root] : []), ...root.querySelectorAll(selector)];
  for (const element of elements) {
    if (seen.has(element)) continue;
    seen.add(element);
    if (element.matches('.chat-placeholder img')) {
      move(element, { opacity: [0, 1], y: [14, 0], rotate: [-6, 0], scale: [0.94, 1] }, { ...spring, stiffness: 180, damping: 16 });
    } else {
      move(element, { opacity: [0, 1], y: [8, 0] });
    }
  }
}
const observer = new MutationObserver(records => {
  for (const record of records) {
    for (const node of record.addedNodes) enter(node);
  }
  for (const [element, control] of running) {
    if (!element.isConnected) { control.stop(); running.delete(element); }
  }
});
observer.observe(document.body, { childList: true, subtree: true });
enter(document.body);

// Native details retain their keyboard and screen-reader behavior.
document.addEventListener('toggle', event => {
  const details = event.target;
  if (details.matches?.('.everyday-workspace, .everyday-settings') && details.open) {
    for (const child of details.children) {
      if (child.tagName !== 'SUMMARY') move(child, { opacity: [0, 1], y: [-4, 0] });
    }
  }
}, true);
let pressed;
document.addEventListener('pointerdown', event => {
  const button = event.target.closest?.('button:not(:disabled)');
  if (button && event.isPrimary) { pressed = button; move(button, { scale: 0.97 }, { duration: 0.08 }); }
}, { passive: true });
function release() { if (pressed) { move(pressed, { scale: 1 }); pressed = undefined; } }
document.addEventListener('pointerup', release, { passive: true });
document.addEventListener('pointercancel', release, { passive: true });
window.addEventListener('blur', release);
reduced.addEventListener('change', () => {
  if (!reduced.matches) return;
  for (const [element, control] of running) {
    control.stop();
    element.style.removeProperty('transform');
    element.style.removeProperty('opacity');
  }
  running.clear();
});
