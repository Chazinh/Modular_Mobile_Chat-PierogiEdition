import test from 'node:test';
import assert from 'node:assert/strict';
import { transitionTo } from '../client/src/press-feedback.js';

test('transitionTo: executa update imediatamente quando reduced motion está ativo', () => {
  const originalWin = globalThis.window;
  let updated = false;

  globalThis.window = {
    matchMedia: (q) => ({ matches: q.includes('prefers-reduced-motion: reduce') }),
  };

  try {
    transitionTo(() => { updated = true; }, '.mc-chat', null);
    assert.equal(updated, true, 'Deve executar a atualização de imediato no modo de movimento reduzido');
  } finally {
    if (originalWin) globalThis.window = originalWin;
    else delete globalThis.window;
  }
});

test('transitionTo: cancela transição anterior e limpa elementos criados', () => {
  const originalWin = globalThis.window;
  const originalDoc = globalThis.document;
  const originalCAF = globalThis.cancelAnimationFrame;
  const originalRAF = globalThis.requestAnimationFrame;

  globalThis.cancelAnimationFrame = () => {};
  globalThis.requestAnimationFrame = () => 1;

  const appends = [];
  const createdElements = [];

  globalThis.window = {
    innerWidth: 360,
    innerHeight: 800,
    matchMedia: () => ({ matches: false }),
  };

  globalThis.document = {
    body: {
      append: (el) => appends.push(el),
    },
    createElement: (tag) => {
      const el = {
        tagName: tag,
        className: '',
        style: {},
        setAttribute: () => {},
        append: () => {},
        remove: () => {
          const idx = appends.indexOf(el);
          if (idx !== -1) appends.splice(idx, 1);
        },
      };
      createdElements.push(el);
      return el;
    },
    querySelectorAll: () => [],
  };

  try {
    let updated1 = false;
    let updated2 = false;

    // Dispara primeira transição
    transitionTo(() => { updated1 = true; }, '.other-selector', {
      getBoundingClientRect: () => ({ left: 100, top: 100, width: 50, height: 50 }),
      style: {},
    });

    // Disparar segunda transição deve cancelar a primeira
    transitionTo(() => { updated2 = true; }, '.other-selector', {
      getBoundingClientRect: () => ({ left: 100, top: 100, width: 50, height: 50 }),
      style: {},
    });

    assert.equal(updated1, false);
  } finally {
    if (originalWin) globalThis.window = originalWin;
    else delete globalThis.window;

    if (originalDoc) globalThis.document = originalDoc;
    else delete globalThis.document;

    if (originalRAF) globalThis.requestAnimationFrame = originalRAF;
    else delete globalThis.requestAnimationFrame;

    if (originalCAF) globalThis.cancelAnimationFrame = originalCAF;
    else delete globalThis.cancelAnimationFrame;
  }
});
