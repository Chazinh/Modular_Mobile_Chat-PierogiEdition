import test from 'node:test';
import assert from 'node:assert/strict';
import { APP_ICONS, getSelectedAppIcon, applyAppIcon, initAppIcon } from '../client/src/app-icon.js';

test('APP_ICONS: integridade dos ícones configurados', () => {
  assert.ok(Array.isArray(APP_ICONS));
  assert.ok(APP_ICONS.length >= 2);

  for (const icon of APP_ICONS) {
    assert.ok(icon.id, 'id deve existir');
    assert.ok(icon.name, 'name deve existir');
    assert.ok(icon.subtitle, 'subtitle deve existir');
    assert.ok(icon.preview.startsWith('/icons/'), 'preview deve apontar para /icons/');
    assert.ok(icon.tile.startsWith('/icons/'), 'tile deve apontar para /icons/');
  }

  assert.equal(APP_ICONS[0].id, 'pierogi_dark');
  assert.equal(APP_ICONS[1].id, 'pierogi_light');
});

test('getSelectedAppIcon: lê do localStorage ou cai no fallback padrão pierogi_dark', () => {
  const originalStorage = globalThis.localStorage;
  const store = new Map();

  globalThis.localStorage = {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, val) => store.set(key, String(val)),
    removeItem: (key) => store.delete(key),
  };

  try {
    // Vazio -> fallback pierogi_dark
    assert.equal(getSelectedAppIcon(), 'pierogi_dark');

    // Valor válido armazenado
    store.set('modularchat_app_icon', 'pierogi_light');
    assert.equal(getSelectedAppIcon(), 'pierogi_light');

    // Valor desconhecido -> fallback pierogi_dark
    store.set('modularchat_app_icon', 'unknown_invalid_icon');
    assert.equal(getSelectedAppIcon(), 'pierogi_dark');

    // Erro no localStorage -> fallback gracioso
    globalThis.localStorage.getItem = () => { throw new Error('Quota or Access Denied'); };
    assert.equal(getSelectedAppIcon(), 'pierogi_dark');
  } finally {
    if (originalStorage) {
      globalThis.localStorage = originalStorage;
    } else {
      delete globalThis.localStorage;
    }
  }
});

test('applyAppIcon e initAppIcon: atualizam armazenamento e gerenciam link no DOM', async () => {
  const originalStorage = globalThis.localStorage;
  const originalDoc = globalThis.document;

  const store = new Map();
  globalThis.localStorage = {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, val) => store.set(key, String(val)),
    removeItem: (key) => store.delete(key),
  };

  const links = [];
  globalThis.document = {
    head: {
      appendChild(node) {
        links.push(node);
      },
    },
    querySelector(selector) {
      const match = selector.match(/rel='([^']+)'/);
      if (!match) return null;
      return links.find((l) => l.rel === match[1]) || null;
    },
    createElement(tag) {
      return { tagName: tag, rel: '', href: '' };
    },
  };

  try {
    // Aplicação do ícone claro
    const result = await applyAppIcon('pierogi_light');
    assert.equal(result.id, 'pierogi_light');
    assert.equal(result.pending, false);
    assert.equal(store.get('modularchat_app_icon'), 'pierogi_light');

    // Deve ter criado links para 'icon' e 'apple-touch-icon'
    const iconLink = links.find((l) => l.rel === 'icon');
    const appleLink = links.find((l) => l.rel === 'apple-touch-icon');
    assert.ok(iconLink, 'link rel=icon criado');
    assert.ok(appleLink, 'link rel=apple-touch-icon criado');
    assert.equal(iconLink.href, '/icons/pierogi-light-192.png');
    assert.equal(appleLink.href, '/icons/pierogi-light-192.png');

    // Inicialização carrega o selecionado e sincroniza
    const initId = await initAppIcon();
    assert.equal(initId, 'pierogi_light');

    // Aplicação com fallback em ID inexistente
    const fallbackRes = await applyAppIcon('inexistente');
    assert.equal(fallbackRes.id, 'pierogi_dark');
    assert.equal(iconLink.href, '/icons/pierogi-dark-192.png');
  } finally {
    if (originalStorage) {
      globalThis.localStorage = originalStorage;
    } else {
      delete globalThis.localStorage;
    }

    if (originalDoc) {
      globalThis.document = originalDoc;
    } else {
      delete globalThis.document;
    }
  }
});
