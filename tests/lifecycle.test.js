import test from 'node:test';
import assert from 'node:assert/strict';
import React from '../client/node_modules/react/index.js';
import { useResume } from '../client/src/lifecycle.js';

test('useResume: registra eventos de retomada, reage a visibilidade e limpa ouvintes ao desmontar', () => {
  const docListeners = new Map();
  const winListeners = new Map();

  const mockDoc = {
    visibilityState: 'hidden',
    addEventListener: (type, fn) => {
      if (!docListeners.has(type)) docListeners.set(type, new Set());
      docListeners.get(type).add(fn);
    },
    removeEventListener: (type, fn) => {
      docListeners.get(type)?.delete(fn);
    },
    emit: (type) => {
      for (const fn of docListeners.get(type) || []) fn();
    },
  };

  const mockWin = {
    addEventListener: (type, fn) => {
      if (!winListeners.has(type)) winListeners.set(type, new Set());
      winListeners.get(type).add(fn);
    },
    removeEventListener: (type, fn) => {
      winListeners.get(type)?.delete(fn);
    },
    emit: (type) => {
      for (const fn of winListeners.get(type) || []) fn();
    },
  };

  const originalDoc = globalThis.document;
  const originalWin = globalThis.window;
  globalThis.document = mockDoc;
  globalThis.window = mockWin;

  let cleanup = null;
  // Configura despachador do React para executar o efeito
  React.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE.H = {
    useEffect: (fn) => {
      cleanup = fn();
    },
  };

  try {
    let callCount = 0;
    useResume(() => {
      callCount++;
    });

    assert.equal(typeof cleanup, 'function', 'Efeito deve retornar função de cleanup');
    assert.ok(docListeners.get('visibilitychange')?.size === 1);
    assert.ok(winListeners.get('mc:resume')?.size === 1);
    assert.ok(winListeners.get('online')?.size === 1);

    // 1. visibilitychange quando oculto não deve invocar o callback
    mockDoc.visibilityState = 'hidden';
    mockDoc.emit('visibilitychange');
    assert.equal(callCount, 0);

    // 2. visibilitychange quando visível deve invocar o callback
    mockDoc.visibilityState = 'visible';
    mockDoc.emit('visibilitychange');
    assert.equal(callCount, 1);

    // 3. Evento mc:resume na window deve invocar o callback
    mockWin.emit('mc:resume');
    assert.equal(callCount, 2);

    // 4. Evento online na window deve invocar o callback
    mockWin.emit('online');
    assert.equal(callCount, 3);

    // 5. Cleanup deve remover todos os ouvintes
    cleanup();
    assert.equal(docListeners.get('visibilitychange').size, 0);
    assert.equal(winListeners.get('mc:resume').size, 0);
    assert.equal(winListeners.get('online').size, 0);

    // Disparos após cleanup não alteram a contagem
    mockDoc.emit('visibilitychange');
    mockWin.emit('mc:resume');
    mockWin.emit('online');
    assert.equal(callCount, 3);
  } finally {
    if (originalDoc) globalThis.document = originalDoc;
    else delete globalThis.document;

    if (originalWin) globalThis.window = originalWin;
    else delete globalThis.window;
  }
});
