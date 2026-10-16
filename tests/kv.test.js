import test from 'node:test';
import assert from 'node:assert/strict';
import { createKeyValueStore } from '../client/src/kv.js';

const quota = () => new DOMException('quota fixture', 'QuotaExceededError');
function fixture() {
  const entries = new Map(), legacy = new Map(), connections = [], events = [];
  const control = { openError: false, blocked: false, writeError: false, quota: false, abort: null, cryptoError: false };
  let active = 0;
  const storage = {
    get length() { return legacy.size; }, key: i => [...legacy.keys()][i] ?? null, getItem: key => legacy.get(key) ?? null,
    setItem(key, value) { if (control.quota) throw quota(); legacy.set(key, String(value)); }, removeItem: key => legacy.delete(key),
  };
  const database = {
    open() {
      const req = {};
      queueMicrotask(() => {
        if (control.openError) { req.error = Error('open fixture'); req.onerror(); return; }
        const db = {
          closed: false, close() { this.closed = true; },
          transaction(_store, mode) {
            if (this.closed) throw new DOMException('closed fixture', 'InvalidStateError');
            const tx = { error: null, finished: false };
            active++;
            const finish = event => {
              if (tx.finished) return;
              tx.finished = true; active--; tx[event]?.();
            };
            tx.abort = () => finish('onabort');
            const request = (operation, result) => {
              const req = {};
              queueMicrotask(() => {
                if (control.abort === mode) { control.abort = null; finish('onabort'); return; }
                if (mode === 'readwrite' && control.writeError) { tx.error = quota(); finish('onabort'); return; }
                events.push(operation);
                req.result = result();
                req.onsuccess?.();
                finish('oncomplete');
              });
              return req;
            };
            tx.objectStore = () => ({
              get: key => request('get', () => entries.get(key)),
              put: (value, key) => request(`put:${key}`, () => { entries.set(key, value); return key; }),
              delete: key => request(`delete:${key}`, () => entries.delete(key)),
              openCursor: range => request('cursor', () => {
                const key = [...entries.keys()].sort().find(key => !range || key > range.after);
                return key === undefined ? null : { key, value: entries.get(key) };
              }),
              getAll() { throw Error('getAll proibido: carrega todos os históricos'); },
              getAllKeys() { throw Error('getAllKeys proibido'); },
            });
            return tx;
          },
        };
        connections.push(db);
        req.result = db;
        if (control.blocked) { control.blocked = false; req.onblocked(); }
        req.onsuccess();
      });
      return req;
    },
  };
  const store = createKeyValueStore({ database: () => database, storage: () => storage,
    keyRange: () => ({ lowerBound: (after, exclusive) => { assert.equal(exclusive, true); return { after }; } }),
    enabled: () => true,
    encode: async text => {
      assert.equal(active, 0, 'crypto fora de transação IDB');
      if (control.cryptoError) throw Error('crypto fixture');
      return { sealed: 1, nonce: 'fixture', data: text };
    },
    decode: async value => value?.sealed ? JSON.parse(value.data) : value,
  });
  // The injected envelope is a persistence fixture; real AES/ABI tests are in private-storage.test.js.
  return { entries, legacy, connections, events, control, store };
}

test('kv: set rejeita quota dupla e migração mantém a única cópia legada', async () => {
  const f = fixture(), source = JSON.stringify({ history: ['fixture-completa'] });
  f.legacy.set('mc.kv.chat.fixture', source);
  f.control.writeError = f.control.quota = true;
  await assert.rejects(f.store.set('chat.fixture', { history: ['novo'] }), AggregateError);
  await assert.rejects(f.store.migratePrivacy(), AggregateError);
  assert.equal(f.legacy.get('mc.kv.chat.fixture'), source);
  assert.equal(f.entries.size, 0);
});

test('kv: migração usa uma entrada/cursor por vez e preserva nomes, históricos e fonte até commit', async () => {
  const f = fixture();
  f.entries.set('chat.a', { history: Array(100).fill('a') });
  f.entries.set('chat.b', { history: Array(200).fill('b') });
  f.legacy.set('mc.kv.chat.legacy', JSON.stringify({ history: ['legacy'] }));
  f.legacy.set('mc.kv.presets', JSON.stringify({ presets: ['legacy-preset'] }));
  await f.store.migratePrivacy();
  assert.deepEqual(f.events.slice(0, 5), ['cursor', 'put:chat.a', 'cursor', 'put:chat.b', 'cursor']);
  assert.equal(f.entries.get('chat.a').sealed, 1);
  assert.equal((await f.store.get('chat.b')).history.length, 200);
  assert.equal(f.entries.has('chat.legacy'), true, 'prefixo mc.kv. tem seis caracteres');
  assert.equal(f.entries.has('hat.legacy'), false);
  assert.deepEqual(await f.store.get('presets'), { presets: ['legacy-preset'] });
  assert.equal(f.legacy.size, 0);
});

test('kv: fallback de migração fica protegido na mesma chave até poder migrar para IDB', async () => {
  const f = fixture();
  f.legacy.set('mc.kv.chat.fixture', JSON.stringify({ history: ['fixture'] }));
  f.control.openError = true;
  await f.store.migratePrivacy();
  assert.equal(JSON.parse(f.legacy.get('mc.kv.chat.fixture')).sealed, 1);
  assert.deepEqual(await f.store.get('chat.fixture'), { history: ['fixture'] });
  f.control.openError = false;
  await f.store.migratePrivacy();
  assert.equal(f.legacy.has('mc.kv.chat.fixture'), false);
  assert.equal(f.entries.get('chat.fixture').sealed, 1);
});

test('kv: falha de crypto não tenta plaintext fallback e não remove fonte', async () => {
  const f = fixture(), source = JSON.stringify({ history: ['preservar'] });
  f.legacy.set('mc.kv.chat.fixture', source);
  f.control.cryptoError = true;
  await assert.rejects(f.store.set('chat.new', { history: ['novo'] }), /crypto fixture/);
  await assert.rejects(f.store.migratePrivacy(), /crypto fixture/);
  assert.equal(f.entries.size, 0);
  assert.equal(f.legacy.size, 1);
  assert.equal(f.legacy.get('mc.kv.chat.fixture'), source);
});

test('kv: quota na migração IDB mantém plaintext original recuperável e rejeita migração', async () => {
  const f = fixture(), source = { history: ['única-cópia-fixture'] };
  f.entries.set('chat.fixture', source);
  f.control.writeError = true;
  await assert.rejects(f.store.migratePrivacy(), /quota fixture/);
  assert.deepEqual(f.entries.get('chat.fixture'), source);
  assert.equal(f.legacy.size, 0);
});

test('kv: onabort sem onerror termina leitura/fallback e rejeita migração sem hang', { timeout: 1000 }, async () => {
  const f = fixture();
  f.control.abort = 'readonly';
  assert.equal(await f.store.get('missing', 'fallback'), 'fallback');
  f.control.abort = 'readonly';
  await assert.rejects(f.store.migratePrivacy(), /interrompida/);
  f.control.abort = 'readwrite';
  await f.store.set('fixture', { value: 1 });
  assert.equal(JSON.parse(f.legacy.get('mc.kv.fixture')).sealed, 1);
});

test('kv: DB fechado/versionchange/onclose libera conexão e reabre; open falho não fica memoizado', async () => {
  const f = fixture();
  f.control.openError = true;
  await f.store.set('fixture', 0);
  f.control.openError = false;
  await f.store.set('fixture', 1);
  assert.equal(f.connections.length, 1);
  f.connections[0].onversionchange();
  assert.equal(f.connections[0].closed, true);
  await f.store.set('fixture', 2);
  f.connections[1].onclose();
  await f.store.set('fixture', 3);
  f.connections[2].closed = true; // Simulate a close before the browser delivered its close event.
  await f.store.set('fixture', 4);
  assert.equal(f.connections.length, 4);
  assert.equal(await f.store.get('fixture'), 4);
});

test('kv: open bloqueado termina fallback, fecha sucesso tardio e permite reabrir', { timeout: 1000 }, async () => {
  const f = fixture();
  f.control.blocked = true;
  await f.store.set('fixture', 1);
  assert.equal(f.connections[0].closed, true);
  assert.equal(JSON.parse(f.legacy.get('mc.kv.fixture')).sealed, 1);
  await f.store.set('fixture', 2);
  assert.equal(f.connections.length, 2);
});

test('kv: del remove IDB e fallback; abort ainda tenta remover a cópia fallback e rejeita', async () => {
  const f = fixture();
  f.entries.set('fixture', { stale: true }); f.legacy.set('mc.kv.fixture', '{"stale":true}');
  await f.store.del('fixture');
  assert.equal(f.entries.has('fixture'), false);
  assert.equal(f.legacy.has('mc.kv.fixture'), false);
  f.legacy.set('mc.kv.fixture', '{"stale":true}');
  f.control.abort = 'readwrite';
  await assert.rejects(f.store.del('fixture'), AggregateError);
  assert.equal(f.legacy.has('mc.kv.fixture'), false);
});

test('kv: fallback confirmado prevalece sobre IDB antigo e próximo put IDB limpa fallback obsoleto', async () => {
  const f = fixture();
  await f.store.set('chat.fixture', { revision: 1 });
  f.control.writeError = true;
  await f.store.set('chat.fixture', { revision: 2 });
  assert.equal(JSON.parse(f.entries.get('chat.fixture').data).revision, 1);
  assert.equal((await f.store.get('chat.fixture')).revision, 2);
  f.control.writeError = false;
  await f.store.set('chat.fixture', { revision: 3 });
  assert.equal(f.legacy.has('mc.kv.chat.fixture'), false);
  assert.equal((await f.store.get('chat.fixture')).revision, 3);
});
