import { animate } from 'motion';

// When a capture's problem is solved, the companion pops in like a game:
// a jump, a burst of stars, the answer, and this visit's streak. The chat
// marks only replies that finished while it watched, and the check that the
// answer is right happens in Rust, never here.
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const played = new Set();
let layer;

function stage() {
  if (layer?.isConnected) return layer;
  layer = document.createElement('div');
  layer.className = 'solve-party';
  layer.setAttribute('role', 'status');
  layer.setAttribute('aria-live', 'polite');
  document.body.append(layer);
  return layer;
}

function celebrate(message) {
  const id = message.dataset.messageId;
  if (!id || played.has(id)) return;
  played.add(id);
  const streak = Number(message.dataset.celebrate) || 1;
  const root = stage();
  root.replaceChildren();
  const card = document.createElement('div');
  card.className = 'solve-card';
  const buddy = new Image();
  buddy.src = '/assets/everyday-companion.png';
  buddy.alt = '';
  buddy.className = 'solve-buddy';
  const title = document.createElement('strong');
  title.textContent = message.dataset.celebrateTitle || 'Solved it!';
  const detail = document.createElement('span');
  detail.className = 'solve-detail';
  detail.textContent = message.dataset.celebrateDetail || '';
  const score = document.createElement('span');
  score.className = 'solve-score';
  score.textContent = streak > 1 ? `+1 · ${message.dataset.celebrateStreak || 'Streak'} ×${streak}` : '+1';
  card.append(buddy, title, detail, score);
  root.append(card);
  if (reduced.matches) {
    setTimeout(() => card.remove(), 2600);
    return;
  }
  const stars = [];
  const count = Math.min(10 + streak * 2, 22);
  for (let i = 0; i < count; i++) {
    const star = document.createElement('span');
    star.className = 'solve-star';
    star.textContent = i % 3 ? '✦' : '★';
    star.setAttribute('aria-hidden', 'true');
    root.append(star);
    stars.push(star);
  }
  animate(card, { opacity: [0, 1, 1, 0], scale: [0.6, 1.08, 1, 0.9], y: [40, 0, 0, -20] },
    { duration: 2.6, times: [0, 0.18, 0.82, 1], ease: 'easeOut' }).then(() => card.remove());
  animate(buddy, { y: [0, -46, 0, -18, 0], rotate: [0, -14, 10, -4, 0], scale: [0.8, 1.15, 0.95, 1.05, 1] },
    { duration: 1.1, delay: 0.15, ease: 'easeInOut' });
  animate(score, { scale: [0, 1.35, 1], opacity: [0, 1, 1] }, { duration: 0.6, delay: 0.45, ease: 'easeOut' });
  stars.forEach((star, i) => {
    const angle = (Math.PI * 2 * i) / count + Math.random() * 0.4;
    const distance = 110 + Math.random() * 90;
    animate(star, {
      x: [0, Math.cos(angle) * distance], y: [0, Math.sin(angle) * distance - 40],
      scale: [0.2, 1.3, 0.6], opacity: [1, 1, 0], rotate: [0, 180 + Math.random() * 180],
    }, { duration: 1.2 + Math.random() * 0.5, delay: 0.2, ease: 'easeOut' }).then(() => star.remove());
  });
  // The page's own companion cheers too, when it is on screen.
  document.querySelector('invisible-companion')?.greet?.();
}

function scan() {
  for (const message of document.querySelectorAll('.chat-message[data-celebrate]')) celebrate(message);
}
new MutationObserver(scan).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-celebrate'] });
scan();
