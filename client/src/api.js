// Talks to the PC server + local device cache.
// In a browser served by the PC itself the base URL is empty (same origin). In the APK (Capacitor) the base is the
// PC's local address entered during setup, and every call carries the pairing token.
import { kv } from './kv.js';
import { privateGet, privateSet, initializePrivacy, privacyNeedsPairing } from './private-storage.js';
import { DEFAULT_PRESETS } from './default-presets.js';
import { currentServer, configuredConnection, configureConnection, setConfiguredServer, hasConfiguredServer, resolveServer, connectionFailed } from './server-connection.js';

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
  },
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

const url = async (p, signal) => `${await resolveServer(signal)}${p}`;
const headers = (json = true) => {
  const h = {};
  if (json) h['Content-Type'] = 'application/json';
  if (config.token) h.Authorization = `Bearer ${config.token}`;
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

function handleAuth(res, data) {
  if (res.status === 401 && data?.code === 'UNPAIRED') window.dispatchEvent(new CustomEvent('mc:unpaired'));
}

async function request(method, path, body, { signal } = {}) {
  signal = signal ? AbortSignal.any([signal, AbortSignal.timeout(15000)]) : AbortSignal.timeout(15000);
  let res;
  try {
    res = await fetch(await url(path, signal), { method, headers: headers(!!body), body: body ? JSON.stringify(body) : undefined, signal, redirect: 'error' });
  } catch (e) {
    if (e.name === 'AbortError') throw e;
    connectionFailed();
    throw new ApiError('Sem conexão com o PC.', 0, { offline: true });
  }
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    handleAuth(res, data);
    throw new ApiError(data.error || `HTTP ${res.status}`, res.status, data);
  }
  return data;
}

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
  if (requireDone && !terminal) throw new ApiError('Fluxo interrompido; reconectando à resposta salva no PC.', 0);
  } finally {
    reader.releaseLock();
  }
}

async function openStream(method, path, body, signal) {
  let res;
  const handshake = new AbortController();
  const timer = setTimeout(() => handshake.abort(), 15000);
  try {
    const combined = signal ? AbortSignal.any([signal, handshake.signal]) : handshake.signal;
    res = await fetch(await url(path, combined), { method, headers: headers(!!body), body: body ? JSON.stringify(body) : undefined, signal: combined, redirect: 'error' });
  } catch (e) {
    if (signal?.aborted) throw e;
    connectionFailed();
    throw new ApiError('Sem conexão com o PC.', 0, { offline: true });
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok || !res.body) {
    const data = await res.json().catch(() => ({}));
    handleAuth(res, data);
    throw new ApiError(data.error || `HTTP ${res.status}`, res.status, data);
  }
  return res;
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
const PUBLIC_PRESETS = new Set(['deepseek-v41-flash', 'claude-opus-5-5', 'claude-sonnet-5-5', 'gemini-38-flash', 'kimi-k3', 'muse-spark-13', 'step-5-preview', 'glm-52']);
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
  health: () => request('GET', '/api/health'),
  pairInfo: () => request('GET', '/api/pair/info'),
  async pair(code, deviceName) {
    const r = await request('POST', '/api/pair', { code, deviceName });
    await privateSet(LS_TOKEN,r.token);
    if(privacyNeedsPairing()) {await initializePrivacy(config.server);await kv.migratePrivacy();await privateSet(LS_TOKEN,r.token);}
    return r;
  },
  startPairing: () => request('POST', '/api/pair/start'),
  status: () => request('GET', '/api/status'),
  devices: () => request('GET', '/api/devices'),
  revokeDevice: (id) => request('DELETE', `/api/devices/${id}`),

  // ----- conversas -----
  async listChats(archived = false, q = '') {
    const { chats } = await request('GET', `/api/chats?archived=${archived ? 1 : 0}&q=${encodeURIComponent(q)}`);
    if (!archived && !q) queueCache(CACHE_LIST, chats);
    return chats;
  },
  async getChat(id) {
    const chat = await request('GET', `/api/chats/${id}`);
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
  stopGeneration: (chatId, genId) => request('POST', `/api/chats/${chatId}/generations/${genId}/stop`),

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
    const res = await fetch(await url(chatId ? `/api/chats/${chatId}/export` : '/api/export'), { headers: headers(false), redirect: 'error' });
    if (!res.ok) throw new ApiError(`HTTP ${res.status}`, res.status);
    return res.json();
  },
  importData: (data) => request('POST', '/api/import', data),

  // ----- streaming send -----
  // onEvent receives: { ack } | { delta } | { done, status, message, error, usage } | { error }
  async sendMessage(id, payload, onEvent, signal) {
    const res = await openStream('POST', `/api/chats/${id}/message`, payload, signal);
    try { await readSSE(res, onEvent, true); }
    catch (error) { if (!signal?.aborted) connectionFailed(); throw error; }
  },
  async attachGeneration(chatId, genId, from, onEvent, signal) {
    const res = await openStream('GET', `/api/chats/${chatId}/generations/${genId}/stream?from=${from}`, null, signal);
    try { await readSSE(res, onEvent, true); }
    catch (error) { if (!signal?.aborted) connectionFailed(); throw error; }
  },

  // ----- cross-device sync events -----
  async subscribeEvents(onEvent, signal) {
    for (;;) {
      if (signal?.aborted) return;
      try {
        const res = await openStream('GET', '/api/events', null, signal);
        await readSSE(res, onEvent);
      } catch (e) {
        if (e.name === 'AbortError') return;
        connectionFailed();
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
