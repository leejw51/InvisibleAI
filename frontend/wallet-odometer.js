// Casino-style rolling numbers. Every digit is a vertical reel of 0-9 that
// spins to its new value with motion.dev, eased exponentially in and out; the
// reels stop one after another (motion's `stagger`), the leftmost first, like
// a slot machine paying out. Digits left of the first change hold still; the
// rest spin at least one full turn, upward when the number grows and downward
// when it shrinks. A touch of motion blur rides the spin.
//
// `plan` is pure (no DOM), so the alignment rules are unit-tested.
import { animate, stagger } from 'motion';
import { easeInOutExpo } from './wallet-format.js';

/** Cycles of 0-9 on a reel: enough for a full extra turn either way. */
export const CYCLES = 3;
const isDigit = c => c >= '0' && c <= '9';
/** The digit at each place value, keyed by its distance from the decimal point. */
function places(text) {
  const [whole, frac = ''] = text.split('.');
  const out = new Map();
  let p = 0;
  for (const c of [...whole].reverse()) if (isDigit(c)) out.set(p++, Number(c));
  [...frac].forEach((c, i) => { if (isDigit(c)) out.set(-(i + 1), Number(c)); });
  return out;
}
const numeric = text => Number(String(text).replace(/[^0-9.]/g, '')) || 0;

/**
 * How each character of `next` moves from `previous`: separators are static;
 * a digit gets its reel positions `from` and `to` (indexes into CYCLES × 10).
 * Digits are matched by place value, so 30.00 → 24.895 lines up on the point.
 */
export function plan(previous, next) {
  const before = places(previous || ''), after = places(next);
  const up = numeric(next) >= numeric(previous || '');
  // The most significant place whose digit changed: everything below it spins.
  let first = -Infinity;
  for (const [p, d] of after) if (before.get(p) !== d && p > first) first = p;
  for (const p of before.keys()) if (!after.has(p) && p > first) first = p;
  const [whole] = next.split('.');
  let place = [...whole].filter(isDigit).length - 1;
  const steps = [];
  let afterPoint = false;
  for (const char of next) {
    if (char === '.') { afterPoint = true; place = -1; steps.push({ char }); continue; }
    if (!isDigit(char)) { steps.push({ char }); continue; }
    const digit = Number(char), old = before.get(place) ?? 0;
    const spin = place <= first && previous !== undefined && previous !== next;
    // Growing numbers roll upward through the reel, shrinking ones downward.
    const from = spin ? (up ? old : old + 10 * (CYCLES - 1)) : digit + 10;
    const to = spin ? (up ? digit + 10 * (CYCLES - 1) : digit) : digit + 10;
    steps.push({ char, digit, place, from, to, spin });
    place -= 1;
  }
  return steps;
}

const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
const offset = index => `translateY(${-(index / (CYCLES * 10)) * 100}%)`;

/**
 * Roll `el` to show `text`. The element keeps its last text, so each call
 * spins from what is on screen. Resolves when every reel has stopped.
 */
export function roll(el, text, { duration = 1.4, gap = 0.07 } = {}) {
  const previous = el.dataset.odometer;
  el.dataset.odometer = text;
  el.setAttribute('aria-label', text);
  const steps = plan(previous, text);
  el.textContent = '';
  const reels = [];
  for (const step of steps) {
    const cell = document.createElement('span');
    if (step.digit === undefined) {
      cell.className = 'odo-sep';
      cell.textContent = step.char;
      el.append(cell);
      continue;
    }
    cell.className = 'odo-digit';
    const reel = document.createElement('span');
    reel.className = 'odo-reel';
    reel.setAttribute('aria-hidden', 'true');
    for (let i = 0; i < CYCLES * 10; i++) {
      const n = document.createElement('span');
      n.textContent = String(i % 10);
      reel.append(n);
    }
    reel.style.transform = offset(step.from);
    cell.append(reel);
    el.append(cell);
    reels.push({ reel, step });
  }
  const moving = reels.filter(r => r.step.spin);
  if (!moving.length || reduced()) {
    for (const { reel, step } of reels) reel.style.transform = offset(step.to);
    return Promise.resolve();
  }
  // Leftmost stops first; each next reel a beat later, like a slot paying out.
  const delay = stagger(gap);
  return Promise.all(moving.map(({ reel, step }, i) => animate(reel, {
    transform: [offset(step.from), offset(step.to)],
    filter: ['blur(0px)', 'blur(1.2px)', 'blur(0px)'],
  }, { duration: duration + i * gap, delay: delay(i, moving.length), ease: easeInOutExpo, filter: { duration: duration + i * gap, times: [0, 0.5, 1], ease: easeInOutExpo } })));
}
