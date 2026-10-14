import test from 'node:test';
import assert from 'node:assert/strict';
import { createServerConnection, normalizeServer } from '../client/src/server-connection.js';
import { createLocalHost } from '../client/src/local-host.js';

const local = 'https://pc.example.com:8443', remote = 'https://chat.example.com';
function fixture(overrides = {}) {
  const data = new Map(), calls = [], changes = [];
  const storage = { getItem: key => data.get(key), setItem: (key, value) => data.set(key, value), removeItem: key => data.delete(key) };
  const fetch = async (url, options) => {
    calls.push({ url, options });
    if (overrides.fetch) return overrides.fetch(url, options);
    return Response.json({ ok: true, name: 'modularchat', encrypted: true });
  };
  const connection = createServerConnection({ storage, native: () => true, mode: 'legacy', token: () => 'fixture-token', fetch, onChange: value => changes.push(value), ...overrides, fetch });
  return { connection, data, calls, changes };
}

test('conexão: HTTPS sem credenciais ou caminhos; nenhum host pessoal embutido', () => {
  assert.equal(normalizeServer(' https://chat.example.com/ '), remote);
  for (const value of ['http://pc.example.com', 'https://user:pass@chat.example.com', `${remote}/api`, `${remote}?token=x`, `${remote}#x`, 'file:///tmp']) {
    assert.throws(() => normalizeServer(value));
  }
  const f = fixture();
  assert.deepEqual(f.connection.configured(), { local: '', remote: '' });
  assert.equal(f.connection.hasServer(), false);
});

const embeddedTarget = { server: 'http://127.0.0.1:3011', token: 'keystore-only', kind: 'local' };
function autoFixture(overrides = {}) {
  const lifecycle = [], statuses = [];
  const embedded = { target: () => embeddedTarget, activate: async () => lifecycle.push('activate'), standby: async () => lifecycle.push('standby') };
  const f = fixture({ mode: 'auto', embedded, sync: async () => ({ converged: true }), onStatus: s => statuses.push(s), ...overrides });
  return { ...f, lifecycle, statuses };
}

test('host: plugin nativo disponível, início serial/idempotente e credencial apenas em memória', async () => {
  let starts = 0, release;
  const calls = [];
  const host = createLocalHost({ native: () => true, available: () => true,
    plugin: { ensureStarted: () => { starts++; return new Promise(r => { release = r; }); } },
    fetch: async (url, options) => { calls.push({ url, options }); return Response.json({ ok: true }); } });
  const first = host.ensureStarted(), second = host.ensureStarted();
  release({ baseUrl: embeddedTarget.server, status: 2, token: embeddedTarget.token });
  assert.deepEqual(await first, embeddedTarget);
  assert.deepEqual(await second, embeddedTarget);
  await host.ensureStarted();
  assert.equal(starts, 1);
  await Promise.all([host.standby(), host.standby()]);
  assert.equal(calls.length, 1, 'standby não chama activate/download');
  await Promise.all([host.activate(), host.activate()]);
  await host.status();
  assert.equal(calls.length, 3);
  for (const call of calls) {
    assert.equal(call.options.headers.Authorization, 'Bearer keystore-only');
    assert.equal(call.options.redirect, 'error');
  }
  host.invalidateLifecycle();
  await host.activate();
  assert.equal(calls.length, 4, 'resume reconcilia standby do serviço reiniciado');
});

test('host: browser/plugin ausente não invoca JNI; falha JNI e ABI inválida são explícitas', async () => {
  for (const [native, available] of [[false, true], [true, false]]) {
    const host = createLocalHost({ native: () => native, available: () => available, plugin: { ensureStarted: () => assert.fail('JNI indevido') } });
    assert.equal(await host.ensureStarted(), null);
  }
  const host = createLocalHost({ native: () => true, available: () => true, plugin: { ensureStarted: async () => { throw Object.assign(Error('JNI indisponível'), { code: 'HOST_FAILED' }); } } });
  await assert.rejects(host.ensureStarted(), { code: 'HOST_FAILED' });
  for (const result of [{ baseUrl: 'http://pc.example.com:3011', status: 2, token: 'secret' }, { baseUrl: embeddedTarget.server, status: 1, token: 'secret' }, { baseUrl: embeddedTarget.server, status: 2, token: '' }]) {
    const bad = createLocalHost({ native: () => true, available: () => true, plugin: { ensureStarted: async () => result } });
    await assert.rejects(bad.ensureStarted(), { code: 'INVALID_LOCAL_HOST' });
    assert.equal(bad.target(), null);
  }
});

test('auto: standalone sem configuração PC; recusa de conexão ativa host sem token PC', async () => {
  const f = autoFixture({ fetch: async () => { throw Error('ECONNREFUSED'); } });
  assert.equal(f.connection.hasServer(), true);
  assert.deepEqual(await f.connection.resolveTarget(), embeddedTarget);
  assert.equal(f.calls.length, 0);
  f.connection.configure({ local, remote });
  assert.deepEqual(await f.connection.resolveTarget(), embeddedTarget);
  assert.equal(f.calls.length, 2);
  assert.ok(f.calls.every(c => c.options.headers.Authorization === 'Bearer fixture-token'));
});

test('auto: deadline PC de 400ms e fallback após abort, sem garantia de latência de recusa', async () => {
  const pendingTimers = new Map(); let seq = 0;
  const timers = { setTimeout: (fn, ms) => { pendingTimers.set(++seq, { fn, ms }); return seq; }, clearTimeout: id => pendingTimers.delete(id) };
  const f = autoFixture({ timers, fetch: (_url, options) => new Promise((_, reject) => options.signal.addEventListener('abort', () => reject(options.signal.reason), { once: true })) });
  f.connection.configure({ local });
  const resolution = f.connection.resolveTarget();
  assert.equal(pendingTimers.size, 1);
  const timer = [...pendingTimers.values()][0];
  assert.equal(timer.ms, 400);
  timer.fn();
  assert.equal((await resolution).kind, 'local');
  assert.equal(pendingTimers.size, 0);
});

test('auto: standby sem activate e sincronização completa antes da promoção inicial', async () => {
  let finish;
  const f = autoFixture({ sync: args => {
    assert.equal(args.pc.server, local);
    assert.equal(args.pc.token, 'fixture-token');
    assert.deepEqual(args.mobile, embeddedTarget);
    return new Promise(r => { finish = r; });
  } });
  f.connection.configure({ local });
  const resolution = f.connection.resolveTarget();
  while (!finish) await Promise.resolve();
  assert.equal(f.connection.current(), embeddedTarget.server);
  assert.deepEqual(f.lifecycle, ['standby']);
  let dispatched = false;
  const writer = f.connection.acquireTarget({ write: true }).then(t => { dispatched = true; return t; });
  await Promise.resolve();
  assert.equal(dispatched, false);
  finish({ converged: true });
  assert.equal((await resolution).server, local);
  const target = await writer;
  assert.equal(target.token, 'fixture-token');
  target.release();
  assert.deepEqual(f.lifecycle, ['standby']);
});

test('auto: BUSY, CONFLICT, 401 e convergência incompleta permanecem locais e reportam', async () => {
  for (const code of ['BUSY', 'CONFLICT', 'UNPAIRED', 'HASH_DIVERGENCE']) {
    const f = autoFixture({ sync: async () => { throw Object.assign(Error(code), { code, status: code === 'UNPAIRED' ? 401 : 409 }); } });
    f.connection.configure({ local });
    assert.equal((await f.connection.resolveTarget()).kind, 'local');
    assert.equal(f.statuses.at(-1).code, code);
    assert.deepEqual(f.lifecycle, ['standby', 'activate']);
  }
  const unpaired = autoFixture({ token: () => '' });
  unpaired.connection.configure({ local });
  assert.equal((await unpaired.connection.resolveTarget()).token, embeddedTarget.token);
  assert.equal(unpaired.statuses.at(-1).code, 'PC_UNPAIRED');
});

test('auto: lease da geração impede troca e sync; liberação permite promoção', async () => {
  let pcUp = false, syncs = 0;
  const f = autoFixture({ fetch: async () => { if (!pcUp) throw Error('offline'); return Response.json({ ok: true, name: 'modularchat', encrypted: true }); }, sync: async () => { syncs++; return { converged: true }; } });
  f.connection.configure({ local });
  const lease = await f.connection.acquireTarget({ write: true, generation: true });
  pcUp = true;
  assert.equal((await f.connection.resolveTarget(undefined, { fresh: true })).kind, 'local');
  assert.equal(syncs, 0);
  lease.release(); lease.release();
  assert.equal((await f.connection.resolveTarget(undefined, { fresh: true })).kind, 'pc');
  assert.equal(syncs, 1);
});

test('auto: BUSY aguarda 10s para novo sync mesmo com writes locais consecutivos', async () => {
  let now = 0, syncs = 0;
  const f = autoFixture({ now: () => now, sync: async () => { syncs++; throw Object.assign(Error('job ativo'), { code: 'BUSY' }); } });
  f.connection.configure({ local });
  assert.equal((await f.connection.resolveTarget()).kind, 'local');
  for (let i = 0; i < 3; i++) {
    const target = await f.connection.acquireTarget({ write: true });
    assert.equal(target.kind, 'local');
    target.release();
  }
  assert.equal(syncs, 1);
  now = 10000;
  assert.equal((await f.connection.resolveTarget(undefined, { fresh: true })).kind, 'local');
  assert.equal(syncs, 2);
});

test('auto: timer foreground 10s, visibility/offline/resume e teardown StrictMode', async () => {
  let now = 0, seq = 0, syncs = 0;
  const jobs = new Map();
  const timers = { setTimeout: (fn, ms) => { jobs.set(++seq, { fn, ms }); return seq; }, clearTimeout: id => jobs.delete(id) };
  const f = autoFixture({ timers, now: () => now, sync: async () => { syncs++; return { converged: true }; } });
  f.connection.configure({ local });
  const win = new EventTarget(), doc = new EventTarget(), network = new EventTarget();
  doc.visibilityState = 'visible';
  const surfaces = { window: win, document: doc, navigator: { connection: network } };
  const runTimer = async ms => { const [id, job] = [...jobs].find(([, j]) => j.ms === ms); jobs.delete(id); await job.fn(); };
  const stop = f.connection.start(surfaces);
  await runTimer(0);
  assert.equal(syncs, 1);
  assert.deepEqual([...jobs.values()].map(j => j.ms), [10000]);
  now = 10000;
  await runTimer(10000);
  assert.equal(syncs, 2);
  doc.visibilityState = 'hidden'; doc.dispatchEvent(new Event('visibilitychange'));
  assert.equal(jobs.size, 0);
  doc.visibilityState = 'visible'; win.dispatchEvent(new Event('mc:resume'));
  assert.deepEqual([...jobs.values()].map(j => j.ms), [0]);
  await runTimer(0);
  win.dispatchEvent(new Event('offline'));
  assert.deepEqual([...jobs.values()].map(j => j.ms), [0]);
  stop(); stop();
  assert.equal(jobs.size, 0);
  win.dispatchEvent(new Event('mc:resume'));
  assert.equal(jobs.size, 0, 'listeners removidos');
  const again = f.connection.start(surfaces);
  again();
  assert.equal(jobs.size, 0);
});

test('auto: cleanup aborta probe em andamento e época antiga nunca promove', async () => {
  let probeSignal;
  const f = autoFixture({ fetch: async (_url, options) => {
    probeSignal = options.signal;
    return new Promise((_, reject) => options.signal.addEventListener('abort', () => reject(options.signal.reason), { once: true }));
  } });
  f.connection.configure({ local });
  const pending = f.connection.resolveTarget();
  f.connection.invalidate();
  assert.equal(probeSignal.aborted, true);
  await assert.rejects(pending, { code: 'STALE_CONNECTION' });
  assert.equal(f.connection.kind(), 'local');
});

test('auto: browser mantém same origin mesmo com registro nativo legado', async () => {
  const f = autoFixture({ native: () => false });
  f.data.set('mc.server', 'http://pc-old.example.com:3000');
  assert.equal(f.connection.current(), '');
  assert.deepEqual(await f.connection.resolveTarget(), { server: '', token: 'fixture-token', kind: 'browser' });
  assert.equal(f.calls.length, 0);
});

test('auto: cancelamento de consumidor não cancela sync compartilhado; stale config impede promoção', async () => {
  let finish;
  const f = autoFixture({ sync: () => new Promise(r => { finish = r; }) });
  f.connection.configure({ local });
  const abort = new AbortController();
  const first = f.connection.resolveTarget(abort.signal), second = f.connection.resolveTarget();
  while (!finish) await Promise.resolve();
  abort.abort();
  await assert.rejects(first, { name: 'AbortError' });
  finish({ converged: true });
  assert.equal((await second).kind, 'pc');
  f.connection.invalidate();
  const stale = f.connection.resolveTarget();
  finish = null;
  while (!finish) await Promise.resolve();
  f.connection.configure({ local: 'https://changed.example.com' });
  finish({ converged: true });
  await assert.rejects(stale, { code: 'STALE_CONNECTION' });
  assert.equal(f.connection.current(), embeddedTarget.server);
});

test('conexão: prefere LAN, compartilha sondagem e mantém escolha por 30 segundos', async () => {
  let now = 0;
  const f = fixture({ now: () => now });
  f.connection.configure({ local, remote });
  assert.deepEqual(await Promise.all([f.connection.resolve(), f.connection.resolve()]), [local, local]);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].options.headers.Authorization, 'Bearer fixture-token');
  assert.equal(f.calls[0].options.redirect, 'error');
  now = 29999;
  assert.equal(await f.connection.resolve(), local);
  assert.equal(f.calls.length, 1);
  now = 30001;
  await f.connection.resolve();
  assert.equal(f.calls.length, 2);
});

test('conexão: usa internet quando LAN falha; falha/rede nova invalida escolha', async () => {
  const f = fixture({ fetch: async url => {
    if (url.startsWith(local)) throw Error('offline');
    return Response.json({ ok: true, name: 'modularchat', encrypted: true });
  } });
  f.connection.configure({ local, remote });
  assert.equal(await f.connection.resolve(), remote);
  assert.deepEqual(f.calls.map(c => c.url), [`${local}/api/health`, `${remote}/api/health`]);
  f.connection.invalidate();
  await f.connection.resolve();
  assert.equal(f.calls.length, 4);
});

test('conexão: revogação conserva o host e não provoca troca para o domínio', async () => {
  const f = fixture({ fetch: async () => Response.json({ code: 'UNPAIRED' }, { status: 401 }) });
  f.connection.configure({ local, remote });
  assert.equal(await f.connection.resolve(), local);
  assert.equal(f.calls.length, 1);
});

test('conexão: outro PC não herda domínio; tentativa malsucedida restaura configuração legada', async () => {
  const f = fixture();
  f.data.set('mc.server', 'http://192.168.0.10:3000');
  assert.equal(f.connection.hasServer(), false, 'HTTP antigo exige configuração HTTPS explícita');
  const undo = f.connection.configure({ local, remote });
  undo();
  assert.deepEqual(f.connection.configured(), { local: 'http://192.168.0.10:3000', remote: '' });
  f.connection.configure({ local, remote });
  f.connection.configure({ local: 'https://other.example.com' });
  assert.deepEqual(f.connection.configured(), { local: 'https://other.example.com', remote: '' });
});

test('conexão: mudança durante sondagem impede requisição com configuração obsoleta', async () => {
  let release;
  const f = fixture({ fetch: () => new Promise(resolve => { release = () => resolve(Response.json({ ok: true, name: 'modularchat', encrypted: true })); }) });
  f.connection.configure({ local, remote });
  const pending = f.connection.resolve();
  f.connection.configure({ local: 'https://other.example.com' });
  release();
  await assert.rejects(pending, /conexão mudou/);
  assert.equal(f.connection.current(), 'https://other.example.com');
});

test('conexão: cancelar um consumidor não cancela a sondagem compartilhada', async () => {
  let release;
  const f = fixture({ fetch: () => new Promise(resolve => { release = () => resolve(Response.json({ ok: true, name: 'modularchat', encrypted: true })); }) });
  f.connection.configure({ local });
  const abort = new AbortController();
  const first = f.connection.resolve(abort.signal), second = f.connection.resolve();
  abort.abort();
  await assert.rejects(first, { name: 'AbortError' });
  release();
  assert.equal(await second, local);
  assert.equal(f.calls.length, 1);
});

test('conexão: rejeita serviço diferente ou sem cofre e mantém origem do navegador', async () => {
  for (const body of [{ ok: true, name: 'another-app', encrypted: true }, { ok: true, name: 'modularchat', encrypted: false }]) {
    const f = fixture({ fetch: async () => Response.json(body) });
    f.connection.configure({ local });
    await assert.rejects(f.connection.resolve(), /Não consegui conectar/);
  }
  const browser = fixture({ native: () => false });
  assert.equal(await browser.connection.resolve(), '');
  assert.equal(browser.calls.length, 0);
});
