import test from 'node:test';
import assert from 'node:assert/strict';
import { uuid } from '../client/src/api.js';

test('UUID fallback uses secure bytes, RFC version/variant and fails without a CSPRNG', t => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'crypto');
  t.after(() => Object.defineProperty(globalThis, 'crypto', descriptor));
  Object.defineProperty(globalThis, 'crypto', { configurable: true, value: {
    getRandomValues(bytes) { bytes.fill(255); return bytes; },
  } });
  assert.equal(uuid(), 'ffffffff-ffff-4fff-bfff-ffffffffffff');
  Object.defineProperty(globalThis, 'crypto', { configurable: true, value: undefined });
  assert.throws(uuid, /seguro indisponível/);
});
