import test from 'node:test';
import assert from 'node:assert/strict';
import { api, config, createApiTransport } from '../client/src/api.js';
import { createServerConnection } from '../client/src/server-connection.js';

test('pareamento: chamadas públicas não enviam token antigo e operações protegidas continuam autenticadas', async () => {
  const calls = [];
  const transport = createApiTransport({ acquire: async () => ({ server: 'https://pc.example.com',
    token: 'revoked-device', kind: 'pc', release() {} }), auth() {},
    fetch: async (url, options) => { calls.push({ url, options }); return Response.json({ ok: true }); },
  });
  for (const path of ['/api/health', '/api/pair/info', '/api/pair']) {
    await transport.request(path === '/api/pair' ? 'POST' : 'GET', path,
      path === '/api/pair' ? { code: '123456' } : undefined, { unauthenticated: true });
    assert.equal(calls.at(-1).options.headers.Authorization, undefined);
  }
  await transport.request('GET', '/api/devices');
  assert.equal(calls.at(-1).options.headers.Authorization, 'Bearer revoked-device');
});

test('API pair: substitui credencial revogada somente depois de aceitar o código no PC', async () => {
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };
  const values = new Map([['mc.token', 'revoked-device']]);
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = { getItem: key => values.get(key) || null,
    setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const calls = [];
  globalThis.fetch = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith('/api/health')) return Response.json({ ok: true, name: 'modularchat', encrypted: true });
    assert.equal(url, 'https://pc.example.com/api/pair');
    assert.equal(options.headers.Authorization, undefined);
    assert.deepEqual(JSON.parse(options.body), { code: '123456', deviceName: 'Phone' });
    assert.equal(config.token, 'revoked-device');
    return Response.json({ token: 'new-device', deviceId: 'phone' }, { status: 201 });
  };
  try {
    config.configureConnection({ local: 'https://pc.example.com' });
    await api.pair('123456', 'Phone');
    assert.equal(config.token, 'new-device');
    assert.equal(calls.filter(call => call.url.endsWith('/api/pair')).length, 1);
  } finally {
    config.configureConnection({});
    if (originals.window === undefined) delete globalThis.window; else globalThis.window = originals.window;
    if (originals.storage === undefined) delete globalThis.localStorage; else globalThis.localStorage = originals.storage;
    globalThis.fetch = originals.fetch;
  }
});

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
    await api.setPending('fictional-chat', null);
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

test('transport: destino+token capturados antes do await/fetch; body segura lease', async () => {
  let selected = { server: 'https://pc.example.com', token: 'pc-credential', kind: 'pc' }, releaseBody;
  const calls = [], releases = [];
  const transport = createApiTransport({
    acquire: async () => { const target = selected; return { ...target, release: () => releases.push(target.server) }; },
    fetch: async (url, options) => {
      calls.push({ url, options });
      selected = { server: 'http://127.0.0.1:3011', token: 'keystore-only', kind: 'local' };
      return { ok: true, json: () => new Promise(r => { releaseBody = () => r({ ok: true }); }) };
    },
  });
  const write = transport.request('POST', '/api/chats', { name: 'novo' });
  while (!releaseBody) await Promise.resolve();
  assert.equal(calls[0].url, 'https://pc.example.com/api/chats');
  assert.equal(calls[0].options.headers.Authorization, 'Bearer pc-credential');
  assert.equal(releases.length, 0);
  releaseBody();
  await write;
  assert.deepEqual(releases, ['https://pc.example.com']);
});

test('transport: GET pode recuperar uma vez; POST/PUT/DELETE nunca repetem após erro de rede', async () => {
  for (const method of ['GET', 'POST', 'PUT', 'DELETE']) {
    let acquires = 0, sends = 0, releases = 0;
    const transport = createApiTransport({
      acquire: async () => ({ server: ++acquires === 1 ? 'https://pc.example.com' : 'http://127.0.0.1:3011',
        token: acquires === 1 ? 'pc-token' : 'local-token', kind: acquires === 1 ? 'pc' : 'local', release: () => releases++ }),
      failed() {}, auth() {}, fetch: async () => { if (++sends === 1) throw Error('lost'); return Response.json({ ok: true }); },
    });
    if (method === 'GET') assert.deepEqual(await transport.request(method, '/api/chats'), { ok: true });
    else await assert.rejects(transport.request(method, '/api/chats', { opId: 'stable' }), /Sem conexão/);
    assert.equal(sends, method === 'GET' ? 2 : 1);
    assert.equal(releases, sends);
  }
});

test('transport: SSE permanece na origem e lease bloqueia promoção durante consumo', async () => {
  const values = new Map(), calls = [], host = { server: 'http://127.0.0.1:3011', token: 'local-only', kind: 'local' };
  let online = false, promoted = 0, controller;
  const connection = createServerConnection({ mode: 'auto', native: () => true,
    storage: { getItem: k => values.get(k), setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) },
    token: () => 'pc-only', embedded: { target: () => host, activate: async () => {}, standby: async () => {} },
    sync: async () => { promoted++; return { converged: true }; },
    fetch: async () => { if (!online) throw Error('offline'); return Response.json({ ok: true, name: 'modularchat', encrypted: true }); },
  });
  connection.configure({ local: 'https://pc.example.com' });
  const transport = createApiTransport({ acquire: options => connection.acquireTarget(options), fetch: async (url, options) => {
    calls.push({ url, options });
    return new Response(new ReadableStream({ start(c) { controller = c; } }));
  } });
  const events = [];
  const sending = transport.stream('POST', '/api/message', { opId: 'saved' }, e => events.push(e), undefined, { generation: true });
  while (!controller) await Promise.resolve();
  online = true;
  assert.equal((await connection.resolveTarget(undefined, { fresh: true })).kind, 'local');
  assert.equal(promoted, 0);
  controller.enqueue(new TextEncoder().encode('data: {"done":true}\n\n'));
  controller.close();
  await sending;
  assert.equal(calls[0].url, `${host.server}/api/message`);
  assert.equal(calls[0].options.headers.Authorization, 'Bearer local-only');
  assert.equal((await connection.resolveTarget(undefined, { fresh: true })).kind, 'pc');
  assert.equal(promoted, 1);
});

test('API: pendência salva antes do POST, recuperação/stop presos à origem e replay recusado', async () => {
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };
  const values = new Map(), calls = [];
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = { getItem: k => values.get(k) || null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) };
  globalThis.fetch = async (url, options) => {
    calls.push({ url, options });
    if (url.endsWith('/api/health')) return Response.json({ ok: true, name: 'modularchat', encrypted: true });
    if (url.endsWith('/message')) {
      assert.equal(api.getPending('guard').originServer, 'https://pc.example.com');
      throw Error('recebimento incerto');
    }
    if (url.includes('/stream?')) return new Response('data: {"ack":{"opId":"guard-op","generationId":"g1"}}\n\ndata: {"done":true}\n\n');
    return Response.json({ history: [], generation: null });
  };
  try {
    config.configureConnection({ local: 'https://pc.example.com', remote: 'https://remote.example.com' });
    config.token = 'pc-only';
    await assert.rejects(api.sendMessage('guard', { opId: 'guard-op', content: 'Oi' }, () => {}), /Sem conexão/);
    assert.deepEqual({ ...api.getPending('guard') }, { opId: 'guard-op', content: 'Oi', originServer: 'https://pc.example.com', originKind: 'pc' });
    await assert.rejects(api.sendMessage('guard', { opId: 'guard-op', content: 'Oi' }, () => {}), { code: 'AMBIGUOUS_WRITE' });
    await assert.rejects(api.sendMessage('guard', { opId: 'different-op', content: 'novo' }, () => {}), { code: 'AMBIGUOUS_WRITE' });
    await assert.rejects(api.queryOperation('guard'), { code: 'AMBIGUOUS_WRITE' });
    assert.equal(calls.filter(c => c.url.endsWith('/message')).length, 1);
    await api.setPending('guard', { ...api.getPending('guard'), generationId: 'g1' });
    await api.attachGeneration('guard', 'g1', 0, () => {});
    await api.stopGeneration('guard', 'g1');
    assert.ok(calls.filter(c => c.url.includes('/generations/')).every(c => c.url.startsWith('https://pc.example.com/')));
    config.configureConnection({ local: 'https://new-pc.example.com' });
    await assert.rejects(api.attachGeneration('guard', 'g1', 0, () => {}), { code: 'ORIGIN_CHANGED' });
  } finally {
    config.configureConnection({});
    if (originals.window === undefined) delete globalThis.window; else globalThis.window = originals.window;
    if (originals.storage === undefined) delete globalThis.localStorage; else globalThis.localStorage = originals.storage;
    globalThis.fetch = originals.fetch;
  }
});

test('auth: UNPAIRED local/PC informa origem e preserva credencial PC independente', async () => {
  const originals = { window: globalThis.window, storage: globalThis.localStorage };
  const saved = new Map(), events = [];
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent: e => events.push(e) };
  globalThis.localStorage = { getItem: k => saved.get(k) || null, setItem: (k, v) => saved.set(k, v), removeItem: k => saved.delete(k) };
  try {
    config.token = 'pc-preserved';
    for (const kind of ['local', 'pc']) {
      const transport = createApiTransport({
        acquire: async () => ({ server: kind === 'local' ? 'http://127.0.0.1:3011' : 'https://pc.example.com', token: kind === 'local' ? 'keystore-only' : config.token, kind, release() {} }),
        fetch: async (_url, options) => {
          assert.equal(options.headers.Authorization, `Bearer ${kind === 'local' ? 'keystore-only' : 'pc-preserved'}`);
          return Response.json({ code: 'UNPAIRED' }, { status: 401 });
        },
      });
      await assert.rejects(transport.request('GET', '/api/devices'), { code: 'UNPAIRED' });
      assert.equal(events.at(-1).detail.kind, kind);
      assert.equal(config.token, 'pc-preserved');
    }
  } finally {
    if (originals.window === undefined) delete globalThis.window; else globalThis.window = originals.window;
    if (originals.storage === undefined) delete globalThis.localStorage; else globalThis.localStorage = originals.storage;
  }
});
