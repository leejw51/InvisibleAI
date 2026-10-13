// Pure helpers for the wallet card: no DOM, no three.js, so they run under
// `node --test` (tests/scripts/wallet-format.test.mjs) as well as in the page.

// Standard easeInOutExpo; the same curve as crates/frontend/src/tween.rs.
// Every tween in the wallet uses it.
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

// What a receive address looks like on each chain. Anything else — above all a
// 64-hex private key or a phrase — is never echoed, not even in part.
const ADDRESS = [
  /^0x[0-9a-fA-F]{40}$/, // EVM
  /^[1-9A-HJ-NP-Za-km-z]{32,44}$/, // Solana (base58 public key)
  /^addr(_test)?1[02-9ac-hj-np-z]{50,120}$/, // Cardano
  /^mn_addr(_[a-z]+)?1[02-9ac-hj-np-z]{40,120}$/, // Midnight
  /^(ecash|ectest):[qp][02-9ac-hj-np-z]{40,60}$/, // eCash
];
/** An address as a card number: the ends only, the middle never drawn. */
export function masked(address) {
  const text = address || '';
  if (!ADDRESS.some(pattern => pattern.test(text))) return '•••• •••• •••• ••••';
  // The address's own characters, never its shared prefix (`ecash:`, `addr1`, `mn_addr_preview1`).
  const head = text.startsWith('0x') ? text.slice(0, 6)
    : text.includes(':') ? text.split(':')[1].slice(0, 6)
    : /^(addr|mn_addr)/.test(text) ? text.slice(text.lastIndexOf('1') + 1, text.lastIndexOf('1') + 7)
    : text.slice(0, 6);
  return `${head}  ••••  ••••  ${text.slice(-4)}`;
}

const LABELS = {
  'cronos-mainnet': 'CRONOS MAINNET', 'cronos-testnet': 'CRONOS TESTNET',
  'arbitrum-one': 'ARBITRUM ONE', 'arbitrum-sepolia': 'ARBITRUM SEPOLIA',
  'solana-mainnet': 'SOLANA MAINNET', 'solana-devnet': 'SOLANA DEVNET', 'solana-testnet': 'SOLANA TESTNET',
  'cardano-mainnet': 'CARDANO MAINNET', 'cardano-preprod': 'CARDANO PREPROD', 'cardano-preview': 'CARDANO PREVIEW',
  'midnight-preview': 'MIDNIGHT PREVIEW', 'midnight-devnet': 'MIDNIGHT DEVNET',
  'ecash-mainnet': 'ECASH MAINNET', 'ecash-testnet': 'ECASH TESTNET',
};
/** The pill on the card face; an unknown key never claims to be a mainnet. */
export function networkLabel(network) {
  return LABELS[network] || 'UNKNOWN NETWORK';
}

/** The chain a network key belongs to: the part before the first dash. */
export const chainOf = network => String(network || '').split('-')[0];

/**
 * Where each particle of the send burst is at progress `t` (0…1): it leaves
 * the card centre, arcs along a quadratic curve to `to`, and spreads out by
 * `spread` as it goes. Pure, so the arc can be tested without a GPU.
 */
export function flight(t, from, to, spread, seed) {
  const e = easeInOutExpo(Math.min(1, Math.max(0, t)));
  const lift = { x: (from.x + to.x) / 2 + Math.sin(seed * 12.9) * 0.6, y: Math.max(from.y, to.y) + 1.2 + Math.cos(seed * 7.1) * 0.4 };
  const u = 1 - e;
  return {
    x: u * u * from.x + 2 * u * e * lift.x + e * e * to.x + Math.sin(seed * 31.7) * spread * e,
    y: u * u * from.y + 2 * u * e * lift.y + e * e * to.y + Math.cos(seed * 17.3) * spread * e,
    z: Math.sin(seed * 5.3) * spread * e,
  };
}
