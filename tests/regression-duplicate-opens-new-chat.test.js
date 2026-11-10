import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { api } from '../client/src/api.js';

const read = file => fs.readFileSync(new URL(`../client/src/${file}`, import.meta.url), 'utf8');

test('regressão: duplicar devolve a conversa nova (id próprio, histórico vazio)', async () => {
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };
  const calls = [], values = new Map();
  globalThis.window = { dispatchEvent() {} };
  globalThis.localStorage = { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) };
  globalThis.fetch = async (url, options) => {
    calls.push({ url, method: options.method });
    return Response.json({ id: 'copy-0000-0000-0000-000000000000', name: 'Mira (cópia)', presetId: 'deepseek-v41-flash' }, { status: 201 });
  };
  try {
    const copy = await api.duplicateChat('orig-0000-0000-0000-000000000000');
    assert.equal(copy.id, 'copy-0000-0000-0000-000000000000');
    assert.deepEqual(copy.history, []);
    assert.deepEqual(calls.map(c => [c.method, c.url]), [['POST', '/api/chats/orig-0000-0000-0000-000000000000/duplicate']]);
  } finally {
    if (originals.window === undefined) delete globalThis.window; else globalThis.window = originals.window;
    if (originals.storage === undefined) delete globalThis.localStorage; else globalThis.localStorage = originals.storage;
    globalThis.fetch = originals.fetch;
  }
});

test('regressão: o botão duplicar abre a cópia em vez de continuar no chat antigo', () => {
  const drawer = read('components/SettingsDrawer.jsx');
  const handler = drawer.slice(drawer.indexOf('api.duplicateChat(chat.id)') - 200, drawer.indexOf('api.duplicateChat(chat.id)') + 200);
  assert.match(handler, /const copy = await api\.duplicateChat\(chat\.id\)/);
  assert.match(handler, /onOpenChat\(copy\)/);
  assert.doesNotMatch(handler, /onHistoryCleared\(\)/, 'recarregar o chat atual mantinha o usuário no chat antigo');
  assert.match(read('components/ChatView.jsx'), /onOpenChat=\{onOpenChat\}/);
  const app = read('App.jsx');
  assert.match(app, /onOpenChat=\{openCreated\}/);
  assert.match(app, /const openCreated = useEvent\(\(chat\) => \{[\s\S]*?setOpenId\(chat\.id\)/);
});
