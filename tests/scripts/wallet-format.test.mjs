import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {easeInOutExpo, digits, money, masked, networkLabel} from '../../frontend/wallet-format.js';

test('easeInOutExpo is the exponential in/out curve shared with tween.rs', () => {
  assert.equal(easeInOutExpo(-1), 0);
  assert.equal(easeInOutExpo(0), 0);
  assert.equal(easeInOutExpo(0.5), 0.5);
  assert.equal(easeInOutExpo(1), 1);
  assert.equal(easeInOutExpo(2), 1);
  // Closed form: 2^(20t-10)/2 below the midpoint, mirrored above it.
  assert.equal(easeInOutExpo(0.25), 2 ** -5 / 2);
  assert.equal(easeInOutExpo(0.75), (2 - 2 ** -5) / 2);
  let previous = 0;
  for (let i = 1; i <= 1000; i++) {
    const t = i / 1000, v = easeInOutExpo(t);
    assert.ok(v >= previous, `monotonic at ${t}`);
    assert.ok(Math.abs(v + easeInOutExpo(1 - t) - 1) < 1e-12, `symmetric at ${t}`);
    previous = v;
  }
  // Slow out of the gate, fast through the middle, soft landing.
  assert.equal(easeInOutExpo(0.1), 2 ** -8 / 2);
  assert.ok(easeInOutExpo(0.1) < 0.002 && easeInOutExpo(0.9) > 0.998);
  // The Rust tween uses the same function: keep the two definitions in step.
  const rust = fs.readFileSync(new URL('../../crates/frontend/src/tween.rs', import.meta.url), 'utf8');
  assert.match(rust, /2f64\.powf\(20\.0 \* t - 10\.0\) \/ 2\.0/);
  assert.match(rust, /\(2\.0 - 2f64\.powf\(-20\.0 \* t \+ 10\.0\)\) \/ 2\.0/);
});

test('money groups thousands, fixes places and never goes scientific', () => {
  assert.equal(money(0), '0.00');
  assert.equal(money('1284.56'), '1,284.56');
  assert.equal(money(52), '52.00');
  assert.equal(money('0.000005'), '0.00');
  assert.equal(money(4.25, 4), '4.2500');
  assert.equal(money(1e21), '1,000,000,000,000,000,000,000.00');
  assert.equal(money('not a number'), '0.00');
  assert.equal(money(undefined), '0.00');
});

test('digits reads balance attributes defensively', () => {
  assert.equal(digits('25.5'), 25.5);
  assert.equal(digits(''), 0);
  assert.equal(digits(null), 0);
  assert.equal(digits('Infinity'), 0);
  assert.equal(digits('NaN'), 0);
  assert.equal(digits('0x10'), 16);
});

test('masked shows only the ends of an address, like a card number', () => {
  const address = '0x9858effd232b4033e47d90003d41ec34ecaeda94';
  const shown = masked(address);
  assert.equal(shown, '0x9858  ••••  ••••  da94');
  // None of the hidden middle survives.
  assert.ok(!shown.includes(address.slice(6, -4)));
  for (let i = 6; i < 38; i += 4) assert.ok(!shown.includes(address.slice(i, i + 5)), `chunk ${i}`);
  // Anything that is not an address — including a 64-hex private key — is never echoed.
  const key = '0x1ab42cc412b618bdea3a599e3c9bae199ebf030895b039e9db1e30dafb12b727';
  for (const value of [key, key.slice(2), 'abandon abandon about', '', undefined, '0x1234']) {
    assert.equal(masked(value), '0x•••• •••• •••• ••••');
  }
});

test('networkLabel names exactly the two supported networks', () => {
  assert.equal(networkLabel('cronos-mainnet'), 'CRONOS MAINNET');
  assert.equal(networkLabel('cronos-testnet'), 'CRONOS TESTNET');
  assert.equal(networkLabel('ethereum'), 'CRONOS TESTNET');
});
