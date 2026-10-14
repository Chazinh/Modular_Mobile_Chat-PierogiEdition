import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID, createHash } from 'node:crypto';
import { existsSync, mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createMobileSync } from '../client/src/mobile-sync.js';
import { RustStore } from './rust-store.js';

const MAX_BYTES = 8 * 1024 * 1024;
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const pc = { server: 'https://pc.example.com:8443', token: 'pc-private', kind: 'pc' };
const mobile = { server: 'http://127.0.0.1:3011', token: 'android-keystore', kind: 'local' };
const clone = value => JSON.parse(JSON.stringify(value));
function markers() {
  const values = new Map();
  return { values, get: async key => values.get(key), set: async (key, value) => { values.set(key, value); } };
}

// Deterministic protocol fixtures. Only Rust (below) exercises the canonical hash and merge implementation.
function fixture(options = {}) {
  const store = markers(), calls = [];
  const states = {
    pc: { serverId: randomUUID(), cursor: 0, rows: new Map(), batches: new Map(), fingerprint: options.pcFingerprint || 'same-model' },
    mobile: { serverId: randomUUID(), cursor: 0, rows: new Map(), batches: new Map(), fingerprint: options.mobileFingerprint || 'same-model' },
  };
  const header = state => ({ format: 'modularchat-sync', version: 1, schemaVersion: 4, serverId: state.serverId, cursor: state.cursor,
    maxBytes: MAX_BYTES, maxConversations: 4, embeddingFingerprint: state.fingerprint });
  const manifest = row => ({ convId: row.convId, hash: row.hash, contentHash: row.contentHash, revision: row.revision,
    cursor: row.cursor, deleted: false, purged: false, busy: !!row.busy });
  const put = (kind, id, name, embeddings = []) => {
    const state = states[kind];
    const snapshot = { full: true, purged: false, convId: id, conversation: { id, name }, publicSettings: {}, embeddings };
    const row = { convId: id, hash: hash(snapshot), contentHash: hash({ ...snapshot, embeddings: [] }), snapshot,
      cursor: ++state.cursor, revision: (state.rows.get(id)?.revision || 0) + 1 };
    state.rows.set(id, row);
    return row;
  };
  const send = async (url, request) => {
    const parsed = new URL(url), kind = parsed.origin === pc.server ? 'pc' : 'mobile', state = states[kind];
    assert.equal(request.headers.Authorization, `Bearer ${kind === 'pc' ? pc.token : mobile.token}`);
    assert.equal(request.redirect, 'error');
    const body = request.body ? JSON.parse(request.body) : undefined;
    calls.push({ kind, url, body, request });
    const override = await options.onCall?.({ kind, state, body, url: parsed, header: header(state) });
    if (override) return override;
    if (parsed.pathname.endsWith('/status')) {
      assert.equal(parsed.searchParams.get('limit'), '128');
      const after = Number(parsed.searchParams.get('after'));
      const all = [...state.rows.values()].filter(r => r.cursor > after).sort((a, b) => a.cursor - b.cursor);
      const page = all.slice(0, options.statusPage || 128);
      return Response.json({ ...header(state), manifests: page.map(manifest), nextCursor: page.at(-1)?.cursor || after, hasMore: all.length > page.length });
    }
    assert.equal(request.method, 'POST');
    assert.equal(body.peerId, states[kind === 'pc' ? 'mobile' : 'pc'].serverId);
    assert.ok(body.convIds.length <= 4);
    assert.equal(body.limit, 4);
    assert.ok(Buffer.byteLength(request.body) <= MAX_BYTES);
    const ignoredEmbeddings = [], accepted = [];
    if (body.pushes?.length) {
      const previous = state.batches.get(body.batchId);
      if (previous) {
        assert.deepEqual(body, previous.body, 'replay usa exatamente o lote anterior');
        return Response.json(previous.response);
      }
      for (const push of body.pushes) {
        const current = state.rows.get(push.convId);
        if (current && current.hash !== push.baseHash && current.hash !== push.hash) return Response.json({ error: 'conflito', code: 'CONFLICT' }, { status: 409 });
      }
      for (const push of body.pushes) {
        const snapshot = clone(push.snapshot);
        if (state.fingerprint !== states[kind === 'pc' ? 'mobile' : 'pc'].fingerprint && snapshot.embeddings.length) {
          snapshot.embeddings = [];
          ignoredEmbeddings.push({ convId: push.convId, reason: 'fingerprint' });
        }
        const row = put(kind, push.convId, snapshot.conversation.name, snapshot.embeddings);
        accepted.push(manifest(row));
      }
    }
    const after = body.after || 0;
    const all = [...state.rows.values()].filter(r => r.cursor > after && (!body.convIds.length || body.convIds.includes(r.convId))).sort((a, b) => a.cursor - b.cursor);
    const page = all.slice(0, options.offerPage || 4);
    const response = { ...header(state), ackBatch: body.pushes?.length ? body.batchId : null, accepted, conflicts: [], ignoredEmbeddings,
      manifests: page.map(manifest), snapshots: page.map(({ convId, hash, contentHash, snapshot }) => ({ convId, hash, contentHash, snapshot })),
      nextCursor: page.at(-1)?.cursor || after, hasMore: all.length > page.length };
    if (body.pushes?.length) state.batches.set(body.batchId, { body: clone(body), response: clone(response) });
    return Response.json(response);
  };
  return { states, store, calls, put, sync: createMobileSync({ fetch: send, store }), options: { pc, mobile } };
}

test('sync: mirror inicial PC→mobile, IDs preservados, incremental por hash e offline bidirecional', async () => {
  const f = fixture();
  f.put('pc', 'stable-conv', 'PRIVATE-TURN');
  const initial = await f.sync.synchronize(f.options);
  assert.equal(initial.converged, true);
  assert.equal(f.states.mobile.rows.get('stable-conv').snapshot.conversation.id, 'stable-conv');
  const initialPushes = f.calls.filter(c => c.body?.pushes).length;
  await f.sync.synchronize(f.options);
  assert.equal(f.calls.filter(c => c.body?.pushes).length, initialPushes, 'não oferece snapshots inalterados');
  f.put('mobile', 'stable-conv', 'offline');
  f.put('mobile', 'mobile-created', 'novo');
  f.put('pc', 'pc-created', 'PC novo');
  await f.sync.synchronize(f.options);
  for (const id of ['stable-conv', 'mobile-created', 'pc-created']) assert.equal(f.states.mobile.rows.get(id).hash, f.states.pc.rows.get(id).hash);
  const marker = [...f.store.values.values()][0];
  assert.equal(marker.includes('PRIVATE-TURN'), false);
  assert.equal(marker.includes('snapshot'), false);
  assert.equal(marker.includes(pc.token), false);
  assert.equal(marker.includes(mobile.token), false);
});

test('sync: paginação completa em status (130 conversas) e delta, lotes de até quatro', async () => {
  const f = fixture({ offerPage: 2 });
  for (let i = 0; i < 130; i++) f.put('pc', `conv-${i}`, `Conversa ${i}`);
  await f.sync.synchronize(f.options);
  assert.equal(f.states.mobile.rows.size, 130);
  assert.ok(f.calls.some(c => c.url.includes('/status?after=128&limit=128')));
  assert.ok(f.calls.some(c => c.body?.after > 0));
  assert.ok(f.calls.filter(c => c.body).every(c => c.body.convIds.length <= 4));
});

test('sync: conflito/BUSY/401 não descartam dados; conflito de push persiste recibo mínimo', async () => {
  for (const [code, status] of [['BUSY', 409], ['UNPAIRED', 401], ['CONFLICT', 409]]) {
    const f = fixture({ onCall: ({ body }) => body?.pushes ? Response.json({ code, error: code }, { status }) : null });
    f.put('pc', 'c', 'original');
    await assert.rejects(f.sync.synchronize(f.options), { code });
    assert.equal(f.states.pc.rows.get('c').snapshot.conversation.name, 'original');
    assert.equal(f.states.mobile.rows.size, 0);
    const pending = JSON.parse([...f.store.values.values()][0]).pending;
    assert.equal(pending.records[0].convId, 'c');
    assert.equal(pending.records[0].snapshot, undefined);
    if (code === 'CONFLICT') {
      const before = f.calls.length;
      await assert.rejects(f.sync.synchronize(f.options), { code: 'CONFLICT' });
      assert.equal(f.calls.slice(before).filter(c => c.body?.pushes).length, 0, 'conflito não é reenviado com o mesmo batchId');
    }
  }
  const busy = fixture();
  busy.put('pc', 'busy-conv', 'ocupada').busy = true;
  await assert.rejects(busy.sync.synchronize(busy.options), { code: 'BUSY' });
  assert.equal(busy.calls.some(c => c.body), false);
});

test('sync: fila exclusiva e replay exato de batch privado após rede voltar', async () => {
  let lost = true;
  const f = fixture({ onCall: ({ body }) => { if (body?.pushes && lost) { lost = false; throw Error('rede caiu antes do ack'); } } });
  f.put('pc', 'replay', 'original');
  await assert.rejects(f.sync.synchronize(f.options), /rede caiu/);
  const originalBatch = JSON.parse([...f.store.values.values()][0]).pending.batchId;
  const results = await Promise.all([f.sync.synchronize(f.options), f.sync.synchronize(f.options)]);
  assert.ok(results.every(r => r.converged));
  const pushes = f.calls.filter(c => c.body?.pushes);
  assert.equal(pushes.length, 2);
  assert.equal(pushes[1].body.batchId, originalBatch);
  assert.deepEqual(pushes[0].body, pushes[1].body);
  assert.equal(JSON.parse([...f.store.values.values()][0]).pending, null);
});

test('sync: snapshot alterado não reutiliza batchId nem perde pendência', async () => {
  let lost = true;
  const f = fixture({ onCall: ({ body }) => { if (body?.pushes && lost) { lost = false; throw Error('lost'); } } });
  f.put('pc', 'c', 'primeiro');
  await assert.rejects(f.sync.synchronize(f.options));
  const marker = [...f.store.values.values()][0];
  f.put('pc', 'c', 'segundo');
  await assert.rejects(f.sync.synchronize(f.options), { code: 'PENDING_BATCH_CHANGED' });
  assert.equal([...f.store.values.values()][0], marker);
  assert.equal(f.calls.filter(c => c.body?.pushes).length, 1);
});

test('sync: embeddings incompatíveis convergem por contentHash e reportam vetores ignorados', async () => {
  const f = fixture({ pcFingerprint: 'pc-model', mobileFingerprint: 'mobile-model' });
  f.put('pc', 'vectors', 'texto', [{ model: 'pc-model', vec: 'AAAA' }]);
  const result = await f.sync.synchronize(f.options);
  assert.equal(result.compatibleEmbeddings, false);
  assert.equal(result.skippedVectors, 1);
  assert.notEqual(f.states.pc.rows.get('vectors').hash, f.states.mobile.rows.get('vectors').hash);
  assert.equal(f.states.pc.rows.get('vectors').contentHash, f.states.mobile.rows.get('vectors').contentHash);
  const pushes = f.calls.filter(c => c.body?.pushes).length;
  await f.sync.synchronize(f.options);
  assert.equal(f.calls.filter(c => c.body?.pushes).length, pushes);
});

test('sync: cursor inválido, conversa ausente e configuração obsoleta impedem promoção', async () => {
  const invalid = fixture({ onCall: ({ url, header }) => url.pathname.endsWith('/status') ? Response.json({ ...header, manifests: [], hasMore: true, nextCursor: 0 }) : null });
  await assert.rejects(invalid.sync.synchronize(invalid.options), { code: 'INVALID_PAGINATION' });
  const missing = fixture({ onCall: ({ body, header }) => body && !body.pushes ? Response.json({ ...header, snapshots: [], hasMore: false, nextCursor: 0 }) : null });
  missing.put('pc', 'c', 'original');
  await assert.rejects(missing.sync.synchronize(missing.options), { code: 'MISSING_CONVERSATIONS' });
  const stale = fixture();
  await assert.rejects(stale.sync.synchronize({ ...stale.options, isCurrent: () => false }), { code: 'STALE_CONNECTION' });
  assert.equal(stale.calls.length, 0);
});

test('sync: conversa acima de 8MiB é recusada sem truncagem ou apply', async () => {
  const f = fixture();
  f.put('pc', 'large', 'x'.repeat(MAX_BYTES + 1));
  await assert.rejects(f.sync.synchronize(f.options), { code: 'SYNC_TOO_LARGE' });
  assert.equal(f.calls.some(c => c.body?.pushes), false);
  assert.equal(f.states.mobile.rows.size, 0);
});

test('sync: lote agregado acima de 8MiB é dividido, sem truncar nenhuma conversa', async () => {
  const f = fixture({ offerPage: 1 });
  for (let i = 0; i < 4; i++) f.put('pc', `big-${i}`, 'x'.repeat(3 * 1024 * 1024));
  await f.sync.synchronize(f.options);
  const pushes = f.calls.filter(c => c.body?.pushes);
  assert.equal(pushes.length, 2);
  assert.ok(pushes.every(c => c.body.pushes.length === 2 && Buffer.byteLength(c.request.body) <= MAX_BYTES));
  assert.equal(f.states.mobile.rows.size, 4);
});

test('sync: hash divergente com fingerprint compatível e conversa perdida bloqueiam convergência', async () => {
  let changed = false;
  const f = fixture({ onCall: ({ kind, state, body }) => {
    if (!body && kind === 'mobile' && state.rows.size && !changed) { state.rows.get('c').hash = 'f'.repeat(64); changed = true; }
  } });
  f.put('pc', 'c', 'original');
  await assert.rejects(f.sync.synchronize(f.options), { code: 'HASH_DIVERGENCE' });
  const lost = fixture();
  lost.put('pc', 'c', 'original');
  await lost.sync.synchronize(lost.options);
  lost.states.mobile.rows.delete('c');
  await assert.rejects(lost.sync.synchronize(lost.options), { code: 'MISSING_CONVERSATIONS' });
});

test('sync Rust real: Stores criptografados, baseline, merge offline, pullback e IDs', {
  skip: !process.env.MC_CONTRACT_EXE || !existsSync(process.env.MC_CONTRACT_EXE), timeout: 60000,
}, async () => {
  const tempRoot = path.join(os.tmpdir(), 'Cline', 'modularchat-autohost-v4-client');
  mkdirSync(tempRoot, { recursive: true });
  const dir = mkdtempSync(path.join(tempRoot, 'rust-sync-'));
  const previousEncryption = process.env.MC_ENCRYPTED_STORAGE;
  process.env.MC_ENCRYPTED_STORAGE = '1';
  const pcStore = new RustStore(path.join(dir, 'pc', 'fixture.sqlite'));
  const mobileStore = new RustStore(path.join(dir, 'mobile', 'fixture.sqlite'));
  if (previousEncryption === undefined) delete process.env.MC_ENCRYPTED_STORAGE; else process.env.MC_ENCRYPTED_STORAGE = previousEncryption;
  const fingerprint = 'a'.repeat(64), store = markers();
  let dropAck = false, droppedBatch, replays = 0;
  const sync = createMobileSync({ store, fetch: async (url, options) => {
    const target = url.startsWith(pc.server) ? pcStore : mobileStore;
    assert.equal(options.headers.Authorization, `Bearer ${target === pcStore ? pc.token : mobile.token}`);
    const query = new URL(url).searchParams;
    const body = options.body ? JSON.parse(options.body) : null;
    try {
      const data = body ? await target.call('syncDelta', body, fingerprint)
        : { ...await target.call('syncStatus', { after: Number(query.get('after')), limit: Number(query.get('limit')) }), embeddingFingerprint: fingerprint };
      if (body?.batchId && body.batchId === droppedBatch) replays++;
      if (dropAck && target === pcStore && body?.pushes) { dropAck = false; droppedBatch = body.batchId; throw Error('ack perdido após salvar cofre'); }
      return Response.json(data, { status: data.code === 'CONFLICT' ? 409 : 200 });
    } catch (error) {
      if (!error.status) throw error;
      return Response.json({ error: error.message, ...error.extra, code: error.code || error.extra?.code }, { status: error.status });
    }
  } });
  try {
    await Promise.all([pcStore.ready(), mobileStore.ready()]);
    const chat = await pcStore.call('createConversation', { name: 'Inicial', prompt: 'Ficha da fixture', routeId: 'fixture-route', presetId: 'generic' });
    await sync.synchronize({ pc, mobile });
    assert.equal((await mobileStore.call('getConversation', chat.id)).id, chat.id);
    await mobileStore.call('patchConversation', chat.id, { name: 'Offline' });
    await pcStore.call('patchConversation', chat.id, { temperature: 0.85 });
    const created = await mobileStore.call('createConversation', { name: 'Só telefone', prompt: 'Outra ficha', routeId: 'fixture-route', presetId: 'generic' });
    dropAck = true;
    await assert.rejects(sync.synchronize({ pc, mobile }), /ack perdido/);
    assert.ok(JSON.parse([...store.values.values()][0]).pending);
    const result = await sync.synchronize({ pc, mobile });
    assert.equal(result.converged, true);
    assert.equal(replays, 1, 'Rust reproduziu o ack persistido com o mesmo batchId');
    const p = await pcStore.call('getConversation', chat.id), m = await mobileStore.call('getConversation', chat.id);
    assert.equal(p.name, 'Offline');
    assert.equal(m.name, p.name);
    assert.equal(m.temperature, p.temperature);
    assert.equal(m.temperature, 0.85);
    assert.equal((await pcStore.call('getConversation', created.id)).id, created.id);
    await sync.synchronize({ pc, mobile });
  } finally {
    await Promise.allSettled([pcStore.close(), mobileStore.close()]);
    const resolved = path.resolve(dir);
    assert.ok(resolved.startsWith(path.resolve(tempRoot) + path.sep));
    rmSync(resolved, { recursive: true, force: true });
  }
});
