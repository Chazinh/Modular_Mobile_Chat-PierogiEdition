// Browser requests keep their same origin. Native requests capture a PC or embedded target and its credential.
import { kv } from './kv.js';
import { privateGet, privateSet, initializePrivacy, privacyNeedsPairing } from './private-storage.js';
import { DEFAULT_PRESETS } from './default-presets.js';
import { currentServer, configuredConnection, configureConnection, setConfiguredServer, hasConfiguredServer,
  acquireTarget, selectedKind, hasEmbeddedServer, invalidateConnection, connectionFailed } from './server-connection.js';

const LS_TOKEN = 'mc.token';

export const isNative = () => !!(typeof window !== 'undefined' && window.Capacitor?.isNativePlatform?.());

export const config = {
  get server() {
    return currentServer();
  },
  set server(v) {
    setConfiguredServer(v);
  },
  get connection() { return configuredConnection(); },
  configureConnection,
  get token() {
    return privateGet(LS_TOKEN) || '';
  },
  set token(v) {
    privateSet(LS_TOKEN, v || null).catch(()=>{});
    invalidateConnection();
  },
  get kind() { return selectedKind(); },
  get embeddedReady() { return hasEmbeddedServer(); },
  get needsServer() {
    return isNative() && !hasConfiguredServer();
  },
};

export function uuid() {
  if (crypto?.randomUUID) return crypto.randomUUID();
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
  });
}

const headers = (target, json = true) => {
  const h = {};
  if (json) h['Content-Type'] = 'application/json';
  if (target.token) h.Authorization = `Bearer ${target.token}`;
  return h;
};

export class ApiError extends Error {
  constructor(message, status, data) {
    super(message);
    this.status = status;
    this.data = data || {};
    this.code = data?.code;
  }
}

function handleAuth(res, data, target) {
  if (res.status === 401 && data?.code === 'UNPAIRED') window.dispatchEvent(new CustomEvent('mc:unpaired', { detail: { kind: target.kind, server: target.server } }));
}

export const originGuard = target => ({ originServer: target.server, originKind: target.kind });
const originTarget = pending => pending?.originKind ? { server: pending.originServer, kind: pending.originKind } : undefined;

export function createApiTransport({ acquire = acquireTarget, fetch: send = (...args) => fetch(...args), failed = connectionFailed, auth = handleAuth } = {}) {
  const networkError = target => new ApiError(target?.kind === 'local' ? 'Sem conexão com o host Android.' : 'Sem conexão com o PC.', 0, { offline: true });
  async function request(method, path, body, options = {}) {
    const timeout = AbortSignal.timeout(15000);
    const signal = options.signal ? AbortSignal.any([options.signal, timeout]) : timeout;
    for (let attempt = 0; ; attempt++) {
      let target, dispatched = false;
      try {
        target = await acquire({ ...options, signal, write: method !== 'GET' });
        dispatched = true;
        const res = await send(`${target.server}${path}`, { method, headers: headers(target, body !== undefined),
          body: body === undefined ? undefined : JSON.stringify(body), signal, redirect: 'error',
          credentials: target.server ? 'omit' : 'same-origin' });
        const data = await res.json();
        if (!res.ok) { auth(res, data, target); throw new ApiError(data.error || `HTTP ${res.status}`, res.status, data); }
        options.onTarget?.(target, data);
        return data;
      } catch (error) {
        if (error instanceof ApiError || error.code) throw error;
        if (signal.aborted) throw signal.reason || error;
        failed(target);
        // Only reads may move to a newly synchronized endpoint. An ambiguous write is never replayed.
        if (method === 'GET' && !options.origin && options.pc === undefined && !options.embedded && dispatched && attempt === 0) continue;
        throw networkError(target);
      } finally { target?.release(); }
    }
  }
  async function stream(method, path, body, onEvent, signal, options = {}) {
    let target;
    const handshake = new AbortController();
    const timer = setTimeout(() => handshake.abort(), 15000);
    try {
      const combined = signal ? AbortSignal.any([signal, handshake.signal]) : handshake.signal;
      target = await acquire({ ...options, signal: combined, write: method !== 'GET', generation: !!options.generation });
      await options.beforeSend?.(target);
      combined.throwIfAborted();
      const res = await send(`${target.server}${path}`, { method, headers: headers(target, body !== undefined),
        body: body === undefined ? undefined : JSON.stringify(body), signal: combined, redirect: 'error',
        credentials: target.server ? 'omit' : 'same-origin' });
      clearTimeout(timer);
      if (!res.ok || !res.body) {
        const data = await res.json().catch(() => ({}));
        auth(res, data, target);
        throw new ApiError(data.error || `HTTP ${res.status}`, res.status, data);
      }
      await readSSE(res, event => onEvent(event, target), !!options.generation);
    } catch (error) {
      if (!signal?.aborted) failed(target);
      if (signal?.aborted || error instanceof ApiError || error.code) throw error;
      throw networkError(target);
    } finally {
      clearTimeout(timer);
      target?.release(); // Includes the response body and the complete SSE lifetime.
    }
  }
  return { request, stream };
}
const transport = createApiTransport();
const request = transport.request;
const generationWrites = new Set();

// Reads an SSE stream (through fetch, so the Authorization header can be sent).
export async function readSSE(res, onEvent, requireDone = false) {
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let terminal = false;
  try {
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let idx;
    while ((idx = buffer.search(/\r?\n\r?\n/)) >= 0) {
      const raw = buffer.slice(0, idx);
      buffer = buffer.slice(idx + (buffer[idx] === '\r' ? 4 : 2));
      const data = raw.split(/\r?\n/).filter(l => l.startsWith('data:')).map(l => l.slice(5).trim()).join('\n');
      if (!data) continue;
      let event;
      try {
        event = JSON.parse(data);
      } catch {
        continue;
      }
      terminal ||= !!event.done;
      onEvent(event);
    }
  }
  if (requireDone && !terminal) throw new ApiError('Fluxo interrompido; aguarde a recuperação no servidor de origem.', 0, { interrupted: true });
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

// Cache writes stay off the critical path: the latest version of each key waits for the browser to be idle
// (or the page to be hidden) and only then is encrypted and written. The PC remains the source of truth.
const pendingCache = new Map();
let cacheTimer = 0;
let cacheWrite = Promise.resolve();
function flushCache() {
  cacheTimer = 0;
  const entries = [...pendingCache];
  pendingCache.clear();
  if (!entries.length) return cacheWrite;
  cacheWrite = cacheWrite.then(async () => {
    for (const [key, value] of entries) await kv.set(key, value);
  }).catch(() => {});
  return cacheWrite;
}
function queueCache(key, value) {
  pendingCache.set(key, value);
  if (cacheTimer) return;
  cacheTimer = typeof requestIdleCallback === 'function' ? requestIdleCallback(flushCache, { timeout: 2500 }) : setTimeout(flushCache, 400);
}
async function cacheGet(key, fallback) {
  return pendingCache.has(key) ? pendingCache.get(key) : kv.get(key, fallback);
}
if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flushCache(); });
  window.addEventListener('pagehide', flushCache);
}

const GLOBAL_SETTINGS_TTL = 60000;
let globalSettingsCache = null; // { at, value }
const CACHE_LIST = 'chats';
const CACHE_CHAT = (id) => `chat.${id}`;
const PUBLIC_PRESETS = new Set(['deepseek-v41-flash', 'claude-opus-5-5', 'claude-sonnet-5-5', 'gemini-38-flash', 'kimi-k3', 'muse-spark-13', 'step-5-preview', 'glm-52', 'mobile-local', 'mobile-api']);
export const publicPresets = data => data ? {
  ...data,
  presets: (data.presets || [])
    .filter(p => PUBLIC_PRESETS.has(p.id))
    .map(p => p.id === 'glm-52' ? { ...p, label: 'GLM' } : p),
} : null;

export const api = {
  // ----- cache local (IndexedDB) -----
  cachedChats: () => cacheGet(CACHE_LIST, []),
  cachedChat: (id) => cacheGet(CACHE_CHAT(id), null),
  saveChatCache: (chat) => queueCache(CACHE_CHAT(chat.id), chat),
  flushCache,
  cachedPresets: async () => {
    const raw = await kv.get('presets', null);
    const cached = publicPresets(raw);
    if (cached?.presets?.some(p => p.id === 'mobile-local' || p.id === 'mobile-api')) return cached;
    const gemini = cached?.presets?.find((p) => p.id === 'gemini-38-flash');
    if (!cached || !gemini || !gemini.efforts || gemini.efforts.length < 3 || !gemini.routes || gemini.routes.length < 3) {
      await kv.set('presets', DEFAULT_PRESETS);
      return DEFAULT_PRESETS;
    }
    return cached;
  },

  // ----- per-conversation drafts -----
  getDraft: (id) => privateGet(`mc.draft.${id}`) || '',
  setDraft: (id, text) => privateSet(`mc.draft.${id}`, text || null).catch(()=>{}),
  getPending: (id) => { try { return JSON.parse(privateGet(`mc.pending.${id}`) || 'null'); } catch { return null; } },
  setPending: (id, operation) => privateSet(`mc.pending.${id}`, operation ? JSON.stringify(operation) : null),

  // ----- pareamento / status -----
  health: (options) => request('GET', '/api/health', undefined, options),
  pairInfo: (pc) => request('GET', '/api/pair/info', undefined, { pc: pc ?? (config.connection.local || config.connection.remote || '') }),
  async pair(code, deviceName, pc = config.connection.local || config.connection.remote || '') {
    const r = await request('POST', '/api/pair', { code, deviceName }, { pc });
    await privateSet(LS_TOKEN,r.token);
    if(privacyNeedsPairing()) {await initializePrivacy(pc);await kv.migratePrivacy();await privateSet(LS_TOKEN,r.token);}
    invalidateConnection();
    return r;
  },
  startPairing: () => request('POST', '/api/pair/start', undefined, { pc: config.connection.local || config.connection.remote || '' }),
  status: () => request('GET', '/api/status'),
  devices: () => request('GET', '/api/devices'),
  revokeDevice: (id) => request('DELETE', `/api/devices/${id}`),
  localStatus: () => request('GET', '/api/local/status', undefined, { embedded: true }),
  localProviders: () => request('GET', '/api/local/providers', undefined, { embedded: true }),
  saveLocalProviders: (body) => request('PUT', '/api/local/providers', body, { embedded: true }),

  // ----- conversas -----
  async listChats(archived = false, q = '') {
    const { chats } = await request('GET', `/api/chats?archived=${archived ? 1 : 0}&q=${encodeURIComponent(q)}`);
    if (!archived && !q) queueCache(CACHE_LIST, chats);
    return chats;
  },
  async getChat(id, options) {
    const chat = await request('GET', `/api/chats/${id}`, undefined, { ...options, onTarget: (target, data) => {
      if (data.generation) Object.assign(data.generation, originGuard(target));
    } });
    chat.history = chat.history.filter(m => m.status !== 'generating');
    queueCache(CACHE_CHAT(id), chat);
    return chat;
  },
  async createChat(data = {}) {
    const chat = await request('POST', '/api/chats', data);
    queueCache(CACHE_CHAT(chat.id), chat);
    return chat;
  },
  async updateChat(id, patch) {
    const updated = await request('PUT', `/api/chats/${id}`, patch);
    const prev = (await cacheGet(CACHE_CHAT(id), {})) || {};
    queueCache(CACHE_CHAT(id), { ...prev, ...updated });
    return updated;
  },
  duplicateChat: (id) => request('POST', `/api/chats/${id}/duplicate`),
  deleteChat: async (id) => {
    const r = await request('DELETE', `/api/chats/${id}`);
    pendingCache.delete(CACHE_CHAT(id));
    await cacheWrite;
    await kv.del(CACHE_CHAT(id));
    return r;
  },
  clearHistory: (id) => request('DELETE', `/api/chats/${id}/history`),
  clearAllHistory: () => request('DELETE', '/api/history'),
  trash: () => request('GET', '/api/trash'),
  restoreTrash: (item) => request('POST', '/api/trash/restore', item),
  globalSettings: () => request('GET', '/api/settings/global'),
  // Summary shown by the side menu: the menu is opened over and over, and each open used to hit the PC (TLS, JSON,
  // radio) just to print "75% falas · 2.000 car.". Kept for a minute and dropped by any save of the settings.
  peekGlobalSettings: () => (globalSettingsCache && Date.now() - globalSettingsCache.at < GLOBAL_SETTINGS_TTL ? globalSettingsCache.value : null),
  globalSettingsCached: async () => {
    const hit = api.peekGlobalSettings();
    if (hit) return { settings: hit };
    const r = await request('GET', '/api/settings/global');
    globalSettingsCache = { at: Date.now(), value: r.settings };
    return r;
  },
  saveGlobalSettings: async (patch) => {
    globalSettingsCache = null;
    const r = await request('PUT', '/api/settings/global', patch);
    globalSettingsCache = null;
    return r;
  },
  purgeTrash: (item) => request('POST', '/api/trash/purge', item),
  purgeAllTrash: () => request('POST', '/api/trash/purge', { all: true }),

  // ----- mensagens -----
  editMessage: (chatId, messageId, content, opId = uuid()) => request('PUT', `/api/chats/${chatId}/messages/${messageId}`, { content, opId }),
  deleteMessage: (chatId, messageId) => request('DELETE', `/api/chats/${chatId}/messages/${messageId}`),
  versions: (chatId, messageId) => request('GET', `/api/chats/${chatId}/messages/${messageId}/versions`),
  selectVersion: (chatId, messageId, versionId) => request('POST', `/api/chats/${chatId}/messages/${messageId}/versions/${versionId}/select`),
  activateBranch: (chatId, interactionId) => request('POST', `/api/chats/${chatId}/branches/activate`, { interactionId }),
  resolveFlag: (chatId, flagId, action) => request('POST', `/api/chats/${chatId}/flags/${flagId}/resolve`, { action }),
  stopGeneration: (chatId, genId) => {
    const pending = api.getPending(chatId);
    return request('POST', `/api/chats/${chatId}/generations/${genId}/stop`, undefined, { origin: originTarget(pending) });
  },
  async queryOperation(chatId, pending = api.getPending(chatId)) {
    if (!originTarget(pending)) throw new ApiError('O envio pendente não registra a origem; preserve-o para revisão.', 409, { code: 'UNKNOWN_ORIGIN' });
    const chat = await request('GET', `/api/chats/${chatId}`, undefined, { origin: originTarget(pending) });
    if (pending.generationId) return { id: pending.generationId, mode: pending.mode, partId: pending.partId };
    // The current HTTP contract has no GenerationByOp endpoint. A running stream exposes its opId in ack.
    if (chat.generation) return chat.generation;
    throw new ApiError('O recebimento do envio ficou incerto. Os dados foram preservados; consulte o servidor de origem antes de enviar novamente.', 409, { code: 'AMBIGUOUS_WRITE' });
  },

  // ----- cast and memory -----
  cast: (id) => request('GET', `/api/chats/${id}/cast`),
  addCast: (id, c) => request('POST', `/api/chats/${id}/cast`, c),
  updateCast: (id, cid, patch) => request('PUT', `/api/chats/${id}/cast/${cid}`, patch),
  deleteCast: (id, cid) => request('DELETE', `/api/chats/${id}/cast/${cid}`),
  mergeCast: (id, cid, intoId) => request('POST', `/api/chats/${id}/cast/${cid}/merge`, { intoId }),
  reanalyzeCast: (id) => request('POST', `/api/chats/${id}/cast/reanalyze`),

  // ----- persona profiles -----
  personas: () => request('GET', '/api/personas'),
  createPersona: (p) => request('POST', '/api/personas', p),
  updatePersona: (id, p) => request('PUT', `/api/personas/${id}`, p),
  deletePersona: (id) => request('DELETE', `/api/personas/${id}`),
  memory: (id) => request('GET', `/api/chats/${id}/memory`),
  addMemory: (id, m) => request('POST', `/api/chats/${id}/memory`, m),
  updateMemory: (id, mid, patch) => request('PUT', `/api/chats/${id}/memory/${mid}`, patch),
  deleteMemory: (id, mid) => request('DELETE', `/api/chats/${id}/memory/${mid}`),
  retryJobs: (id) => request('POST', `/api/chats/${id}/jobs/retry`),
  source: (id, iid) => request('GET', `/api/chats/${id}/interactions/${iid}/source`),

  // ----- modelos / presets -----
  async presets() {
    const data = publicPresets(await request('GET', '/api/presets'));
    await kv.set('presets', data);
    return data;
  },

  // ----- busca / backup -----
  search: (q, chat) => request('GET', `/api/search?q=${encodeURIComponent(q)}${chat ? `&chat=${chat}` : ''}`),
  backup: () => request('POST', '/api/backup'),
  async exportAll(chatId) {
    return request('GET', chatId ? `/api/chats/${chatId}/export` : '/api/export');
  },
  importData: (data) => request('POST', '/api/import', data),

  // ----- streaming send -----
  // onEvent receives: { ack } | { delta } | { done, status, message, error, usage } | { error }
  async sendMessage(id, payload, onEvent, signal) {
    const existing = api.getPending(id);
    if (existing || generationWrites.has(id)) throw new ApiError('Este chat aguarda confirmação de um envio no servidor de origem.', 409, { code: 'AMBIGUOUS_WRITE' });
    generationWrites.add(id);
    let persisted = Promise.resolve();
    const outgoing = { ...payload, opId: payload.opId || uuid() };
    for (const key of ['originServer', 'originKind', 'generationId', 'partId']) delete outgoing[key];
    try {
      await transport.stream('POST', `/api/chats/${id}/message`, outgoing, (event, target) => {
        if (event.ack) persisted = api.setPending(id, { ...api.getPending(id), ...originGuard(target), generationId: event.ack.generationId,
          mode: event.ack.mode, partId: event.ack.partId }).catch(error => { throw new ApiError('Não foi possível proteger a confirmação do envio.', 0, { code: 'PRIVATE_CACHE_FAILED', cause: error.message }); });
        persisted.catch(() => {});
        onEvent(event);
      }, signal, { generation: true, beforeSend: target => api.setPending(id, { ...outgoing, ...originGuard(target) }) });
    } finally { try { await persisted; } finally { generationWrites.delete(id); } }
  },
  async attachGeneration(chatId, genId, from, onEvent, signal) {
    const pending = api.getPending(chatId);
    await transport.stream('GET', `/api/chats/${chatId}/generations/${genId}/stream?from=${from}`, undefined, (event, target) => {
      if (event.ack && pending?.opId && pending.opId !== event.ack.opId) throw new ApiError('A geração encontrada pertence a outro envio; a pendência foi preservada.', 409, { code: 'OPERATION_MISMATCH' });
      if (event.ack) api.setPending(chatId, { ...api.getPending(chatId), ...originGuard(target), opId: event.ack.opId,
        generationId: event.ack.generationId, mode: event.ack.mode, partId: event.ack.partId }).catch(() => {});
      onEvent(event);
    }, signal, { generation: true, origin: originTarget(pending), beforeSend: target => api.setPending(chatId, {
      ...pending, ...originGuard(target), generationId: genId,
    }) });
  },

  // ----- cross-device sync events -----
  async subscribeEvents(onEvent, signal) {
    for (;;) {
      if (signal?.aborted) return;
      try {
        await transport.stream('GET', '/api/events', undefined, onEvent, signal);
      } catch (e) {
        if (e.name === 'AbortError') return;
      }
      await new Promise((r) => {
        const timer = setTimeout(done, 3000);
        function done() { clearTimeout(timer); signal?.removeEventListener('abort', done); r(); }
        signal?.addEventListener('abort', done, { once: true });
        if (signal?.aborted) done();
      });
    }
  },
};
