import test from 'node:test';
import assert from 'node:assert/strict';
import { notifyProviderCatalog, watchProviderCatalog, PROVIDER_CATALOG_CHANGED } from '../client/src/provider-catalog.js';

test('notifyProviderCatalog: despacha evento PROVIDER_CATALOG_CHANGED na window', () => {
  const originalWindow = globalThis.window;
  const events = [];

  globalThis.window = {
    dispatchEvent: (ev) => events.push(ev.type),
  };

  try {
    notifyProviderCatalog();
    assert.equal(events.length, 1);
    assert.equal(events[0], PROVIDER_CATALOG_CHANGED);
  } finally {
    if (originalWindow) globalThis.window = originalWindow;
    else delete globalThis.window;
  }
});

test('watchProviderCatalog: entrega presets em cache e atualiza com presets remotos e eventos de rede', async () => {
  const listeners = new Map();
  const mockWin = {
    addEventListener: (type, fn) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type).add(fn);
    },
    removeEventListener: (type, fn) => {
      listeners.get(type)?.delete(fn);
    },
    emit: (type) => {
      for (const fn of listeners.get(type) || []) fn();
    },
  };

  let epoch = 1;
  const delivered = [];

  let resolvePresets;
  const presetsPromise = new Promise((r) => { resolvePresets = r; });

  const stop = watchProviderCatalog({
    window: mockWin,
    snapshot: () => epoch,
    getCached: async () => [{ id: 'preset-cache' }],
    getPresets: async () => {
      await presetsPromise;
      return [{ id: 'preset-remote' }];
    },
    onPresets: (val) => delivered.push(val),
  });

  // Aguarda a resolução da microtask do getCached
  await new Promise((r) => setTimeout(r, 10));

  // O cache deve ter sido entregue primeiro
  assert.equal(delivered.length, 1);
  assert.deepEqual(delivered[0], [{ id: 'preset-cache' }]);

  // Resolve a resposta remota
  resolvePresets();
  await new Promise((r) => setTimeout(r, 10));

  // Os presets remotos devem ter sido entregues
  assert.equal(delivered.length, 2);
  assert.deepEqual(delivered[1], [{ id: 'preset-remote' }]);

  // Verifica que os listeners foram registrados
  assert.ok(listeners.get(PROVIDER_CATALOG_CHANGED).size > 0);
  assert.ok(listeners.get('mc:connection-change').size > 0);

  // Encerra e confirma limpeza dos listeners
  stop();
  assert.equal(listeners.get(PROVIDER_CATALOG_CHANGED).size, 0);
  assert.equal(listeners.get('mc:connection-change').size, 0);
});
