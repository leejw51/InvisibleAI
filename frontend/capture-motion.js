// The Capture page as a motion graphic: the hero builds in line by line, a
// viewfinder frames the press and flies to the new post, the post scans while
// the Mac captures, the screenshot develops, and the answer settles in. Like
// capture-shine, this writes only transforms, opacity and CSS variables on
// Yew's elements, plus body-level particles it removes itself. Reduced motion
// turns all of it off; the page then renders exactly as Yew made it.
import { animate, stagger } from 'motion';

const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const seen = new WeakSet();
const expo = [0.87, 0, 0.13, 1];
const settle = [0.16, 1, 0.3, 1];
const quick = { duration: 0.55, ease: settle };
const active = () => !reduced.matches && document.visibilityState === 'visible';
// Motion resolves keyframes on a later frame, and on first load that frame can
// come a tenth of a second after Yew paints: the hero would show, vanish and
// fade back in. Hiding a part inline before its fade is queued keeps the first
// painted frame blank; the fade ends by writing opacity 1 over it.
const hide = element => { element.style.opacity = '0'; return element; };

// Hero build-in: the connect strip, each line of the intro, the shortcuts from
// the right, the companion's pill, the console, the placeholder, then the dock
// rises. One run per page instance; a re-render never replays it.
function build(page) {
  // A page still settling (restoring a room) is hidden by the stylesheet
  // until its conversation is known; its build-in starts when that class goes.
  if (seen.has(page) || page.classList.contains('is-settling')) return;
  seen.add(page);
  if (!active()) return;
  const intro = page.querySelector(':scope > .page-intro');
  const lines = [
    ...(intro ? [...intro.querySelectorAll(':scope > div > *'), intro.querySelector(':scope > .secondary-button')] : []),
    page.querySelector('.everyday-workspace > summary'),
    ...page.querySelectorAll('.chat-placeholder > *'),
  ].filter(Boolean);
  lines.forEach((line, i) => animate(hide(line), { opacity: [0, 1], y: [14, 0] }, { ...quick, delay: 0.05 + i * 0.07 }));
  const wide = matchMedia('(min-width: 701px)').matches;
  page.querySelectorAll('.quick-pick').forEach((pick, i) =>
    animate(hide(pick), { opacity: [0, 1], ...(wide ? { x: [12, 0] } : { y: [8, 0] }) }, { ...quick, delay: 0.25 + i * 0.08 }));
  // The companion animates its own figure; only its pill and prop fade in.
  page.querySelectorAll('.companion-state, .companion-prop').forEach((part, i) =>
    animate(hide(part), { opacity: [0, 1] }, { duration: 0.6, delay: 0.45 + i * 0.1 }));
  const dock = page.querySelector('.chat-dock');
  if (dock) animate(hide(dock), { opacity: [0, 1], y: [28, 0] }, { duration: 0.7, ease: settle, delay: 0.3 });
}
function strip(element) {
  if (seen.has(element)) return;
  seen.add(element);
  if (active()) animate(hide(element), { opacity: [0, 1], y: [-10, 0] }, quick);
}

// The press: a shutter flash and a viewfinder that opens on the button, then
// glides to the post the capture creates and lets go.
let finder;
function frame(button) {
  if (!active() || !button.isConnected) return;
  const flash = document.createElement('span');
  flash.className = 'capture-flash';
  document.body.append(flash);
  animate(flash, { opacity: [0, 0.32, 0] }, { duration: 0.5, ease: 'easeOut' }).then(() => flash.remove());
  finder?.remove();
  const box = document.createElement('span');
  box.className = 'capture-viewfinder';
  box.innerHTML = '<i></i><i></i><i></i><i></i>';
  finder = box;
  const r = button.getBoundingClientRect();
  place(box, r);
  document.body.append(box);
  animate(box, { opacity: [0, 1], scale: [0.92, 1.12, 1] }, { duration: 0.45, ease: settle });
  box.dataset.since = String(performance.now());
  box.timer = setTimeout(() => release(box), 2600);
}
function place(box, r) {
  Object.assign(box.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
}
function fly(post) {
  const box = finder;
  if (!box?.isConnected || box.flying) return;
  box.flying = true;
  clearTimeout(box.timer);
  // The post keeps moving while the page eases it into view, so the frame
  // reads the post's rectangle every frame instead of flying to a snapshot.
  // A post can be replaced by its synced copy mid-flight; the newest post
  // from the user is then the one the capture made.
  const target = () => post.isConnected ? post : [...document.querySelectorAll('.chat-page.is-capture .chat-message.from-user')].at(-1);
  const from = box.getBoundingClientRect();
  const mix = (a, b, p) => a + (b - a) * p;
  const follow = p => {
    const current = target();
    if (!current || !box.isConnected) return;
    const to = current.getBoundingClientRect();
    place(box, { left: mix(from.left, to.left, p), top: mix(from.top, to.top, p), width: mix(from.width, to.width, p), height: mix(from.height, to.height, p) });
  };
  animate(0, 1, { duration: 0.55, ease: expo, onUpdate: follow }).then(() => {
    const track = () => { if (box.isConnected) { follow(1); requestAnimationFrame(track); } };
    track();
    release(box, 0.25);
  });
}
function release(box, hold = 0) {
  if (!box.isConnected) return;
  clearTimeout(box.timer);
  animate(box, { opacity: 0 }, { duration: 0.35, delay: hold, ease: 'easeOut' }).then(() => { box.remove(); if (finder === box) finder = null; });
}

// While the Mac captures, the post's stage text rolls in each time it changes.
function roll(text) {
  if (active()) animate(text, { opacity: [0, 1], y: [8, 0] }, { duration: 0.35, ease: settle });
}
// A screenshot that arrives in an existing post develops: a wipe, a focus
// pull and a sheen (the sheen is a CSS variable the stylesheet paints).
function develop(figure) {
  if (!active()) return;
  const image = figure.querySelector('img');
  animate(figure, { clipPath: ['inset(0 0 100% 0 round 10px)', 'inset(0 0 0% 0 round 10px)'] }, { duration: 0.7, ease: expo })
    .then(() => figure.style.removeProperty('clip-path'));
  if (image) animate(image, { filter: ['blur(12px)', 'blur(0px)'], scale: [1.06, 1] }, { duration: 0.9, ease: expo })
    .then(() => { image.style.removeProperty('filter'); image.style.removeProperty('transform'); });
  animate(figure, { '--sheen-x': ['-60%', '160%'] }, { duration: 0.9, delay: 0.35, ease: [0.45, 0, 0.25, 1] })
    .then(() => figure.style.removeProperty('--sheen-x'));
}
// The answer's blocks settle in from the top, one after another.
function settleIn(answer) {
  if (!active()) return;
  // The markdown sits inside one wrapper; the blocks are its children.
  let root = answer;
  while (root.children.length === 1 && root.children[0].children.length > 1) root = root.children[0];
  const blocks = [...root.children].slice(0, 10);
  if (blocks.length) animate(blocks, { opacity: [0, 1], y: [10, 0] }, { duration: 0.45, ease: settle, delay: stagger(0.06) });
}

// Dynamic Island: a capsule at the top of the page that opens while a capture
// runs, reads out the latest post's stage, seals with a green dot and closes.
// It is a body-level element; the stage text is read from the post, never changed.
const island = { node: null, label: null, state: '', timer: 0 };
const islandSpring = { type: 'spring', stiffness: 420, damping: 30, mass: 0.8 };
function islandNode() {
  if (island.node?.isConnected) return island.node;
  const anchor = document.createElement('div');
  anchor.className = 'capture-island-anchor';
  anchor.setAttribute('aria-hidden', 'true');
  island.node = document.createElement('div');
  island.node.className = 'capture-island';
  island.node.innerHTML = '<i></i><span></span>';
  island.label = island.node.lastChild;
  anchor.append(island.node);
  document.body.append(anchor);
  return island.node;
}
function islandShow(text, done) {
  const node = islandNode();
  node.classList.toggle('is-done', done);
  clearTimeout(island.timer);
  if (island.state !== 'open') {
    island.state = 'open';
    island.label.textContent = text;
    node.style.removeProperty('width');
    animate(node, { scale: [0.6, 1], opacity: [0, 1] }, islandSpring);
  } else if (island.label.textContent !== text) {
    const before = node.offsetWidth;
    node.style.removeProperty('width');
    island.label.textContent = text;
    const after = node.offsetWidth;
    animate(node, { width: [before, after] }, { ...islandSpring, stiffness: 380 }).then(() => node.style.removeProperty('width'));
    animate(island.label, { opacity: [0, 1], y: [6, 0] }, { duration: 0.3, ease: settle });
  }
  if (done) island.timer = setTimeout(islandHide, 1500);
}
function islandHide() {
  if (island.state !== 'open' || !island.node) return;
  island.state = 'closing';
  clearTimeout(island.timer);
  animate(island.node, { scale: 0.6, opacity: 0 }, { duration: 0.3, ease: [0.32, 0.72, 0, 1] }).then(() => { if (island.state === 'closing') island.state = ''; });
}
function islandSync() {
  const page = document.querySelector('.chat-page.is-capture');
  if (!page || !active()) return islandHide();
  const post = [...page.querySelectorAll('.chat-message[data-status="capturing"], .chat-message[data-status="running"]')].at(-1);
  const text = post?.querySelector('.chat-waiting-text')?.textContent.trim();
  if (post && text) islandShow(text, false);
  else if (island.state === 'open' && !island.node.classList.contains('is-done')) islandShow('\u2713', true);
}

function scan(node, fresh) {
  if (!(node instanceof Element)) return;
  if (node.matches('.chat-page.is-capture')) build(node);
  node.querySelectorAll?.('.chat-page.is-capture').forEach(build);
  if (node.matches('.connect-strip')) strip(node);
  node.querySelectorAll?.('.connect-strip').forEach(strip);
  if (!node.closest('.chat-page.is-capture')) return;
  // A whole post arriving gets the generic entrance; parts arriving inside an
  // existing post are the capture's progress and get the capture's motion.
  const post = node.closest('.chat-message');
  if (post && node === post && fresh.has(post) && post.matches('.from-user')) fly(post);
  if (node === post || fresh.has(post)) return;
  if (node.matches('.chat-waiting-text')) roll(node);
  node.querySelectorAll?.('.chat-waiting-text').forEach(roll);
  if (node.matches('figure.chat-image')) develop(node);
  node.querySelectorAll?.('figure.chat-image').forEach(develop);
  if (node.matches('.from-ai .answer-content')) settleIn(node);
  node.querySelectorAll?.('.from-ai .answer-content').forEach(settleIn);
}
new MutationObserver(records => {
  const fresh = new Set();
  for (const r of records) for (const n of r.addedNodes) if (n instanceof Element && n.matches('.chat-message')) fresh.add(n);
  for (const r of records) r.addedNodes.forEach(n => scan(n, fresh));
  for (const r of records) {
    // Yew swaps the text node when the stage changes; the span itself stays.
    if (r.type === 'attributes' && r.attributeName === 'class' && r.target.matches('.chat-page.is-capture')) build(r.target);
    if (r.type === 'characterData' || (r.type === 'childList' && r.target instanceof Element && r.target.matches('.chat-waiting-text') && r.target.closest('.chat-page.is-capture'))) {
      const text = r.target instanceof Element ? r.target : r.target.parentElement;
      if (text?.matches?.('.chat-waiting-text') && text.closest('.chat-page.is-capture')) roll(text);
    }
  }
  islandSync();
}).observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['data-status', 'class'] });
scan(document.body, new Set());

const ready = button => button.isConnected && !button.disabled && button.classList.contains('primary-button');
document.addEventListener('pointerdown', event => {
  const button = event.target.closest?.('.dock-capture');
  if (button && event.isPrimary && ready(button)) frame(button);
}, true);
document.addEventListener('keydown', event => {
  if (event.isComposing) return;
  const target = event.target;
  if (target?.matches?.('.dock-capture') && (event.key === 'Enter' || event.key === ' ') && ready(target)) return frame(target);
  if (event.key !== 'Enter' || event.shiftKey || !target?.matches?.('.chat-dock textarea') || target.value.trim()) return;
  const button = document.querySelector('.dock-capture.primary-button');
  if (button && ready(button)) frame(button);
}, true);
reduced.addEventListener('change', () => { if (reduced.matches) { islandHide(); finder?.remove(); finder = null; document.querySelectorAll('.capture-flash').forEach(f => f.remove()); } });
