import test from 'node:test';
import assert from 'node:assert/strict';
import { createServerConnection, normalizeServer, normalizePcServer, testPcConnection } from '../client/src/server-connection.js';
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

test('conexão: HTTP privado explícito ou sem esquema; internet continua exigindo HTTPS', () => {
  for (const host of ['10.229.240.173', '192.168.1.100', '172.16.0.2', '172.31.255.254', 'pc.local']) {
    assert.equal(normalizeServer(`${host}:3000`, false), `http://${host}:3000`);
    assert.equal(normalizeServer(`http://${host}:3000`, false), `http://${host}:3000`);
    assert.throws(() => normalizeServer(`http://${host}:3000`));
  }
  assert.equal(normalizeServer('chat.example.com', false), remote);
  assert.equal(normalizeServer('chat.example.com'), remote);
  for (const host of ['172.15.0.1', '172.32.0.1', '10.example.com', 'example.com', '8.8.8.8']) {
    assert.throws(() => normalizeServer(`http://${host}:3000`, false));
  }
  assert.throws(() => fixture().connection.configure({ local: 'http://example.com:3000' }));
  assert.throws(() => fixture().connection.configure({ remote: 'http://192.168.1.100:3000' }));
});

test('conexão: testar candidato novo não exige cadastro e não envia credencial do PC anterior', async () => {
  const f = fixture();
  f.connection.configure({ local, remote });
  const before = [...f.data];
  let clock = 100;
  const result = await testPcConnection('10.229.240.173:3000', { now: () => clock,
    fetch: async (url, options) => {
      assert.equal(url, 'http://10.229.240.173:3000/api/health');
      assert.equal(options.headers, undefined);
      assert.equal(options.credentials, 'omit');
      assert.equal(options.redirect, 'error');
      clock += 23;
      return Response.json({ ok: true, name: 'modularchat', encrypted: true });
    } });
  assert.equal(result.latencyMs, 23);
  assert.deepEqual([...f.data], before);
  await assert.rejects(f.connection.acquireTarget({ pc: result.server }), { code: 'ORIGIN_CHANGED' });
  assert.equal(f.calls.length, 0, 'operações normais ainda exigem o PC configurado');
});

test('PC no celular: loopback é explicado antes de testar ou alterar a configuração', async () => {
  const f = fixture();
  f.connection.configure({ local });
  const before = [...f.data];
  for (const address of ['http://127.0.0.1:3000', '127.0.0.1:3000', 'http://127.42.1.2:3000',
    'http://127.1:3000', 'http://localhost:3000', 'http://[::1]:3000', 'https://[::ffff:127.0.0.1]:3000', 'https://0.0.0.0:3000']) {
    assert.throws(() => normalizePcServer(address), /próprio celular/);
    assert.throws(() => f.connection.configure({ local: address }), /próprio celular/);
    await assert.rejects(testPcConnection(address, { fetch: () => assert.fail('loopback não deve ser sondado como PC') }), /próprio celular/);
    assert.deepEqual([...f.data], before, 'a configuração válida anterior é preservada');
  }
  assert.equal(normalizePcServer('192.168.1.100:3000'), 'http://192.168.1.100:3000');
  assert.equal(normalizePcServer(remote, true), remote);
  assert.equal(normalizeServer('http://127.0.0.1:3011', false), 'http://127.0.0.1:3011', 'o host Android continua usando loopback');
  assert.equal(f.calls.length, 0);
});

test('conexão: teste público valida identidade/cofre e respeita cancelamento antes do envio', async () => {
  for (const data of [{ ok: true }, { ok: true, name: 'other', encrypted: true }, { ok: true, name: 'modularchat', encrypted: false }]) {
    await assert.rejects(testPcConnection(local, { fetch: async () => Response.json(data) }), /não é um servidor ModularChat/);
  }
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(testPcConnection(local, { signal: controller.signal, fetch: () => assert.fail('envio cancelado') }), { name: 'AbortError' });
});

const embeddedTarget = { server: 'http://127.0.0.1:3011', token: 'keystore-only', kind: 'local' };
function autoFixture(overrides = {}) {
  const lifecycle = [], statuses = [];
  const embedded = { target: () => embeddedTarget, activate: async () => lifecycle.push('activate'), standby: async () => lifecycle.push('standby') };
  const f = fixture({ mode: 'auto', embedded, sync: async () => ({ converged: true }), onStatus: s => statuses.push(s), ...overrides });
  return { ...f, lifecycle, statuses };
}

test('PC no celular: cadastro antigo de loopback não é contatado nem bloqueia o host Android', async () => {
  const f = autoFixture();
  f.data.set('mc.connection', JSON.stringify({ local: 'http://127.0.0.1:3000', remote: '' }));
  assert.equal((await f.connection.resolveTarget()).kind, 'local');
  assert.equal(await f.connection.probePc(), false);
  const target = await f.connection.acquireTarget({ embedded: true, write: true });
  assert.equal(target.server, embeddedTarget.server); target.release();
  await assert.rejects(f.connection.acquireTarget({ pc: 'http://127.0.0.1:3000' }), /próprio celular/);
  assert.equal(f.calls.length, 0);
});

test('auto: LAN HTTP funciona na seleção, pareamento, recuperação, probe e troca após preferência local', async () => {
  const lan = 'http://10.229.240.173:3000';
  const f = autoFixture();
  f.connection.configure({ local: '10.229.240.173:3000' });
  assert.equal((await f.connection.resolveTarget()).server, lan);
  for (const options of [{ pc: lan }, { pc: lan, write: true }, { origin: { server: lan, kind: 'pc' }, write: true }]) {
    const target = await f.connection.acquireTarget(options);
    assert.equal(target.server, lan);
    assert.equal(target.token, 'fixture-token');
    target.release();
  }
  await f.connection.switchToLocal();
  assert.equal(await f.connection.probePc(), true);
  assert.equal((await f.connection.switchToPc()).server, lan);
  assert.equal(f.connection.kind(), 'pc');
  assert.ok(f.calls.every(call => call.url === `${lan}/api/health`));
  assert.deepEqual(f.lifecycle, ['standby', 'activate', 'standby']);
});

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
  assert.equal(f.connection.hasServer(), true, 'HTTP privado legado continua utilizável');
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

test('conexão: switchToLocal promove host Android sem reiniciar e probePc detecta volta do PC', async () => {
  let pcOnline = true;
  const f = autoFixture({
    fetch: async (url) => {
      if (!pcOnline) throw new Error('PC offline');
      return Response.json({ ok: true, name: 'modularchat', encrypted: true });
    },
    sync: async () => ({ converged: true }),
  });
  f.connection.configure({ local });

  // Initial target is PC
  const target1 = await f.connection.resolveTarget();
  assert.equal(target1.kind, 'pc');
  assert.equal(await f.connection.probePc(), true);

  // Switch to local host without restarting
  const localTarget = await f.connection.switchToLocal();
  assert.equal(localTarget.kind, 'local');
  assert.equal((await f.connection.resolveTarget()).kind, 'local');

  // PC goes offline: probePc returns false
  pcOnline = false;
  assert.equal(await f.connection.probePc(), false);

  // PC recovers: probePc returns true
  pcOnline = true;
  assert.equal(await f.connection.probePc(), true);

  // Switch back to PC
  const pcTarget = await f.connection.switchToPc();
  assert.equal(pcTarget.kind, 'pc');
  assert.equal(f.connection.kind(), 'pc');
});

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const drain = async () => { for (let i = 0; i < 24; i++) await Promise.resolve(); };
function startupFixture(overrides = {}) {
  const native = deferred(), scheduled = new Map();
  let starts = 0, sequence = 0;
  const timers = { setTimeout(fn, ms) { scheduled.set(++sequence, { fn, ms }); return sequence; }, clearTimeout(id) { scheduled.delete(id); } };
  const embedded = createLocalHost({ native: () => true, available: () => true,
    plugin: { ensureStarted: () => { starts++; return native.promise; } }, fetch: async () => Response.json({ ok: true }) });
  const f = autoFixture({ embedded, timers, fetch: async () => { throw Error('PC desligado'); }, ...overrides });
  return { ...f, embedded, native, scheduled, timers, starts: () => starts,
    ready: () => native.resolve({ baseUrl: embeddedTarget.server, status: 2, token: embeddedTarget.token }) };
}

test('lote3a: JNI tardio compartilha boot/seleção, limpa deadline e notifica sem credencial', async () => {
  const f = startupFixture(), notifications = [];
  const unsubscribe = f.embedded.subscribe(state => notifications.push(state));
  const boot = f.embedded.ensureStarted();
  const ac = new AbortController();
  const first = f.connection.resolveTarget(ac.signal), second = f.connection.resolveTarget();
  ac.abort();
  await assert.rejects(first, { name: 'AbortError' });
  f.ready();
  assert.deepEqual(await boot, embeddedTarget);
  assert.equal((await second).kind, 'local');
  assert.equal(f.starts(), 1);
  assert.equal(f.scheduled.size, 0);
  assert.deepEqual(notifications.map(n => n.phase), ['starting', 'ready']);
  assert.ok(notifications.every(n => !JSON.stringify(n).includes(embeddedTarget.token)));
  assert.ok(!f.statuses.some(s => s.code === 'PC_CONNECTION_LOST'));
  unsubscribe();
});

test('lote3a: cancelar único consumidor durante JNI não promove nem deixa timer', async () => {
  const f = startupFixture(), ac = new AbortController();
  const pending = f.connection.switchToLocal(ac.signal);
  ac.abort();
  await assert.rejects(pending, { name: 'AbortError' });
  await drain();
  assert.equal(f.scheduled.size, 0);
  f.ready(); await drain();
  assert.equal(f.changes.length, 0);
  assert.equal((await f.connection.switchToLocal()).kind, 'local');
  assert.equal(f.starts(), 1);
});

test('lote3a: reconfigurar durante JNI descarta época antiga sem promover PC antigo', async () => {
  const f = startupFixture();
  f.connection.configure({ local });
  const old = f.connection.resolveTarget();
  f.connection.configure({ local: remote });
  await assert.rejects(old, { code: 'STALE_CONNECTION' });
  f.ready(); await drain();
  assert.equal(f.changes.length, 0);
  assert.equal((await f.connection.resolveTarget()).kind, 'local');
  assert.ok(f.calls.every(c => !c.url.startsWith(local)));
  assert.equal(f.scheduled.size, 0);
});

test('lote3a: timeout JNI limpa timer; readiness tardia permite retry real sem novo JNI', async () => {
  const f = startupFixture();
  const first = f.connection.switchToLocal();
  const timeout = [...f.scheduled.values()].find(t => t.ms === 5000);
  timeout.fn();
  await assert.rejects(first, { code: 'LOCAL_HOST_TIMEOUT' });
  assert.equal(f.scheduled.size, 0);
  f.ready(); await drain();
  assert.equal(f.embedded.startupState().phase, 'ready');
  assert.equal((await f.connection.switchToLocal()).kind, 'local');
  assert.equal(f.starts(), 1);
});

test('lote3a: JNI indisponível informa erro; retry preserva endpoints e volta a chamar plugin', async () => {
  let calls = 0;
  const embedded = createLocalHost({ native: () => true, available: () => true,
    plugin: { ensureStarted: async () => { if (++calls === 1) throw Object.assign(Error('JNI indisponível'), { code: 'HOST_FAILED' });
      return { baseUrl: embeddedTarget.server, status: 2, token: embeddedTarget.token }; } }, fetch: async () => Response.json({ ok: true }) });
  const f = autoFixture({ embedded });
  f.connection.configure({ local, remote });
  const record = f.data.get('mc.connection');
  await assert.rejects(f.connection.switchToLocal(), { code: 'HOST_FAILED' });
  assert.equal(embedded.startupState().phase, 'error');
  assert.equal((await f.connection.switchToLocal()).kind, 'local');
  assert.equal(f.data.get('mc.connection'), record);
  assert.equal(calls, 2);
});

test('lote3a: troca local nunca devolve PC quando plugin ausente; activate não é silenciado', async () => {
  const absent = autoFixture({ embedded: { target: () => null, ensureStarted: async () => null } });
  absent.connection.configure({ local });
  assert.equal((await absent.connection.resolveTarget()).kind, 'pc');
  await assert.rejects(absent.connection.switchToLocal(), { code: 'LOCAL_HOST_UNAVAILABLE' });
  assert.equal(absent.connection.kind(), 'pc');
  const f = autoFixture({ embedded: { target: () => embeddedTarget, standby: async () => {}, activate: async () => { throw Error('activate recusado'); } } });
  f.connection.configure({ local });
  await f.connection.resolveTarget();
  await assert.rejects(f.connection.switchToLocal(), { code: 'LOCAL_ACTIVATE_FAILED' });
  assert.equal(f.connection.kind(), 'pc');
  assert.equal((await f.connection.resolveTarget()).kind, 'pc', 'preferência local só muda após sucesso');
});

test('lote3a: troca PC offline ou sync incompleto rejeita sem declarar PC reconectado', async () => {
  const noHost = autoFixture({ embedded: { target: () => null, ensureStarted: async () => null } });
  noHost.connection.configure({ local });
  await assert.rejects(noHost.connection.switchToPc(), { code: 'LOCAL_HOST_UNAVAILABLE' });
  for (const converged of [false, true]) {
    const f = autoFixture({ fetch: async () => { if (converged) throw Error('offline'); return Response.json({ ok: true, name: 'modularchat', encrypted: true }); },
      sync: async () => ({ converged }) });
    f.connection.configure({ local });
    await f.connection.switchToLocal();
    await assert.rejects(f.connection.switchToPc(), { code: converged ? 'PC_UNAVAILABLE' : 'INCOMPLETE_SYNC' });
    assert.equal(f.connection.kind(), 'local');
    assert.ok(!f.statuses.some(s => s.code === 'SYNCED'));
  }
});

test('lote3a: trocas manuais compartilham sync e só resolvem após convergência', async () => {
  const gate = deferred(); let syncs = 0;
  const f = autoFixture({ sync: () => { syncs++; return gate.promise; } });
  f.connection.configure({ local });
  await f.connection.switchToLocal();
  const first = f.connection.switchToPc(), second = f.connection.switchToPc();
  await drain();
  assert.equal(syncs, 1); assert.equal(f.connection.kind(), 'local');
  gate.resolve({ converged: true });
  assert.ok((await Promise.all([first, second])).every(t => t.kind === 'pc'));
});

test('lote3a: cancelar promoção PC aborta sync e resposta tardia não promove', async () => {
  const gate = deferred(); let syncSignal;
  const f = autoFixture({ sync: options => { syncSignal = options.signal; return gate.promise; } });
  f.connection.configure({ local }); await f.connection.switchToLocal();
  const ac = new AbortController(), promotion = f.connection.switchToPc(ac.signal);
  await drain(); ac.abort();
  await assert.rejects(promotion, { name: 'AbortError' });
  assert.equal(syncSignal.aborted, true);
  gate.resolve({ converged: true }); await drain();
  assert.equal(f.connection.kind(), 'local');
  assert.ok(!f.statuses.some(s => s.code === 'SYNCED'));
});

test('lote3a: leases POST/SSE inclusive origem fixa bloqueiam trocas manuais e sync', async () => {
  const f = autoFixture();
  f.connection.configure({ local }); await f.connection.switchToLocal();
  for (const options of [{ write: true }, { generation: true }, { write: true, origin: { server: local, kind: 'pc' } }]) {
    const lease = await f.connection.acquireTarget(options);
    const before = f.lifecycle.length;
    await assert.rejects(f.connection.switchToPc(), { code: 'ORIGIN_BUSY' });
    await assert.rejects(f.connection.switchToLocal(), { code: 'ORIGIN_BUSY' });
    assert.equal(f.lifecycle.length, before);
    assert.equal((await f.connection.resolveTarget(undefined, { fresh: true })).kind, 'local');
    lease.release();
  }
  assert.equal((await f.connection.switchToPc()).kind, 'pc');
});

test('lote3a: preferência local reativa após standby/resume sem sondar PC', async () => {
  let posts = 0;
  const embedded = createLocalHost({ native: () => true, available: () => true,
    plugin: { ensureStarted: async () => ({ baseUrl: embeddedTarget.server, status: 2, token: embeddedTarget.token }) },
    fetch: async () => { posts++; return Response.json({ ok: true }); } });
  const f = autoFixture({ embedded });
  await f.connection.switchToLocal();
  await embedded.standby(); embedded.invalidateLifecycle(); f.connection.invalidate();
  assert.equal((await f.connection.resolveTarget()).kind, 'local');
  assert.equal(posts, 3); assert.equal(f.calls.length, 0);
});

test('lote3a: probePc cancelado limpa deadline e não tenta outro endpoint', async () => {
  const scheduled = new Map(); let sequence = 0;
  const f = autoFixture({ timers: { setTimeout(fn, ms) { scheduled.set(++sequence, { fn, ms }); return sequence; }, clearTimeout(id) { scheduled.delete(id); } },
    fetch: () => new Promise(() => {}) });
  f.connection.configure({ local, remote });
  const ac = new AbortController(), probe = f.connection.probePc(ac.signal);
  ac.abort(); await assert.rejects(probe, { name: 'AbortError' });
  assert.equal(scheduled.size, 0); assert.equal(f.calls.length, 1);
});

test('lote3a: watcher distingue falha de activate para UI oferecer retry', async () => {
  const scheduled = new Map(); let sequence = 0;
  const f = autoFixture({ timers: { setTimeout(fn, ms) { scheduled.set(++sequence, { fn, ms }); return sequence; }, clearTimeout(id) { scheduled.delete(id); } },
    embedded: { target: () => embeddedTarget, activate: async () => { throw Object.assign(Error('Activate recusado'), { code: 'ACTIVATE_FAILED' }); } } });
  const document = new EventTarget(); document.visibilityState = 'visible';
  const stop = f.connection.start({ document, window: new EventTarget() });
  const [id, tick] = [...scheduled].find(([, timer]) => timer.ms === 0); scheduled.delete(id); await tick.fn();
  assert.equal(f.statuses.at(-1).selectionFailed, true); assert.equal(f.statuses.at(-1).code, 'ACTIVATE_FAILED');
  stop(); assert.equal(scheduled.size, 0);
});

test('lote3a: activate confirmado após cancelamento não promove nem repete POST no retry', async () => {
  const gate = deferred(); let posts = 0;
  const embedded = createLocalHost({ native: () => true, available: () => true,
    plugin: { ensureStarted: async () => ({ baseUrl: embeddedTarget.server, status: 2, token: embeddedTarget.token }) },
    fetch: async () => { posts++; return posts === 1 ? gate.promise : Response.json({ ok: true }); } });
  await embedded.ensureStarted();
  const f = autoFixture({ embedded }), ac = new AbortController();
  const activating = f.connection.switchToLocal(ac.signal); await drain();
  ac.abort(); await assert.rejects(activating, { name: 'AbortError' });
  gate.resolve(Response.json({ ok: true })); await drain();
  assert.equal(f.changes.length, 0);
  await f.connection.switchToLocal();
  assert.equal(posts, 1, 'resposta confirmada preserva lifecycle sem repetir o POST aceito');
});
