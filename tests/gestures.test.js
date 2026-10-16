import test from 'node:test';
import assert from 'node:assert/strict';
import { gestureAxis } from '../client/src/gestures.js';

test('gestureAxis: distingue eixos com precisão trigonométrica e descarta micro-movimentos', () => {
  // Micro-movimentos abaixo do limite de 8px retornam null
  assert.equal(gestureAxis(0, 0), null);
  assert.equal(gestureAxis(4, 3), null); // hypot = 5
  assert.equal(gestureAxis(7.9, 0), null);
  assert.equal(gestureAxis(0, 7.9), null);
  assert.equal(gestureAxis(-5, -5), null); // hypot = ~7.07

  // Movimento puramente horizontal (dy = 0, dx >= 8)
  assert.equal(gestureAxis(10, 0), 'horizontal');
  assert.equal(gestureAxis(-20, 0), 'horizontal');

  // Movimento puramente vertical (dx = 0, dy >= 8)
  assert.equal(gestureAxis(0, 10), 'vertical');
  assert.equal(gestureAxis(0, -20), 'vertical');

  // Diagonal 45 graus (dx = 10, dy = 10 -> dy/hypot = 10/14.14 = 0.707 >= 0.5) -> vertical
  assert.equal(gestureAxis(10, 10), 'vertical');
  assert.equal(gestureAxis(-10, 10), 'vertical');
  assert.equal(gestureAxis(10, -10), 'vertical');
  assert.equal(gestureAxis(-10, -10), 'vertical');

  // Limiar exato de 30 graus: dy / hypot = 0.5
  // dx = sqrt(3)*dy ~ 1.732 * dy. Se dy = 10, dx = 17.3205 -> hypot = 20 -> 10/20 = 0.5 -> vertical
  assert.equal(gestureAxis(17.32, 10), 'vertical');

  // Abaixo de 30 graus: predominantemente horizontal
  // Se dy = 4, dx = 20 -> hypot = sqrt(416) ~ 20.39 -> 4/20.39 = 0.196 < 0.5 -> horizontal
  assert.equal(gestureAxis(20, 4), 'horizontal');
  assert.equal(gestureAxis(-30, 5), 'horizontal');
  assert.equal(gestureAxis(25, -5), 'horizontal');
});
