// Select an endpoint before opening a request. Health probes never replay writes.
import { privateGet } from './private-storage.js';

export function normalizeServer(value, httpsOnly = true) {
  if (!String(value || '').trim()) return '';
  let url;
  try { url = new URL(String(value).trim()); } catch { throw Error('Informe o endereço completo do PC.'); }
  if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
    throw Error('Informe somente o endereço do PC, sem senha, caminho ou parâmetros.');
  }
  if (url.protocol !== 'https:' && (httpsOnly || url.protocol !== 'http:')) {
    throw Error('A conexão com o PC precisa usar HTTPS.');
  }
  return url.origin;
}

// Dependencies are explicit so network changes and concurrent probes can be verified in isolation.
export function createServerConnection({ storage, native, token, fetch: probe, now = Date.now, onChange = () => {} }) {
  let active = '', validUntil = 0, epoch = 0, pending = null;
  const read = key => { try { return storage.getItem(key) || ''; } catch { return ''; } };
  function configured() {
    try {
      const saved = JSON.parse(read('mc.connection'));
      if (saved && typeof saved.local === 'string' && typeof saved.remote === 'string') return { local: saved.local, remote: saved.remote };
    } catch { /* Older clients store just one address. */ }
    return { local: read('mc.server').replace(/\/+$/, ''), remote: '' };
  }
  function invalidate() { validUntil = 0; epoch++; }
  function configure({ local = '', remote = '' } = {}) {
    const next = { local: normalizeServer(local, native()), remote: normalizeServer(remote) };
    const previous = { record: read('mc.connection'), legacy: read('mc.server'), active };
    // Keep both endpoints together: changing computers cannot inherit another computer's domain.
    storage.setItem('mc.connection', JSON.stringify(next));
    try { storage.removeItem('mc.server'); } catch { /* The new record takes precedence. */ }
    active = '';
    invalidate();
    return () => {
      if (previous.record) storage.setItem('mc.connection', previous.record); else storage.removeItem('mc.connection');
      if (previous.legacy) storage.setItem('mc.server', previous.legacy); else storage.removeItem('mc.server');
      active = previous.active;
      invalidate();
    };
  }
  const current = () => active || configured().local || configured().remote;
  function hasServer() {
    const saved = configured();
    try { return !!(normalizeServer(saved.local, native()) || normalizeServer(saved.remote)); } catch { return false; }
  }
  async function available(server, timeout) {
    try {
      const key = token();
      const response = await probe(`${server}/api/health`, {
        cache: 'no-store', credentials: 'omit', redirect: 'error',
        headers: key ? { Authorization: `Bearer ${key}` } : {},
        signal: AbortSignal.timeout(timeout),
      });
      const data = await response.json();
      // Revocation is an authentication event, not a reason to switch computers.
      if (response.status === 401 && data.code === 'UNPAIRED') return true;
      return response.ok && data.ok === true && data.name === 'modularchat' && data.encrypted === true;
    } catch { return false; }
  }
  async function waitFor(promise, signal) {
    if (!signal) return promise;
    signal.throwIfAborted();
    let abort;
    const cancelled = new Promise((_, reject) => {
      abort = () => reject(signal.reason || new DOMException('Operação cancelada.', 'AbortError'));
      signal.addEventListener('abort', abort, { once: true });
    });
    try { return await Promise.race([promise, cancelled]); }
    finally { signal.removeEventListener('abort', abort); }
  }
  async function resolve(signal) {
    signal?.throwIfAborted();
    if (!native()) return current() ? normalizeServer(current(), false) : '';
    if (active && now() < validUntil) return active;
    if (pending?.epoch === epoch) return waitFor(pending.promise, signal);
    const saved = configured();
    const local = normalizeServer(saved.local), remote = normalizeServer(saved.remote);
    if (!local && !remote) throw Error('Informe o endereço HTTPS do PC e faça o pareamento.');
    const startedAt = epoch;
    const job = (async () => {
      let selected;
      if (local && await available(local, 1500)) selected = local;
      else if (remote && remote !== local && await available(remote, 5000)) selected = remote;
      else throw Error('Não consegui conectar ao PC por HTTPS. Confira os endereços e o certificado.');
      // A request must never be sent to an endpoint resolved for an obsolete configuration.
      if (startedAt !== epoch) throw Error('A conexão mudou. Tente novamente.');
      const changed = active !== selected;
      active = selected;
      validUntil = now() + 30000;
      if (changed) onChange(selected);
      return selected;
    })();
    pending = { epoch: startedAt, promise: job };
    // Clear shared state even if an individual caller aborts its own wait.
    job.then(() => { if (pending?.promise === job) pending = null; }, () => { if (pending?.promise === job) pending = null; });
    return waitFor(job, signal);
  }
  return { configured, configure, current, hasServer, resolve, invalidate };
}

const native = () => !!(typeof window !== 'undefined' && window.Capacitor?.isNativePlatform?.());
const connection = createServerConnection({
  storage: {
    getItem: key => localStorage.getItem(key),
    setItem: (key, value) => localStorage.setItem(key, value),
    removeItem: key => localStorage.removeItem(key),
  },
  native, token: () => privateGet('mc.token'), fetch: (...args) => fetch(...args),
  onChange: server => window.dispatchEvent(new CustomEvent('mc:connection-change', { detail: { server } })),
});
export const configuredConnection = () => connection.configured();
export const configureConnection = value => connection.configure(value);
export const currentServer = () => connection.current();
export const hasConfiguredServer = () => connection.hasServer();
export const setConfiguredServer = value => connection.configure({ local: value || '', remote: '' });
export const resolveServer = signal => connection.resolve(signal);
export const invalidateConnection = () => connection.invalidate();
export const connectionFailed = () => { if (native()) connection.invalidate(); };

if (typeof window !== 'undefined') {
  for (const event of ['online', 'offline', 'mc:resume']) window.addEventListener(event, invalidateConnection);
  if (typeof navigator !== 'undefined') navigator.connection?.addEventListener?.('change', invalidateConnection);
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') invalidateConnection();
  });
}
