import test from 'node:test';
import assert from 'node:assert/strict';
import { installElastic } from '../client/src/elastic.js';

test('installElastic: respeita guardas de reduced motion, document e previne instalação duplicada', () => {
  const originalDoc = globalThis.document;
  const originalWin = globalThis.window;
  const originalMatchMedia = globalThis.matchMedia;

  try {
    // 1. Sem document -> retorno imediato
    delete globalThis.document;
    delete globalThis.window;
    installElastic();
    assert.equal(globalThis.window?.__mcElastic, undefined);

    // 2. Com reduced motion ativo -> não instala
    globalThis.window = {};
    globalThis.document = { addEventListener: () => {} };
    globalThis.matchMedia = (query) => ({
      matches: query.includes('prefers-reduced-motion: reduce'),
    });

    installElastic();
    assert.equal(globalThis.window.__mcElastic, undefined);

    // 3. Com reduced motion inativo -> instala e adiciona ouvintes
    globalThis.matchMedia = () => ({ matches: false });
    const addedListeners = [];
    globalThis.document.addEventListener = (type, handler, options) => {
      addedListeners.push({ type, handler, options });
    };

    installElastic();
    assert.equal(globalThis.window.__mcElastic, true);
    assert.ok(addedListeners.some((l) => l.type === 'touchstart'));
    assert.ok(addedListeners.some((l) => l.type === 'touchmove'));
    assert.ok(addedListeners.some((l) => l.type === 'touchend'));
    assert.ok(addedListeners.some((l) => l.type === 'scroll'));

    // 4. Chamada subsequente -> guardada pelo flag __mcElastic sem duplicar ouvintes
    const countBefore = addedListeners.length;
    installElastic();
    assert.equal(addedListeners.length, countBefore, 'Não deve duplicar ouvintes');
  } finally {
    if (originalDoc) globalThis.document = originalDoc;
    else delete globalThis.document;

    if (originalWin) globalThis.window = originalWin;
    else delete globalThis.window;

    if (originalMatchMedia) globalThis.matchMedia = originalMatchMedia;
    else delete globalThis.matchMedia;
  }
});
