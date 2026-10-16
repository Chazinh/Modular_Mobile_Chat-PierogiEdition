import test from 'node:test';
import assert from 'node:assert/strict';
import { synchronizePairedPc } from '../client/src/pc-pairing.js';
import { createServerConnection } from '../client/src/server-connection.js';

const drain = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
function clock() {
  let id = 0;
  const jobs = new Map();
  return { jobs, setTimeout(fn, ms) { jobs.set(++id, { fn, ms }); return id; }, clearTimeout(id) { jobs.delete(id); },
    async fire(ms) { const entry = [...jobs].find(([, task]) => task.ms === ms); assert.ok(entry, `timer ${ms} exists`);
      jobs.delete(entry[0]); entry[1].fn(); await drain(); } };
}

test('pareamento: PC ocupado aguarda 10s e promove somente depois da sincronização real', async () => {
  const values = new Map(), timers = clock(), waits = [], syncTokens = [];
  let attempts = 0;
  const host = { server: 'http://127.0.0.1:3011', kind: 'local', token: 'android-only' };
  const connection = createServerConnection({ mode: 'auto', native: () => true, token: () => 'already-paired-pc',
    storage: { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) },
    embedded: { target: () => host, standby: async () => {}, activate: async () => {} },
    fetch: async () => Response.json({ ok: true, name: 'modularchat', encrypted: true }),
    sync: async ({ pc }) => { syncTokens.push(pc.token); if (++attempts < 3) throw Object.assign(Error('job ativo'), { code: 'BUSY' }); return { converged: true }; },
  });
  connection.configure({ local: 'http://192.168.1.6:3000' });
  let complete = false;
  const pending = synchronizePairedPc({ connect: signal => connection.switchToPc(signal), timers, onWaiting: () => waits.push('busy') }).then(target => { complete = true; return target; });
  await drain(); assert.equal(attempts, 1); assert.equal(complete, false);
  assert.equal(connection.kind(), 'local');
  await timers.fire(10000); assert.equal(attempts, 2); assert.equal(complete, false);
  await timers.fire(10000);
  assert.equal((await pending).kind, 'pc'); assert.equal(connection.kind(), 'pc');
  assert.deepEqual(waits, ['busy', 'busy']);
  assert.deepEqual(syncTokens, Array(3).fill('already-paired-pc'));
  assert.equal(timers.jobs.size, 0);
});

test('pareamento: cancelar espera não repete sincronização e limpa timers', async () => {
  const timers = clock(), controller = new AbortController(); let attempts = 0;
  const pending = synchronizePairedPc({ timers, signal: controller.signal,
    connect: async () => { attempts++; throw Object.assign(Error('ativo'), { code: 'BUSY' }); } });
  await drain(); assert.equal(attempts, 1);
  controller.abort(); await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(timers.jobs.size, 0); assert.equal(attempts, 1);
});

test('pareamento: conflito e autenticação não são repetidos, e espera ocupada tem prazo', async () => {
  for (const code of ['CONFLICT', 'UNPAIRED', 'ORIGIN_CHANGED']) {
    const timers = clock(); let attempts = 0;
    await assert.rejects(synchronizePairedPc({ timers, connect: async () => { attempts++; throw Object.assign(Error(code), { code }); } }), { code });
    assert.equal(attempts, 1); assert.equal(timers.jobs.size, 0);
  }
  const timers = clock();
  const pending = synchronizePairedPc({ timers, connect: async () => { throw Object.assign(Error('ativo'), { code: 'BUSY' }); } });
  await drain();
  const rejected = assert.rejects(pending, { code: 'SYNC_WAIT_TIMEOUT' });
  await timers.fire(300000); await rejected;
  assert.equal(timers.jobs.size, 0);
});

test('pareamento: cancelar durante resposta impede conclusão tardia e deadline limpa', async () => {
  const timers = clock(), controller = new AbortController(); let finish;
  const pending = synchronizePairedPc({ timers, signal: controller.signal, connect: () => new Promise(resolve => { finish = resolve; }) });
  controller.abort(); finish({ kind: 'pc' });
  await assert.rejects(pending, { name: 'AbortError' }); assert.equal(timers.jobs.size, 0);
});
