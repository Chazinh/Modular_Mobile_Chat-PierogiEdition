// Select an endpoint before opening a request. Health probes never replay writes.
import { privateGet } from './private-storage.js';
import { localHost } from './local-host.js';
import { mobileSync } from './mobile-sync.js';

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
export function createServerConnection({ storage, native, token, fetch: probe, now = Date.now, onChange = () => {},
  onStatus = () => {}, embedded = null, sync = null, mode = 'legacy', timers = globalThis }) {
  let active = null, validUntil = 0, syncedUntil = 0, syncRetryAt = 0, epoch = 0, pending = null, boundary = null, stopWatching = null;
  const leases = new Set();
  const capturedEpochs = new WeakMap();
  const local = () => mode === 'auto' && native() ? embedded?.target() : null;
  const error = (code, message) => Object.assign(Error(message), { code });
  const captured = target => {
    if (!target) throw error('ORIGIN_UNAVAILABLE', 'O servidor de origem não está disponível.');
    const result = Object.freeze({ server: target.server, kind: target.kind,
      token: target.kind === 'local' ? local()?.token || '' : token() || '' });
    capturedEpochs.set(result, epoch);
    return result;
  };
  const read = key => { try { return storage.getItem(key) || ''; } catch { return ''; } };
  function configured() {
    try {
      const saved = JSON.parse(read('mc.connection'));
      if (saved && typeof saved.local === 'string' && typeof saved.remote === 'string') return { local: saved.local, remote: saved.remote };
    } catch { /* Older clients store just one address. */ }
    return { local: read('mc.server').replace(/\/+$/, ''), remote: '' };
  }
  function invalidate() {
    validUntil = 0; syncedUntil = 0; syncRetryAt = 0; epoch++;
    pending?.controller.abort(error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.'));
  }
  function configure({ local: newLocal = '', remote: newRemote = '' } = {}) {
    const next = { local: normalizeServer(newLocal, native()), remote: normalizeServer(newRemote) };
    const previous = { record: read('mc.connection'), legacy: read('mc.server'), active };
    // Keep both endpoints together: changing computers cannot inherit another computer's domain.
    storage.setItem('mc.connection', JSON.stringify(next));
    try { storage.removeItem('mc.server'); } catch { /* The new record takes precedence. */ }
    if (!leases.size) active = local();
    invalidate();
    return () => {
      if (previous.record) storage.setItem('mc.connection', previous.record); else storage.removeItem('mc.connection');
      if (previous.legacy) storage.setItem('mc.server', previous.legacy); else storage.removeItem('mc.server');
      active = previous.active;
      invalidate();
    };
  }
  const current = () => !native() && mode === 'auto' ? '' : active?.server || local()?.server || configured().local || configured().remote;
  const kind = () => active?.kind || (local() ? 'local' : native() ? 'pc' : 'browser');
  const hasLocal = () => !!local();
  function hasServer() {
    if (local()) return true;
    const saved = configured();
    try { return !!(normalizeServer(saved.local, native()) || normalizeServer(saved.remote)); } catch { return false; }
  }
  async function available(server, key, signal) {
    const timeout = new AbortController();
    const timer = timers.setTimeout(() => timeout.abort(), 400);
    try {
      const response = await probe(`${server}/api/health`, {
        cache: 'no-store', credentials: 'omit', redirect: 'error',
        headers: key ? { Authorization: `Bearer ${key}` } : {},
        signal: AbortSignal.any([signal, timeout.signal]),
      });
      const data = await response.json();
      // Revocation is an authentication event, not a reason to switch computers.
      if (mode === 'legacy' && response.status === 401 && data.code === 'UNPAIRED') return true;
      return response.ok && data.ok === true && data.name === 'modularchat' && data.encrypted === true;
    } catch (failure) { if (signal.aborted) throw signal.reason || failure; return false; }
    finally { timers.clearTimeout(timer); }
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
  async function resolveTarget(signal, { fresh = false } = {}) {
    signal?.throwIfAborted();
    if (!native()) return captured({ server: current() ? normalizeServer(current(), mode === 'auto') : '', kind: 'browser' });
    if (pending?.epoch === epoch) return waitFor(pending.promise, signal);
    if (!fresh && active && now() < validUntil) return captured(active);
    const saved = configured();
    const host = local();
    let lan = '', remote = '';
    try { lan = normalizeServer(saved.local); remote = normalizeServer(saved.remote); }
    catch (failure) { if (!host) throw failure; onStatus({ code: 'INVALID_PC_CONFIG', message: failure.message, kind: 'local' }); }
    if (!lan && !remote && !host) throw Error('Informe o endereço HTTPS do PC e faça o pareamento.');
    const startedAt = epoch;
    const controller = new AbortController();
    const pcToken = token() || '';
    const job = (async () => {
      let selected = null, syncResult;
      if (lan && await available(lan, pcToken, controller.signal)) selected = { server: lan, kind: 'pc' };
      else if (remote && remote !== lan && await available(remote, pcToken, controller.signal)) selected = { server: remote, kind: 'pc' };
      if (startedAt !== epoch) throw error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.');
      if (leases.size) {
        // Accepted writes/generations hold their original endpoint until body/SSE consumption ends.
        if (active?.kind === 'pc' && selected?.server !== active.server) throw error('ORIGIN_BUSY', 'A operação ainda pertence ao servidor de origem. Aguarde sua conclusão ou recuperação.');
        return captured(active || host || selected);
      }
      if (host && selected && !pcToken) {
        onStatus({ code: 'PC_UNPAIRED', message: 'O PC precisa de pareamento; o host Android continua disponível.', kind: 'local' });
        selected = null;
      }
      if (host && selected && now() < syncRetryAt) selected = null;
      if (host && selected && (active?.server !== selected.server || active?.kind !== 'pc' || now() >= syncedUntil)) {
        boundary = controller;
        try {
          if (!sync) throw error('SYNC_REQUIRED', 'A sincronização precisa ser concluída antes de usar o PC.');
          await embedded.standby(controller.signal);
          syncResult = await sync({ pc: { ...selected, token: pcToken }, mobile: host, signal: controller.signal,
            isCurrent: () => startedAt === epoch });
          if (syncResult?.converged !== true) throw error('INCOMPLETE_SYNC', 'Os cofres ainda não convergiram.');
          syncedUntil = now() + 10000;
          syncRetryAt = 0;
        } catch (failure) {
          if (startedAt !== epoch || controller.signal.aborted) throw failure;
          onStatus({ code: failure.code || 'SYNC_FAILED', message: failure.message, kind: 'local', data: failure.data });
          syncRetryAt = now() + 10000;
          selected = null;
        } finally { if (boundary === controller) boundary = null; }
      }
      if (!selected && host) {
        await embedded.activate(controller.signal); // Starts downloads through the API, never loads a model in JS.
        selected = host;
      }
      if (!selected) throw Error('Não consegui conectar ao PC por HTTPS. Confira os endereços e o certificado.');
      if (startedAt !== epoch || controller.signal.aborted) throw error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.');
      const changed = active?.server !== selected.server || active?.kind !== selected.kind;
      active = selected;
      validUntil = now() + (host ? 10000 : 30000);
      if (changed) onChange(selected.server, { kind: selected.kind, sync: syncResult });
      if (syncResult) onStatus({ code: 'SYNCED', kind: selected.kind, skippedVectors: syncResult.skippedVectors,
        compatibleEmbeddings: syncResult.compatibleEmbeddings });
      return captured(selected);
    })();
    pending = { epoch: startedAt, promise: job, controller };
    // Clear shared state even if an individual caller aborts its own wait.
    job.then(() => { if (pending?.promise === job) pending = null; }, () => { if (pending?.promise === job) pending = null; });
    return waitFor(job, signal);
  }

  async function acquireTarget({ signal, write = false, generation = false, origin, pc, embedded: wantLocal = false } = {}) {
    let target;
    if (origin || pc !== undefined || wantLocal) {
      // Recovery and pairing never go through automatic selection or cross to another database.
      if (boundary || pending) await waitFor(pending?.promise || Promise.resolve(), signal);
      const fixed = origin || (wantLocal ? local() : { server: pc || configured().local || configured().remote, kind: native() ? 'pc' : 'browser' });
      if (!fixed) throw error('LOCAL_HOST_NOT_READY', 'O host Android não está pronto.');
      if (fixed.kind === 'local') {
        if (fixed.server !== local()?.server) throw error('ORIGIN_UNAVAILABLE', 'O host Android de origem não está disponível.');
        target = captured(fixed);
      } else {
        const server = fixed.server ? normalizeServer(fixed.server, native() || mode === 'auto') : '';
        const saved = configured();
        if (native() && (!server || ![saved.local, saved.remote].includes(server))) throw error('ORIGIN_CHANGED', 'Reconfigure o PC de origem antes de recuperar esta operação.');
        target = captured({ server, kind: fixed.kind });
      }
    } else {
      target = await resolveTarget(signal, { fresh: write });
      if (capturedEpochs.get(target) !== epoch) throw error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.');
    }
    signal?.throwIfAborted();
    const id = {};
    if (write || generation) leases.add(id);
    if (write && (origin || pc !== undefined) && target.kind === 'pc') {
      try {
        const control = new AbortController();
        if (!await available(target.server, target.token, signal || control.signal)) throw error('ORIGIN_UNAVAILABLE', 'Sem conexão com o PC de origem. O envio não foi repetido.');
        if (capturedEpochs.get(target) !== epoch) throw error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.');
      } catch (failure) { leases.delete(id); throw failure; }
    }
    let released = false;
    return Object.freeze({ ...target, release() {
      if (released) return;
      released = true;
      leases.delete(id);
    } });
  }
  const resolve = async signal => (await resolveTarget(signal)).server;
  function start({ window: win, document: doc, navigator: nav } = {}) {
    stopWatching?.();
    if (!native()) return () => {};
    let timer = null, stopped = false, watchEpoch = 0;
    const foreground = () => !doc || doc.visibilityState !== 'hidden';
    const tick = async () => {
      const cycle = watchEpoch;
      timer = null;
      if (stopped || !foreground()) return;
      try { if (!leases.size) await resolveTarget(undefined, { fresh: true }); }
      catch (failure) { if (!stopped && failure.code !== 'STALE_CONNECTION') onStatus({ code: failure.code || 'CONNECTION_FAILED', message: failure.message, kind: kind() }); }
      finally { if (!stopped && foreground() && cycle === watchEpoch) timer = timers.setTimeout(tick, 10000); }
    };
    const changed = () => {
      watchEpoch++;
      embedded?.invalidateLifecycle?.();
      invalidate();
      if (doc && doc.visibilityState === 'hidden') {
        timers.clearTimeout(timer);
        timer = null;
        return;
      }
      timers.clearTimeout(timer);
      timer = null;
      if (foreground() && !stopped) timer = timers.setTimeout(tick, 0);
    };
    for (const event of ['online', 'offline', 'mc:resume']) win?.addEventListener(event, changed);
    doc?.addEventListener('visibilitychange', changed);
    nav?.connection?.addEventListener('change', changed);
    timer = timers.setTimeout(tick, 0);
    const stop = () => {
      if (stopped) return;
      stopped = true;
      timers.clearTimeout(timer);
      invalidate();
      for (const event of ['online', 'offline', 'mc:resume']) win?.removeEventListener(event, changed);
      doc?.removeEventListener('visibilitychange', changed);
      nav?.connection?.removeEventListener('change', changed);
      if (stopWatching === stop) stopWatching = null;
    };
    stopWatching = stop;
    return stop;
  }
  return { configured, configure, current, kind, hasLocal, hasServer, resolve, resolveTarget, acquireTarget, invalidate, start };
}

const native = () => !!(typeof window !== 'undefined' && window.Capacitor?.isNativePlatform?.());
const connection = createServerConnection({
  storage: {
    getItem: key => localStorage.getItem(key),
    setItem: (key, value) => localStorage.setItem(key, value),
    removeItem: key => localStorage.removeItem(key),
  },
  native, mode: 'auto', embedded: localHost, sync: options => mobileSync.synchronize(options),
  token: () => privateGet('mc.token'), fetch: (...args) => fetch(...args),
  onChange: (server, detail) => window.dispatchEvent(new CustomEvent('mc:connection-change', { detail: { server, ...detail } })),
  onStatus: detail => window.dispatchEvent(new CustomEvent('mc:connection-health', { detail })),
});
export const configuredConnection = () => connection.configured();
export const configureConnection = value => connection.configure(value);
export const currentServer = () => connection.current();
export const hasConfiguredServer = () => connection.hasServer();
export const setConfiguredServer = value => connection.configure({ local: value || '', remote: '' });
export const resolveServer = signal => connection.resolve(signal);
export const resolveTarget = (signal, options) => connection.resolveTarget(signal, options);
export const acquireTarget = options => connection.acquireTarget(options);
export const selectedKind = () => connection.kind();
export const hasEmbeddedServer = () => connection.hasLocal();
export const startConnection = () => connection.start({ window, document, navigator });
export const invalidateConnection = () => connection.invalidate();
export const connectionFailed = target => { if (native() && (!target || target.server === connection.current())) connection.invalidate(); };
