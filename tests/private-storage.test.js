import test from 'node:test';
import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { registerHooks } from 'node:module';

const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
let sequence = 0;
async function fixture(t, { encrypt, native = false } = {}) {
  const names = ['localStorage', 'fetch', 'crypto', '__privateStorageFixture'];
  const originals = Object.fromEntries(names.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const values = new Map(), persisted = [], keyBytes = new Uint8Array(32).fill(37), calls = [];
  const aad = new TextEncoder().encode('ModularChat.PrivateCache.v1');
  const key = await webcrypto.subtle.importKey('raw', keyBytes, 'AES-GCM', false, ['encrypt', 'decrypt']);
  let active = 0, maximum = 0, quota = false, cryptoFailure = false, authorization, nativeReady = 0;
  const storage = { get length() { return values.size; }, key: i => [...values.keys()][i] ?? null,
    getItem: k => values.get(k) ?? null, removeItem: k => values.delete(k),
    setItem(k, value) { if (quota) throw Error('quota fixture'); values.set(k, String(value)); if (k === 'mc.private.v1') persisted.push(JSON.parse(value)); } };
  async function encryptValue(algorithm, cryptoKey, input) {
    const index = calls.length;
    calls.push(new TextDecoder().decode(input));
    active++; maximum = Math.max(maximum, active);
    try {
      if (cryptoFailure) throw Error('crypto fixture');
      await encrypt?.(index);
      return await webcrypto.subtle.encrypt(algorithm, cryptoKey, input);
    } finally { active--; }
  }
  Object.defineProperty(globalThis, 'crypto', { configurable: true, value: {
    getRandomValues: a => webcrypto.getRandomValues(a),
    subtle: { importKey: (...args) => webcrypto.subtle.importKey(...args), decrypt: (...args) => webcrypto.subtle.decrypt(...args), encrypt: encryptValue },
  } });
  globalThis.localStorage = storage;
  globalThis.fetch = async (_url, options) => { authorization = options.headers.Authorization; return Response.json({ key: Buffer.from(keyBytes).toString('base64') }); };
  const url = new URL(`../client/src/private-storage.js?fixture=${++sequence}`, import.meta.url).href;
  if (native) {
    // Replace only the Capacitor boundary. AES-GCM still runs on a fixture key; no Keystore is read.
    globalThis.__privateStorageFixture = {
      async ready() { nativeReady++; },
      async encrypt({ text }) {
        const nonce = webcrypto.getRandomValues(new Uint8Array(12));
        const data = await encryptValue({ name: 'AES-GCM', iv: nonce, additionalData: aad }, key, new TextEncoder().encode(text));
        return { nonce: Buffer.from(nonce).toString('base64'), data: Buffer.from(data).toString('base64') };
      },
      async decrypt(value) {
        const data = await webcrypto.subtle.decrypt({ name: 'AES-GCM', iv: Buffer.from(value.nonce, 'base64'), additionalData: aad }, key, Buffer.from(value.data, 'base64'));
        return { text: new TextDecoder().decode(data) };
      },
    };
  }
  const hook = registerHooks({ resolve(specifier, context, next) {
    if (specifier === '@capacitor/core' && context.parentURL?.includes('/private-storage.js?fixture=')) return { shortCircuit: true,
      url: `data:text/javascript,export const Capacitor={isNativePlatform:()=>${native}};export const registerPlugin=()=>globalThis.__privateStorageFixture;` };
    return next(specifier, context);
  } });
  let module;
  try { module = await import(url); } finally { hook?.deregister(); }
  t.after(() => {
    for (const [name, descriptor] of Object.entries(originals)) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name];
    }
  });
  return { module, values, persisted, calls, storage, get maximum() { return maximum; }, get authorization() { return authorization; }, get nativeReady() { return nativeReady; },
    quota: value => { quota = value; }, cryptoFailure: value => { cryptoFailure = value; } };
}

test('private: crypto lento coalesce 100 revisões, uma criptografia em voo e Promise aguarda sua durabilidade', async t => {
  const gates = [deferred(), deferred()], starts = [deferred(), deferred()];
  const f = await fixture(t, { encrypt: async index => { starts[index]?.resolve(); await gates[index]?.promise; } });
  await f.module.initializePrivacy();
  let firstDone = false, latestDone = false;
  const first = f.module.privateSet('mc.pending.fixture', 'revision-0').then(() => { firstDone = true; });
  await starts[0].promise;
  const changes = [];
  for (let i = 1; i <= 100; i++) changes.push(f.module.privateSet('mc.pending.fixture', `revision-${i}`));
  changes.at(-1).then(() => { latestDone = true; });
  assert.equal(f.calls.length, 1);
  assert.equal(firstDone, false);
  assert.equal(latestDone, false);
  assert.equal(f.values.has('mc.private.v1'), false);
  gates[0].resolve();
  await first;
  await starts[1].promise;
  assert.equal((await f.module.decodePrivate(f.persisted[0]))['mc.pending.fixture'], 'revision-0');
  assert.equal(latestDone, false);
  gates[1].resolve();
  await Promise.all(changes);
  assert.equal(f.calls.length, 2);
  assert.equal(f.maximum, 1);
  assert.equal((await f.module.decodePrivate(f.persisted[1]))['mc.pending.fixture'], 'revision-100');
  await f.module.privateSet('mc.pending.fixture', 'revision-100');
  assert.equal(f.calls.length, 2, 'no-op durável não recriptografa');
});

test('private: remoção posterior não confirma antecipadamente pending/ack em voo', async t => {
  const gates = [deferred(), deferred()], starts = [deferred(), deferred()];
  const f = await fixture(t, { encrypt: async index => { starts[index]?.resolve(); await gates[index]?.promise; } });
  await f.module.initializePrivacy();
  const pending = f.module.privateSet('mc.pending.fixture', '{"opId":"fixture-op"}');
  await starts[0].promise;
  let removed = false;
  const deletion = f.module.privateSet('mc.pending.fixture', null).then(() => { removed = true; });
  gates[0].resolve();
  await pending;
  await starts[1].promise;
  assert.equal(removed, false);
  assert.ok((await f.module.decodePrivate(f.persisted[0]))['mc.pending.fixture']);
  gates[1].resolve();
  await deletion;
  assert.equal((await f.module.decodePrivate(f.persisted[1]))['mc.pending.fixture'], undefined);
});

test('private: falha rejeita ack/sync/draft/token, retém estado e retry de no-op persiste todas as revisões', async t => {
  const gate = deferred(), start = deferred();
  const f = await fixture(t, { encrypt: async index => { if (!index) { start.resolve(); await gate.promise; } } });
  await f.module.initializePrivacy();
  const ack = f.module.privateSet('mc.pending.fixture', 'ack');
  await start.promise;
  const tasks = [ack, f.module.privateSet('mc.sync.fixture', 'receipt'), f.module.privateSet('mc.draft.fixture', 'draft'), f.module.privateSet('mc.token', 'fixture-token')];
  const settled = Promise.allSettled(tasks);
  f.quota(true); gate.resolve();
  const results = await settled;
  assert.ok(results.every(result => result.status === 'rejected' && /quota/.test(result.reason.message)));
  assert.equal(f.persisted.length, 0);
  assert.equal(f.calls.length, 1, 'falha não inicia um retry infinito');
  assert.equal(f.module.privateGet('mc.sync.fixture'), 'receipt');
  f.quota(false);
  await f.module.privateSet('mc.token', 'fixture-token');
  const restored = await f.module.decodePrivate(f.persisted[0]);
  assert.deepEqual(restored, { 'mc.pending.fixture': 'ack', 'mc.sync.fixture': 'receipt', 'mc.draft.fixture': 'draft', 'mc.token': 'fixture-token' });
});

test('private: falha crypto não grava plaintext; migração privada preserva fonte até gravação confirmada', async t => {
  const f = await fixture(t);
  f.values.set('mc.draft.legacy', 'PRIVATE-FIXTURE-LEGACY');
  f.cryptoFailure(true);
  await assert.rejects(f.module.initializePrivacy(), /crypto fixture/);
  assert.equal(f.values.get('mc.draft.legacy'), 'PRIVATE-FIXTURE-LEGACY');
  assert.equal(f.values.has('mc.private.v1'), false);
  f.cryptoFailure(false);
  await f.module.initializePrivacy();
  assert.equal(f.values.has('mc.draft.legacy'), false);
  assert.equal(f.module.privateGet('mc.draft.legacy'), 'PRIVATE-FIXTURE-LEGACY');
  assert.ok(!f.values.get('mc.private.v1').includes('PRIVATE-FIXTURE-LEGACY'));
});

test('private: AES-256-GCM mantém envelope/AAD, cache legado e rejeita adulteração', async t => {
  const f = await fixture(t);
  await f.module.initializePrivacy();
  const envelope = await f.module.encodePrivate(JSON.stringify({ history: ['fixture'] }));
  assert.deepEqual(Object.keys(envelope).sort(), ['data', 'nonce', 'sealed']);
  assert.equal(Buffer.from(envelope.nonce, 'base64').length, 12);
  assert.deepEqual(await f.module.decodePrivate(envelope), { history: ['fixture'] });
  assert.deepEqual(await f.module.decodePrivate({ legacy: true }), { legacy: true });
  const tampered = Buffer.from(envelope.data, 'base64'); tampered[0] ^= 1;
  await assert.rejects(f.module.decodePrivate({ ...envelope, data: tampered.toString('base64') }));
  const nonce = Buffer.from(envelope.nonce, 'base64'); nonce[0] ^= 1;
  await assert.rejects(f.module.decodePrivate({ ...envelope, nonce: nonce.toString('base64') }));
});

test('private: ABI nativa encrypt {text} → {nonce,data}, decrypt → {text} preservada com fixture', async t => {
  const f = await fixture(t, { native: true });
  await f.module.initializePrivacy();
  assert.equal(f.nativeReady, 1);
  assert.equal(f.authorization, undefined, 'fixture nativa não usa a chave do servidor');
  await f.module.privateSet('mc.pending.native-fixture', 'native-ack');
  assert.equal(f.persisted[0].sealed, 1);
  assert.equal((await f.module.decodePrivate(f.persisted[0]))['mc.pending.native-fixture'], 'native-ack');
  assert.equal(f.maximum, 1);
});

test('private: espera de pareamento não confirma mutações sem proteção; token explícito abre a chave', async t => {
  const f = await fixture(t);
  const fetch = globalThis.fetch;
  globalThis.fetch = async () => new Response(null, { status: 401 });
  await f.module.initializePrivacy();
  await assert.rejects(f.module.privateSet('mc.pending.fixture', 'preservar'), /proteger/);
  await assert.rejects(f.module.privateSet('mc.sync.fixture', 'receipt'), /protegido/);
  assert.equal(f.values.size, 0);
  globalThis.fetch = fetch;
  await f.module.initializePrivacy('', 'pairing-fixture-token');
  assert.equal(f.authorization, 'Bearer pairing-fixture-token');
  assert.equal((await f.module.decodePrivate(f.persisted[0]))['mc.pending.fixture'], 'preservar');
});
