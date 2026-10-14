import {test} from 'node:test';
import assert from 'node:assert/strict';
import {plan, CYCLES} from '../../frontend/wallet-odometer.js';

const digits = steps => steps.filter(s => s.digit !== undefined);

test('a first appearance shows the number without spinning', () => {
  const steps = plan(undefined, '30.00');
  assert.deepEqual(steps.map(s => s.char).join(''), '30.00');
  assert.ok(digits(steps).every(s => !s.spin && s.from === s.to));
});

test('digits line up on the decimal point, and only those below the first change spin', () => {
  // 5,911.61716 → 5,911.60904: thousands and units hold, the decimals roll.
  const steps = digits(plan('5,911.61716', '5,911.60904'));
  assert.deepEqual(steps.map(s => s.spin), [false, false, false, false, false, true, true, true, true]);
  // Growing: 30.00 → 55.08 rolls upward through the reel.
  const up = digits(plan('30.00', '55.08'));
  for (const s of up) {
    assert.ok(s.spin);
    assert.ok(s.to > s.from, 'upward');
    assert.equal(s.to % 10, s.digit);
    assert.ok(s.to - s.from >= 10, 'at least one full turn');
  }
  // Shrinking: 30.00 → 24.895 rolls downward, matched by place value.
  const down = digits(plan('30.00', '24.895'));
  assert.deepEqual(down.map(s => s.place), [1, 0, -1, -2, -3]);
  for (const s of down) { assert.ok(s.from > s.to, 'downward'); assert.equal(s.to, s.digit); }
  // The old units digit (0) is where the new units reel starts.
  assert.equal(down[1].from, 0 + 10 * (CYCLES - 1));
});

test('separators never spin, and an unchanged number does not move', () => {
  const steps = plan('1,284.56', '1,284.56');
  assert.ok(steps.every(s => !s.spin));
  assert.deepEqual(steps.filter(s => s.digit === undefined).map(s => s.char), [',', '.']);
});
