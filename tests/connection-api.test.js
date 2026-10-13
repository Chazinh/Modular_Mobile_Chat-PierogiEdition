import test from 'node:test';
import assert from 'node:assert/strict';
import { api, config } from '../client/src/api.js';

test('API: seleção anterior ao POST, endereço fixo durante SSE e nenhuma repetição automática', async () => {
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };
  const saved = new Map();
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = { getItem: k => saved.get(k) || null, setItem: (k, v) => saved.set(k, v), removeItem: k => saved.delete(k) };
  const local = 'https://pc.example.com:8443', remote = 'https://chat.example.com';
  const calls = [], events = [];
  let fail = false;
  globalThis.fetch = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith('/api/health')) return Response.json({ ok: true, name: 'modularchat', encrypted: true });
    if (fail) throw Error('network lost');
    config.configureConnection({ local: 'https://other.example.com' });
    return new Response('data: {"delta":"Mira: Oi."}\n\ndata: {"done":true}\n\n');
  };
  try {
    config.configureConnection({ local, remote });
    await api.sendMessage('fictional-chat', { content: 'Oi', opId: 'stable-operation' }, e => events.push(e));
    assert.equal(calls.filter(c => c.options.method === 'POST').length, 1);
    const post = calls.find(c => c.options.method === 'POST');
    assert.equal(post.url, `${local}/api/chats/fictional-chat/message`);
    assert.equal(JSON.parse(post.options.body).opId, 'stable-operation');
    assert.equal(post.options.redirect, 'error');
    assert.deepEqual(events, [{ delta: 'Mira: Oi.' }, { done: true }]);
    fail = true;
    config.configureConnection({ local, remote });
    await assert.rejects(api.sendMessage('fictional-chat', { opId: 'second-operation' }, () => {}), /Sem conexão/);
    assert.equal(calls.filter(c => c.options.method === 'POST').length, 2, 'falha não repete POST em outro endpoint');
  } finally {
    config.configureConnection({});
    if (originals.window === undefined) delete globalThis.window; else globalThis.window = originals.window;
    if (originals.storage === undefined) delete globalThis.localStorage; else globalThis.localStorage = originals.storage;
    globalThis.fetch = originals.fetch;
  }
});
