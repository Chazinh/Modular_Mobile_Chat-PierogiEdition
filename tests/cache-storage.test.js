import test from 'node:test';
import assert from 'node:assert/strict';
import { createCacheWriter } from '../client/src/storage-writes.js';
import { api, config } from '../client/src/api.js';
import { kv } from '../client/src/kv.js';

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
function fixture(idle = false) {
  const scheduled = new Map(), saved = new Map(), writes = [];
  let id = 0;
  const timers = { setTimeout: fn => { scheduled.set(++id, fn); return id; }, clearTimeout: id => scheduled.delete(id) };
  if (idle) { timers.requestIdleCallback = timers.setTimeout; timers.cancelIdleCallback = timers.clearTimeout; }
  const store = { get: async (key, fallback) => saved.get(key) ?? fallback,
    set: async (key, value) => { writes.push([key, value]); saved.set(key, value); }, del: async key => saved.delete(key) };
  return { scheduled, saved, writes, store, cache: createCacheWriter(store, timers) };
}

test('cache: flush manual cancela timeout/idle e devolve a mesma drenagem', async () => {
  for (const idle of [false, true]) {
    const f = fixture(idle);
    f.cache.queue('chat.a', { history: ['completa'] });
    assert.equal(f.scheduled.size, 1);
    const drain = f.cache.flush();
    assert.equal(f.scheduled.size, 0);
    assert.equal(f.cache.flush(), drain);
    await drain;
    assert.equal(f.writes.length, 1);
  }
});

test('cache: storm de 100 flushes retém apenas writer atual e último valor por chave; get vê inflight', async () => {
  const f = fixture(), started = deferred(), gate = deferred();
  const set = f.store.set;
  f.store.set = async (key, value) => { if (!f.writes.length) { started.resolve(); await gate.promise; } await set(key, value); };
  f.cache.queue('chat.a', { revision: 0, history: ['primeira'] });
  const drain = f.cache.flush();
  await started.promise;
  assert.equal((await f.cache.get('chat.a', null)).revision, 0);
  for (let revision = 1; revision <= 100; revision++) {
    f.cache.queue('chat.a', { revision, history: Array(100).fill('histórico preservado') });
    assert.equal(f.cache.flush(), drain);
  }
  assert.equal((await f.cache.get('chat.a', null)).revision, 100);
  assert.equal(f.scheduled.size, 0);
  gate.resolve();
  await drain;
  assert.deepEqual(f.writes.map(([, value]) => value.revision), [0, 100]);
  assert.equal(f.saved.get('chat.a').history.length, 100);
});

test('cache: delete cancela valor pendente, aguarda inflight e impede ressurreição pelo timer/UI', async () => {
  const f = fixture(true), started = deferred(), gate = deferred(), order = [];
  f.store.set = async (key, value) => { started.resolve(); await gate.promise; f.saved.set(key, value); order.push('set'); };
  f.store.del = async key => { f.saved.delete(key); order.push('delete'); };
  f.cache.queue('chat.a', 'antigo');
  const first = f.cache.flush();
  await started.promise;
  f.cache.queue('chat.a', 'pendente');
  const deleted = f.cache.remove('chat.a');
  f.cache.queue('chat.a', 'save obsoleto');
  assert.equal(await f.cache.get('chat.a', null), null);
  gate.resolve();
  await Promise.all([first, deleted]);
  assert.deepEqual(order, ['set', 'delete']);
  assert.equal(f.saved.has('chat.a'), false);
  assert.equal(f.scheduled.size, 0);
});

test('cache: falha isolada não perde outras chaves, não repete indefinidamente e permite retry explícito', async () => {
  const f = fixture(), attempts = [];
  let fail = true;
  f.store.set = async (key, value) => { attempts.push(key); if (key === 'bad' && fail) throw Error('quota'); f.saved.set(key, value); };
  f.cache.queue('bad', 'preservado'); f.cache.queue('good', 'gravado');
  await f.cache.flush();
  assert.deepEqual(attempts, ['bad', 'good']);
  assert.equal(await f.cache.get('bad', null), 'preservado');
  assert.equal(f.saved.get('good'), 'gravado');
  assert.equal(f.scheduled.size, 0);
  fail = false;
  await f.cache.flush();
  assert.equal(f.saved.get('bad'), 'preservado');
});

test('cache: leitura de disco iniciada antes de um save devolve o valor mais recente', async () => {
  const f = fixture(), gate = deferred();
  f.store.get = () => gate.promise;
  const reading = f.cache.get('chat.a', null);
  f.cache.queue('chat.a', 'novo');
  gate.resolve('obsoleto');
  assert.equal(await reading, 'novo');
  await f.cache.flush();
});

function apiFixture(t, native = false) {
  const originals = Object.fromEntries(['window', 'localStorage', 'fetch'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const values = new Map(), calls = [];
  globalThis.window = { Capacitor: { isNativePlatform: () => native }, dispatchEvent() {} };
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, String(value)), removeItem: key => values.delete(key) };
  globalThis.localStorage = storage;
  globalThis.fetch = async (url, options) => { calls.push({ url, options }); return Response.json({ ok: true, name: 'modularchat', encrypted: true }); };
  config.configureConnection({});
  t.after(async () => {
    await api.flushCache();
    config.configureConnection({});
    for (const [key, descriptor] of Object.entries(originals)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globalThis[key];
    }
  });
  return { values, calls, storage };
}

test('settings: GETs concorrentes compartilham destino; epoch invalida cache mesmo em same-origin', async t => {
  apiFixture(t);
  const gate = deferred(), started = deferred(); let count = 0;
  globalThis.fetch = async () => { count++; started.resolve(); return gate.promise; };
  const first = api.globalSettingsCached(), concurrent = api.globalSettingsCached();
  await started.promise;
  gate.resolve(Response.json({ settings: { botChars: 2000 } }));
  assert.ok((await Promise.all([first, concurrent])).every(result => result.settings.botChars === 2000));
  await api.globalSettingsCached();
  assert.equal(count, 1);
  config.configureConnection({ local: 'https://other.example.com' });
  assert.equal(api.peekGlobalSettings(), null, 'same-origin também isola épocas');
  globalThis.fetch = async () => { count++; return Response.json({ settings: { botChars: 3000 } }); };
  assert.equal((await api.globalSettingsCached()).settings.botChars, 3000);
  assert.equal(count, 2);
});

test('settings: primeira seleção PC compartilha GET e aceita resposta do destino recém-selecionado', async t => {
  apiFixture(t, true);
  config.configureConnection({ local: 'https://first-pc.example.com' });
  let settingsGets = 0;
  globalThis.fetch = async url => {
    if (url.endsWith('/api/health')) return Response.json({ ok: true, name: 'modularchat', encrypted: true });
    settingsGets++;
    return Response.json({ settings: { botChars: 2500 } });
  };
  const results = await Promise.all([api.globalSettingsCached(), api.globalSettingsCached()]);
  assert.ok(results.every(result => result.settings.botChars === 2500));
  assert.equal(settingsGets, 1);
  assert.equal(api.peekGlobalSettings().botChars, 2500);
});

test('settings: resposta antiga não contamina novo host/kind, e save invalida GET inclusive se PUT falhar', async t => {
  apiFixture(t, true);
  config.configureConnection({ local: 'https://pc.example.com' });
  await api.health(); // Complete endpoint selection before capturing the settings origin.
  const old = deferred(), started = deferred();
  globalThis.fetch = async () => { started.resolve(); return old.promise; };
  const reading = api.globalSettingsCached();
  const rejected = assert.rejects(reading, error => error.code === 'STALE_CONNECTION' || error.name === 'AbortError');
  await started.promise;
  globalThis.window.Capacitor.isNativePlatform = () => false;
  config.configureConnection({});
  globalThis.fetch = async () => Response.json({ settings: { botChars: 500 } });
  assert.equal((await api.globalSettingsCached()).settings.botChars, 500);
  old.resolve(Response.json({ settings: { botChars: 9000 } }));
  await rejected;
  assert.equal(api.peekGlobalSettings().botChars, 500);
  globalThis.fetch = async (_url, options) => options.method === 'PUT'
    ? Response.json({ error: 'falha fixture' }, { status: 500 }) : Response.json({ settings: { botChars: 600 } });
  await assert.rejects(api.saveGlobalSettings({ botChars: 700 }), /falha fixture/);
  assert.equal(api.peekGlobalSettings(), null);
  assert.equal((await api.globalSettingsCached()).settings.botChars, 600);
});

test('settings: save descarta GET pendente sem permitir que sua resposta repovoe o cache', async t => {
  apiFixture(t);
  const old = deferred(), started = deferred();
  globalThis.fetch = async (_url, options) => { if (options.method === 'GET') { started.resolve(); return old.promise; } return Response.json({ settings: {} }); };
  const reading = api.globalSettingsCached();
  const rejected = assert.rejects(reading, error => error.code === 'STALE_CONNECTION' || error.name === 'AbortError');
  await started.promise;
  await api.saveGlobalSettings({ botChars: 800 });
  old.resolve(Response.json({ settings: { botChars: 1 } }));
  await rejected;
  assert.equal(api.peekGlobalSettings(), null);
});

test('API: token/draft/pending propagam falha; POST espera pendência durável e ack/attach rejeitam falha', async t => {
  const f = apiFixture(t);
  let posts = 0;
  const set = f.storage.setItem;
  f.storage.setItem = () => { throw Error('quota fixture'); };
  await assert.rejects(config.setToken('fixture-token'), /quota fixture/);
  await assert.rejects(api.setDraft('fixture-draft', 'rascunho'), /quota fixture/);
  await assert.rejects(api.setPending('fixture-pending', { opId: 'fixture' }), { code: 'PRIVATE_CACHE_FAILED' });
  globalThis.fetch = async () => { posts++; return new Response('data: {"done":true}\n\n'); };
  await assert.rejects(api.sendMessage('fixture-send', { opId: 'before-post' }, () => {}), { code: 'PRIVATE_CACHE_FAILED' });
  assert.equal(posts, 0);
  for (const attach of [false, true]) {
    let saves = 0;
    const chatId = attach ? 'attach-fixture' : 'ack-fixture';
    f.storage.setItem = set;
    if (attach) await api.setPending(chatId, { opId: 'fixture-op', originServer: '', originKind: 'browser' });
    f.storage.setItem = (key, value) => { if (key === `mc.pending.${chatId}` && ++saves > 1) throw Error('ack quota fixture'); return set(key, value); };
    globalThis.fetch = async () => {
      posts++;
      assert.ok(f.values.has(`mc.pending.${chatId}`), 'pendência já persistida antes do envio');
      return new Response('data: {"ack":{"opId":"fixture-op","generationId":"g-fixture","mode":"append","partId":"p-fixture"}}\n\ndata: {"done":true}\n\n');
    };
    const task = attach ? api.attachGeneration(chatId, 'g-fixture', 0, () => {}) : api.sendMessage(chatId, { opId: 'fixture-op' }, () => {});
    await assert.rejects(task, { code: 'PRIVATE_CACHE_FAILED' });
    assert.ok(api.getPending(chatId));
  }
});

test('API: cache de presets continua melhor-esforço quando kv.set rejeita', async t => {
  apiFixture(t);
  const original = kv.set;
  t.after(() => { kv.set = original; });
  kv.set = async () => { throw Error('quota'); };
  globalThis.fetch = async () => Response.json({ presets: [{ id: 'glm-52' }] });
  assert.equal((await api.presets()).presets[0].id, 'glm-52');
});
