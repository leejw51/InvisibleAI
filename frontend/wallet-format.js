// Pure helpers for the wallet card: no DOM, no three.js, so they run under
// `node --test` (tests/scripts/wallet-format.test.mjs) as well as in the page.

// Standard easeInOutExpo; the same curve as crates/frontend/src/tween.rs.
export const easeInOutExpo = t => t <= 0 ? 0 : t >= 1 ? 1 : t < 0.5 ? 2 ** (20 * t - 10) / 2 : (2 - 2 ** (-20 * t + 10)) / 2;

/** A balance attribute as a number; anything unreadable is 0. */
export function digits(value) {
  const n = Number(value);
  return Number.isFinite(n) ? n : 0;
}

/** 1284.56 → "1,284.56": grouped, fixed places, never scientific. */
export function money(value, places = 2) {
  return digits(value).toLocaleString('en-US', { minimumFractionDigits: places, maximumFractionDigits: places });
}

/** An address as a card number: the ends only, the middle never drawn. */
export function masked(address) {
  if (!/^0x[0-9a-fA-F]{40}$/.test(address || '')) return '0x•••• •••• •••• ••••';
  return `${address.slice(0, 6)}  ••••  ••••  ${address.slice(-4)}`;
}

/** The pill on the card face. */
export function networkLabel(network) {
  return network === 'cronos-mainnet' ? 'CRONOS MAINNET' : 'CRONOS TESTNET';
}
