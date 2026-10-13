import test from 'node:test';
import assert from 'node:assert/strict';
import { validHex, hexToHsl, hslToHex } from '../client/src/color.js';

test('paleta preserva cores HEX arbitrárias ao alternar entre entrada e controles HSL', () => {
  for (let r = 0; r < 256; r += 17) for (let g = 0; g < 256; g += 17) for (let b = 0; b < 256; b += 17) {
    const hex = '#' + [r, g, b].map(n => n.toString(16).padStart(2, '0')).join('');
    assert.equal(hslToHex(hexToHsl(hex)), hex);
  }
  assert.ok(validHex('#AaBbCc'));
  for (const bad of ['', '#abc', 'red', '#zzzzzz', '#1234567']) assert.equal(validHex(bad), false);
});

test('matiz circular, tons de cinza e extremos não produzem componentes inválidos', () => {
  assert.equal(hslToHex({ h: 360, s: 100, l: 50 }), '#ff0000');
  assert.equal(hslToHex({ h: -120, s: 100, l: 50 }), '#0000ff');
  assert.equal(hslToHex({ h: 20, s: 0, l: 50 }), '#808080');
  assert.equal(hslToHex({ h: 140, s: 100, l: 0 }), '#000000');
  assert.equal(hslToHex({ h: 140, s: 100, l: 100 }), '#ffffff');
});
