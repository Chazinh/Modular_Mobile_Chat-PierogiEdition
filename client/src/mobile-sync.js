import { privateGet, privateSet, privacyEnabled } from './private-storage.js';
import { normalizeServer } from './server-address.js';

const MAX_BYTES = 8 * 1024 * 1024;
const hashValid = value => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const idValid = value => typeof value === 'string' && /^[0-9a-z_-]{1,128}$/i.test(value);
const uuidValid = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value) && !/^0{8}-0{4}-0{4}-0{4}-0{12}$/.test(value);
const bytes = value => new TextEncoder().encode(typeof value === 'string' ? value : JSON.stringify(value)).byteLength;

export class SyncError extends Error {
  constructor(code, message, data = {}, status = 409) {
    super(message); this.code = code; this.data = data; this.status = status;
  }
}

// All network calls use the two captured targets directly; no API/connection resolver recursion.
export function createMobileSync({ fetch: send, store, uuid = () => crypto.randomUUID() }) {
  let queue = Promise.resolve();
  const fail = (code, message) => { throw new SyncError(code, message); };
  const envelope = (peerId, convIds = []) => ({ format: 'modularchat-sync', version: 1, peerId, known: [], convIds, after: 0, limit: 4 });

  async function run({ pc, mobile, signal, isCurrent = () => true, pauseMemory = true }) {
    const check = () => {
      signal?.throwIfAborted();
      if (!isCurrent()) fail('STALE_CONNECTION', 'A configuração mudou durante a sincronização.');
    };
    for (const target of [pc, mobile]) {
      let validServer = false;
      try { validServer = normalizeServer(target.server, false) === target.server; } catch { /* Refuse an invalid/public HTTP target before sending its credential. */ }
      if (!target.token || !validServer || (target.kind === 'local' && target.server !== 'http://127.0.0.1:3011')) {
        fail('INVALID_TARGET', 'A sincronização exige destinos fixos e credenciais independentes.');
      }
    }
    async function call(target, path, body) {
      check();
      if (body && bytes(body) > MAX_BYTES) fail('SYNC_TOO_LARGE', 'O lote de sincronização excede 8 MiB.');
      const timeout = AbortSignal.timeout(15000);
      const observe = !pauseMemory && path.startsWith('status?') ? '&observe=1' : '';
      const response = await send(`${target.server}/api/sync/${path}${observe}`, {
        method: body ? 'POST' : 'GET', redirect: 'error', cache: 'no-store', credentials: 'omit',
        signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
        headers: { Authorization: `Bearer ${target.token}`, ...(body ? { 'Content-Type': 'application/json' } : {}) },
        body: body ? JSON.stringify(body) : undefined,
      });
      const text = await response.text();
      check();
      if (bytes(text) > MAX_BYTES) fail('SYNC_TOO_LARGE', 'A resposta de sincronização excede 8 MiB.');
      let data;
      try { data = JSON.parse(text); } catch { fail('INVALID_SYNC', 'O servidor retornou uma resposta de sincronização inválida.'); }
      if (!response.ok) throw new SyncError(data.code || (response.status === 401 ? 'UNPAIRED' : 'SYNC_HTTP'), data.error ||
        (data.code === 'CONFLICT' ? 'A sincronização encontrou alterações conflitantes. As duas versões foram preservadas.' : `Sincronização: HTTP ${response.status}`), data, response.status);
      if (data.format !== 'modularchat-sync' || data.version !== 1 || !uuidValid(data.serverId)
          || data.schemaVersion !== 4 || data.maxBytes !== MAX_BYTES || data.maxConversations !== 4) {
        fail('INVALID_SYNC', 'O contrato de sincronização do servidor é incompatível.');
      }
      if (data.code || data.conflicts?.length) throw new SyncError(data.code || 'CONFLICT', 'As conversas possuem alterações conflitantes.', data);
      return data;
    }
    async function status(target, expectedId) {
      let after = 0, header;
      const manifests = new Map();
      for (;;) {
        const data = await call(target, `status?after=${after}&limit=128`);
        if ((expectedId && data.serverId !== expectedId) || (header && (data.serverId !== header.serverId || data.cursor !== header.cursor))) {
          fail('STALE_SYNC', 'O cofre mudou durante a leitura dos manifestos.');
        }
        if (!Array.isArray(data.manifests) || data.manifests.length > 128 || typeof data.hasMore !== 'boolean' || typeof data.embeddingFingerprint !== 'string'
            || !Number.isSafeInteger(data.cursor) || !Number.isSafeInteger(data.nextCursor) || data.nextCursor < after || data.nextCursor > data.cursor) {
          fail('INVALID_PAGINATION', 'A paginação dos manifestos está incompleta.');
        }
        for (const entry of data.manifests) {
          if (!idValid(entry.convId) || !hashValid(entry.hash) || !hashValid(entry.contentHash) || manifests.has(entry.convId)
              || !Number.isSafeInteger(entry.cursor) || entry.cursor <= after || entry.cursor > data.nextCursor
              || !Number.isSafeInteger(entry.revision) || typeof entry.deleted !== 'boolean' || typeof entry.purged !== 'boolean' || typeof entry.busy !== 'boolean') {
            fail('INVALID_SYNC', 'Um manifesto de conversa é inválido ou duplicado.');
          }
          if (entry.busy) throw new SyncError('BUSY', 'Uma geração ou job está ativo; a sincronização será tentada novamente em 10 segundos.', { convId: entry.convId, retryable: true });
          manifests.set(entry.convId, entry);
        }
        header = data;
        if (!data.hasMore) return { ...header, manifests };
        if (data.nextCursor <= after || !data.manifests.length) fail('INVALID_PAGINATION', 'O cursor da sincronização não avançou.');
        after = data.nextCursor;
      }
    }
    async function offer(source, sourceId, peerId, ids) {
      const result = new Map();
      for (let start = 0; start < ids.length; start += 4) {
        const convIds = ids.slice(start, start + 4);
        let after = 0;
        for (;;) {
          const data = await call(source, 'delta', { ...envelope(peerId, convIds), after });
          if (data.serverId !== sourceId || !Array.isArray(data.snapshots) || data.snapshots.length > 4
              || typeof data.hasMore !== 'boolean' || !Number.isSafeInteger(data.nextCursor) || data.nextCursor < after) {
            fail('INVALID_PAGINATION', 'A oferta de snapshots está incompleta.');
          }
          for (const entry of data.snapshots) {
            if (!convIds.includes(entry.convId) || result.has(entry.convId) || !hashValid(entry.hash) || !hashValid(entry.contentHash)
                || entry.snapshot?.convId !== entry.convId || entry.snapshot.full !== true) fail('INVALID_SYNC', 'Snapshot de conversa inválido.');
            if (bytes(entry.snapshot) > MAX_BYTES) fail('SYNC_TOO_LARGE', 'Uma conversa excede 8 MiB; ela foi preservada sem truncagem.');
            result.set(entry.convId, entry);
          }
          if (!data.hasMore) break;
          if (data.nextCursor <= after) fail('INVALID_PAGINATION', 'A oferta de snapshots não avançou.');
          after = data.nextCursor;
        }
        if (convIds.some(id => !result.has(id))) fail('MISSING_CONVERSATIONS', 'Uma conversa não foi oferecida pelo servidor de origem.');
      }
      return result;
    }

    let pcState = await status(pc), mobileState = await status(mobile);
    if (pcState.serverId === mobileState.serverId) fail('INVALID_TARGET', 'Os dois destinos identificam o mesmo cofre.');
    const pcId = pcState.serverId, mobileId = mobileState.serverId;
    const key = `mc.sync.${pcId}.${mobileId}`;
    const saved = await store.get(key);
    let stored = saved;
    let state;
    try { state = saved ? JSON.parse(saved) : { version: 1, pcId, mobileId, baselines: {}, pending: null }; }
    catch { fail('INVALID_BASELINE', 'Não foi possível ler o checkpoint privado de sincronização.'); }
    if (state.version !== 1 || state.pcId !== pcId || state.mobileId !== mobileId || !state.baselines) fail('INVALID_BASELINE', 'O checkpoint não pertence a estes cofres.');
    const persist = async () => {
      check();
      const value = JSON.stringify(state);
      if (value !== stored) { await store.set(key, value); stored = value; }
      check();
    };
    const compatible = () => typeof pcState.embeddingFingerprint === 'string' && pcState.embeddingFingerprint === mobileState.embeddingFingerprint;
    let skippedVectors = 0;

    async function applyBatch(dest, sourceId, destId, records, entries, batchId) {
      const pushes = records.map(record => {
        const entry = entries.get(record.convId);
        if (!entry || entry.hash !== record.hash) fail('PENDING_BATCH_CHANGED', 'O snapshot de um lote pendente mudou. Preserve a pendência para recuperação.');
        return { convId: record.convId, baseHash: record.baseHash, hash: record.hash, snapshot: entry.snapshot };
      });
      const body = { ...envelope(sourceId, records.map(r => r.convId)), pushes, batchId };
      const ack = await call(dest, 'delta', body);
      if (ack.serverId !== destId || ack.ackBatch !== batchId || !Array.isArray(ack.accepted) || ack.accepted.length !== records.length
          || records.some(r => !ack.accepted.some(a => a.convId === r.convId && hashValid(a.hash)))) {
        fail('INCOMPLETE_ACK', 'O destino não confirmou todas as conversas do lote.');
      }
      skippedVectors += ack.ignoredEmbeddings?.length || 0;
    }

    async function commit(dest, sourceId, destId, records, entries, to, batchId = uuid()) {
      // Verify recovery before replacing any private receipt.
      for (const record of records) {
        if (entries.get(record.convId)?.hash !== record.hash) fail('PENDING_BATCH_CHANGED', 'O snapshot de um lote pendente mudou. Preserve a pendência para recuperação.');
      }
      state.pending = { to, batchId, records };
      await persist();
      try { await applyBatch(dest, sourceId, destId, records, entries, batchId); }
      catch (error) {
        const data = error.data;
        const separate = records.length > 1 && error.status === 409 && error.code === 'CONFLICT' && data?.serverId === destId
          && data.format === 'modularchat-sync' && data.version === 1 && data.schemaVersion === 4
          && data.maxBytes === MAX_BYTES && data.maxConversations === 4 && data.ackBatch === null
          && Array.isArray(data.accepted) && data.accepted.length === 0
          && Array.isArray(data.conflicts) && data.conflicts.length > 0
          && data.conflicts.every(c => c.reason === 'PUBLIC_SETTINGS_BATCH' && records.some(r => r.convId === c.convId));
        if (separate) {
          // This specific response confirms the whole batch was rejected atomically.
          // Single-conversation batches let the server reconcile global defaults in order.
          state.pending = null;
          await persist();
          // Reconcile an existing baseline first; new conversations then inherit those defaults.
          const ordered = [...records].sort((a, b) => Number(a.baseHash == null) - Number(b.baseHash == null));
          for (const record of ordered) await commit(dest, sourceId, destId, [record], entries, to);
          return;
        }
        if (error.code === 'CONFLICT') {
          state.pending.code = 'CONFLICT';
          state.pending.conflicts = (data?.conflicts || []).map(c => ({ convId: c.convId, reason: c.reason }));
          await persist();
        }
        throw error;
      }
      state.pending = null;
      await persist();
    }
    // Only metadata is persisted. Recover exact source snapshots from the encrypted Store.
    if (state.pending) {
      const pending = state.pending;
      const toPc = pending.to === 'pc';
      const source = toPc ? mobile : pc, dest = toPc ? pc : mobile;
      const sourceId = toPc ? mobileId : pcId, destId = toPc ? pcId : mobileId;
      const destState = toPc ? pcState : mobileState;
      const isRecordResolved = r => {
        const manifest = destState.manifests.get(r.convId);
        if (!manifest) return false;
        if (manifest.purged) {
          const srcManifest = (toPc ? mobileState : pcState).manifests.get(r.convId);
          if (srcManifest?.purged) return true;
        }
        return compatible() ? manifest.hash === r.hash : manifest.contentHash === r.contentHash;
      };
      const alreadyApplied = pending.records.every(isRecordResolved);
      if (alreadyApplied) {
        state.pending = null;
        await persist();
      } else {
        if (pending.code === 'CONFLICT' && (pending.records.length < 2 || pending.conflicts !== undefined)) {
          throw new SyncError('CONFLICT', 'Um lote conflitante foi preservado; resolva o conflito antes de promover o PC.');
        }
        const entries = await offer(source, sourceId, destId, pending.records.map(r => r.convId));
        // Legacy multi-conversation conflicts did not save reasons. An exact idempotent replay
        // retrieves the recorded rejection so only PUBLIC_SETTINGS_BATCH can be separated.
        await commit(dest, sourceId, destId, pending.records, entries, pending.to, pending.batchId);
      }
      pcState = await status(pc, pcId); mobileState = await status(mobile, mobileId);
    }

    async function transfer(source, dest, sourceId, destId, ids, bases, to) {
      // At most four conversations' snapshots are retained in memory at a time.
      for (let start = 0; start < ids.length; start += 4) {
        const group = ids.slice(start, start + 4);
        const entries = await offer(source, sourceId, destId, group);
        let records = [];
        const flush = async () => {
          if (!records.length) return;
          await commit(dest, sourceId, destId, records, entries, to);
          records = [];
        };
        for (const id of group) {
          const entry = entries.get(id);
          const record = { convId: id, hash: entry.hash, contentHash: entry.contentHash, baseHash: bases.get(id) ?? null };
          const prospective = [...records, record];
          const size = bytes({ ...envelope(sourceId, prospective.map(r => r.convId)), batchId: '00000000-0000-4000-8000-000000000001',
            pushes: prospective.map(r => ({ convId: r.convId, baseHash: r.baseHash, hash: r.hash, snapshot: entries.get(r.convId).snapshot })) });
          if (records.length && (records.length === 4 || size > MAX_BYTES)) await flush();
          records.push(record);
        }
        await flush();
      }
    }

    const pushIds = [], pushBases = new Map();
    const ids = [...new Set([...pcState.manifests.keys(), ...mobileState.manifests.keys()])].sort();
    for (const id of ids) {
      const p = pcState.manifests.get(id), m = mobileState.manifests.get(id), base = Object.hasOwn(state.baselines, id) ? state.baselines[id] : null;
      if (base && (!p || !m)) fail('MISSING_CONVERSATIONS', 'Uma conversa da baseline desapareceu de um dos cofres.');
      if (!m) continue;
      if (!p) { pushIds.push(id); continue; }
      if (p.purged && m.purged) continue;
      const equal = compatible() ? p.hash === m.hash : p.contentHash === m.contentHash;
      if (equal) continue;
      if (!base) fail('CONFLICT', 'Há conversas diferentes com o mesmo ID e nenhuma baseline compartilhada.');
      const mobileChanged = compatible() ? m.hash !== base.mobileHash : m.contentHash !== base.contentHash;
      if (mobileChanged) { pushIds.push(id); pushBases.set(id, base.pcHash); }
    }
    if (pushIds.length) {
      await transfer(mobile, pc, mobileId, pcId, pushIds, pushBases, 'pc');
      pcState = await status(pc, pcId); mobileState = await status(mobile, mobileId);
    }
    const pullIds = [], pullBases = new Map();
    for (const [id, p] of pcState.manifests) {
      const m = mobileState.manifests.get(id);
      if (p.purged && m?.purged) continue;
      if (!m || (compatible() ? m.hash !== p.hash : m.contentHash !== p.contentHash)) {
        pullIds.push(id);
        if (m) pullBases.set(id, Object.hasOwn(state.baselines, id) ? state.baselines[id].mobileHash : m.hash);
      }
    }
    if (pullIds.length) await transfer(pc, mobile, pcId, mobileId, pullIds, pullBases, 'mobile');
    pcState = await status(pc, pcId); mobileState = await status(mobile, mobileId);
    if (pcState.manifests.size !== mobileState.manifests.size) fail('MISSING_CONVERSATIONS', 'A sincronização ainda não contém todas as conversas.');
    const baselines = Object.create(null);
    for (const [id, p] of pcState.manifests) {
      const m = mobileState.manifests.get(id);
      if (p.purged && m?.purged) {
        baselines[id] = { pcHash: p.hash, mobileHash: m.hash, contentHash: p.contentHash };
        continue;
      }
      if (!m || p.contentHash !== m.contentHash || (compatible() && p.hash !== m.hash)
          || p.deleted !== m.deleted || p.purged !== m.purged) fail('HASH_DIVERGENCE', 'Os cofres ainda não convergiram; o telefone permanece local.');
      baselines[id] = { pcHash: p.hash, mobileHash: m.hash, contentHash: p.contentHash };
    }
    state.baselines = baselines;
    await persist();
    return { converged: true, pcId, mobileId, conversations: baselines, skippedVectors, compatibleEmbeddings: compatible() };
  }
  return {
    synchronize(options) {
      // A rejected batch never poisons the exclusive queue or discards its private receipt.
      const task = queue.catch(() => {}).then(() => run(options));
      queue = task;
      return task;
    },
  };
}

export const mobileSync = createMobileSync({
  fetch: (...args) => fetch(...args),
  store: {
    get: key => privateGet(key),
    set: (key, value) => {
      if (!privacyEnabled()) throw new SyncError('PRIVACY_REQUIRED', 'Abra o armazenamento protegido antes de sincronizar.');
      return privateSet(key, value);
    },
  },
});
