// Select an endpoint before opening a request. Health probes never replay writes.
import { privateGet } from './private-storage.js';
import { localHost } from './local-host.js';
import { mobileSync } from './mobile-sync.js';
import { waitForOperation } from './connection-lifetime.js';

import { normalizeServer, normalizePcServer } from './server-address.js';
export { normalizeServer, normalizePcServer, isLocalAddress } from './server-address.js';

// A candidate health check is public: it never changes the saved PC or sends its credential.
export async function testPcConnection(value, { httpsOnly = false, signal, fetch: send = globalThis.fetch,
  now = Date.now } = {}) {
  const server = normalizePcServer(value, httpsOnly);
  if (!server) throw Error('Informe o endereço do PC primeiro.');
  const deadline = AbortSignal.timeout(5000);
  const combined = signal ? AbortSignal.any([signal, deadline]) : deadline;
  combined.throwIfAborted();
  const started = now();
  const response = await waitForOperation(send(`${server}/api/health`, {
    method: 'GET', cache: 'no-store', credentials: 'omit', redirect: 'error', signal: combined,
  }), { signal: combined });
  const data = await waitForOperation(response.json(), { signal: combined });
  if (!response.ok || data.ok !== true || data.name !== 'modularchat' || data.encrypted !== true) {
    throw Error('O PC respondeu, mas não é um servidor ModularChat válido.');
  }
  return { ...data, server, latencyMs: Math.max(0, Math.round(now() - started)) };
}

// Dependencies are explicit so network changes and concurrent probes can be verified in isolation.
export function createServerConnection({ storage, native, token, fetch: probe, now = Date.now, onChange = () => {},
  onStatus: notifyStatus = () => {}, embedded = null, sync = null, mode = 'legacy', timers = globalThis }) {
  let health = null;
  const onStatus = detail => { health = detail; notifyStatus(detail); };
  let active = null, validUntil = 0, syncedUntil = 0, syncRetryAt = 0, epoch = 0, pending = null, boundary = null, stopWatching = null;
  let preferredKind = 'auto';
  const leases = new Map();
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
    const previous = pending;
    pending = null;
    previous?.controller.abort(error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.'));
  }
  function configure({ local: newLocal = '', remote: newRemote = '' } = {}) {
    const normalize = native() ? normalizePcServer : normalizeServer;
    const next = { local: normalize(newLocal, false), remote: normalize(newRemote, true) };
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
  const kind = () => active?.kind || (local() ? 'local' : native() ? mode === 'auto' ? 'starting' : 'pc' : 'browser');
  const snapshot = () => JSON.stringify([epoch, current(), kind()]);
  const hasLocal = () => !!local();
  function hasServer() {
    if (mode === 'auto' && native() && (embedded?.supported?.() || local())) return true;
    if (local()) return true;
    const saved = configured();
    try { return !!(normalizeServer(saved.local, false) || normalizeServer(saved.remote)); } catch { return false; }
  }
  async function available(server, key, signal, timeoutMs) {
    const timeout = new AbortController();
    const deadline = timeoutMs || (local() ? 400 : 2500);
    const timer = timers.setTimeout(() => timeout.abort(), deadline);
    try {
      const combined = signal ? AbortSignal.any([signal, timeout.signal]) : timeout.signal;
      combined.throwIfAborted();
      const response = await waitForOperation(probe(`${server}/api/health`, {
        cache: 'no-store', credentials: 'omit', redirect: 'error',
        headers: key ? { Authorization: `Bearer ${key}` } : {},
        signal: combined,
      }), { signal: combined });
      const data = await waitForOperation(response.json(), { signal: combined });
      // Revocation is an authentication event, not a reason to switch computers.
      if (mode === 'legacy' && response.status === 401 && data.code === 'UNPAIRED') return true;
      return response.ok && data.ok === true && data.name === 'modularchat' && data.encrypted === true;
    } catch (failure) { if (signal?.aborted) throw signal.reason || failure; return false; }
    finally { timers.clearTimeout(timer); }
  }
  const waitFor = (promise, signal) => waitForOperation(promise, { signal });
  async function consume(job, signal) {
    job.consumers++;
    try { return await waitFor(job.promise, signal); }
    finally {
      if (--job.consumers === 0 && !job.settled) job.controller.abort(error('STALE_CONNECTION', 'A seleção foi cancelada.'));
    }
  }
  async function resolveTarget(signal, { fresh = false, requiredKind } = {}) {
    signal?.throwIfAborted();
    if (!native()) return captured({ server: current() ? normalizeServer(current(), mode === 'auto') : '', kind: 'browser' });
    if (pending?.epoch === epoch && !pending.controller.signal.aborted) {
      if (!requiredKind || pending.requiredKind === requiredKind) return consume(pending, signal);
      invalidate();
    }
    if (leases.size) {
      if (requiredKind) throw error('ORIGIN_BUSY', 'Aguarde a operação do servidor de origem antes de trocar de host.');
      return captured(active || leases.values().next().value || local());
    }
    if (!fresh && active && now() < validUntil) return captured(active);
    const startedAt = epoch;
    const controller = new AbortController();
    const pcToken = token() || '';
    const wasPc = active?.kind === 'pc';
    const check = () => {
      if (startedAt !== epoch || controller.signal.aborted) throw error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.');
    };
    const job = { epoch: startedAt, requiredKind, controller, consumers: 0, settled: false };
    // Register the transaction before any await, including native startup.
    pending = job;
    job.promise = (async () => {
      let startupFailure;
      if (mode === 'auto' && embedded?.ensureStarted && !embedded.target()) {
        try {
          await waitForOperation(embedded.ensureStarted(), { signal: controller.signal, timeoutMs: 5000, timers,
            timeoutError: error('LOCAL_HOST_TIMEOUT', 'O host Android está demorando para iniciar. Tente novamente.') });
        } catch (failure) {
          check();
          startupFailure = failure.code ? failure : error('LOCAL_HOST_FAILED', failure.message || 'Falha ao iniciar o host Android.');
        }
      }
      check();
      const host = local(), desired = requiredKind || preferredKind;
      if (mode === 'auto' && requiredKind === 'pc' && !host) throw startupFailure || error('LOCAL_HOST_UNAVAILABLE', 'O host Android precisa estar disponível para sincronizar com o PC.');
      const activate = async () => {
        check();
        try { await embedded.activate(controller.signal); }
        catch (failure) { check(); throw failure.code ? failure : error('LOCAL_ACTIVATE_FAILED', failure.message || 'Falha ao ativar o host Android.'); }
        check();
      };
      let lan = '', remote = '', selected = null, syncResult, pcReachable = false;
      if (desired !== 'local') {
        const saved = configured();
        try { lan = normalizePcServer(saved.local, false); remote = normalizePcServer(saved.remote, true); }
        catch (failure) { if (!host || requiredKind === 'pc') throw failure; onStatus({ code: 'INVALID_PC_CONFIG', message: failure.message, kind: 'local' }); }
        if (lan && await available(lan, pcToken, controller.signal)) selected = { server: lan, kind: 'pc' };
        else if (remote && remote !== lan && await available(remote, pcToken, controller.signal)) selected = { server: remote, kind: 'pc' };
        check();
        pcReachable = !!selected;
      }
      if (desired === 'local' && !host) throw startupFailure || error('LOCAL_HOST_UNAVAILABLE', 'O host Android não está disponível. Tente novamente ou configure o PC.');
      if (host && selected && !pcToken) {
        if (requiredKind === 'pc') throw error('PC_UNPAIRED', 'Faça o pareamento com o PC antes de reconectar.');
        onStatus({ code: 'PC_UNPAIRED', message: 'O PC precisa de pareamento; o host Android continua disponível.', kind: 'local' });
        selected = null;
      }
      if (!requiredKind && !wasPc && host && selected && now() < syncRetryAt) selected = null;
      if (host && selected && (requiredKind || now() >= syncRetryAt) && (active?.server !== selected.server || active?.kind !== 'pc' || now() >= syncedUntil)) {
        boundary = controller;
        try {
          if (!sync) throw error('SYNC_REQUIRED', 'A sincronização precisa ser concluída antes de usar o PC.');
          onStatus({ code: 'SYNCING', kind: 'local' });
          await embedded.standby(controller.signal);
          check();
          syncResult = await sync({ pc: { ...selected, token: pcToken }, mobile: host, signal: controller.signal, pauseMemory: !wasPc || requiredKind === 'pc',
            isCurrent: () => startedAt === epoch && !controller.signal.aborted });
          check();
          if (syncResult?.converged !== true) throw error('INCOMPLETE_SYNC', 'Os cofres ainda não convergiram.');
          syncedUntil = now() + 10000;
          syncRetryAt = 0;
        } catch (failure) {
          check();
          syncResult = undefined;
          onStatus({ code: failure.code || 'SYNC_FAILED', message: failure.message, kind: 'local', data: failure.data });
          syncRetryAt = now() + 10000;
          if (requiredKind === 'pc') {
            if (active?.kind !== 'pc') await activate();
            throw failure;
          }
          // An occupied memory worker must not disconnect a PC already in use.
          // Periodic checks observe it without interrupting its inference.
          if (!wasPc || failure.code !== 'BUSY') selected = null;
        } finally { if (boundary === controller) boundary = null; }
      }
      check();
      if (!selected && requiredKind === 'pc') throw error('PC_UNAVAILABLE', 'Não consegui conectar ao PC. Confira o endereço, a porta, o servidor e o firewall.');
      if (!selected && host) {
        if (wasPc && !pcReachable) {
          onStatus({ code: 'PC_CONNECTION_LOST', message: 'Conexão com o PC perdida.', server: lan || remote, kind: 'pc' });
        }
        await activate(); // Host activation does not mean the model is online.
        selected = host;
      }
      if (!selected) {
        if (wasPc && !pcReachable) {
          onStatus({ code: 'PC_CONNECTION_LOST', message: 'Conexão com o PC perdida.', server: lan || remote, kind: 'pc' });
        }
        if (mode === 'auto') throw startupFailure || error('LOCAL_HOST_UNAVAILABLE', 'O host Android não está disponível. Tente novamente ou configure o PC.');
        throw Error('Não consegui conectar ao PC por HTTPS. Confira os endereços e o certificado.');
      }
      if (startedAt !== epoch || controller.signal.aborted) throw error('STALE_CONNECTION', 'A conexão mudou. Tente novamente.');
      const changed = active?.server !== selected.server || active?.kind !== selected.kind;
      active = selected;
      validUntil = now() + (host ? 10000 : 30000);
      if (requiredKind) preferredKind = requiredKind === 'local' ? 'local' : 'auto';
      if (changed || requiredKind) onChange(selected.server, { kind: selected.kind, sync: syncResult });
      if (syncResult) onStatus({ code: 'SYNCED', kind: selected.kind, skippedVectors: syncResult.skippedVectors,
        compatibleEmbeddings: syncResult.compatibleEmbeddings });
      return captured(selected);
    })();
    const finish = () => { job.settled = true; if (pending === job) pending = null; };
    job.promise.then(finish, finish);
    return consume(job, signal);
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
        const server = fixed.server ? (native() ? normalizePcServer(fixed.server, false) : normalizeServer(fixed.server, false)) : '';
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
    if (write || generation) leases.set(id, target);
    if (write && (origin || pc !== undefined) && target.kind === 'pc') {
      try {
        const control = new AbortController();
        if (!await available(target.server, target.token, signal || control.signal, 3000)) throw error('ORIGIN_UNAVAILABLE', 'Sem conexão com o PC de origem. O envio não foi repetido.');
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
      catch (failure) { if (!stopped && failure.code !== 'STALE_CONNECTION') onStatus({ code: failure.code || 'CONNECTION_FAILED', message: failure.message, kind: kind(), selectionFailed: true }); }
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
    const stopHost = embedded?.subscribe?.(state => {
      if (state.phase === 'ready' && (!pending || pending.controller.signal.aborted)) changed();
    });
    timer = timers.setTimeout(tick, 0);
    const stop = () => {
      if (stopped) return;
      stopped = true;
      timers.clearTimeout(timer);
      invalidate();
      stopHost?.();
      for (const event of ['online', 'offline', 'mc:resume']) win?.removeEventListener(event, changed);
      doc?.removeEventListener('visibilitychange', changed);
      nav?.connection?.removeEventListener('change', changed);
      if (stopWatching === stop) stopWatching = null;
    };
    stopWatching = stop;
    return stop;
  }
  function switchKind(requiredKind, signal) {
    signal?.throwIfAborted();
    if (leases.size) return Promise.reject(error('ORIGIN_BUSY', 'Aguarde a operação do servidor de origem antes de trocar de host.'));
    if (!native()) return Promise.reject(error('HOST_UNAVAILABLE', 'A troca de host exige o aplicativo Android.'));
    if (pending?.requiredKind !== requiredKind || pending.controller.signal.aborted) invalidate();
    return resolveTarget(signal, { fresh: true, requiredKind });
  }
  const switchToLocal = signal => switchKind('local', signal);
  const switchToPc = signal => switchKind('pc', signal);
  async function probePc(signal) {
    signal?.throwIfAborted();
    const saved = configured();
    let lan = '', remote = '';
    try {
      lan = normalizePcServer(saved.local, false);
      remote = normalizePcServer(saved.remote, true);
    } catch {
      return false;
    }
    const pcToken = token() || '';
    if (lan && await available(lan, pcToken, signal, 2500)) return true;
    if (remote && remote !== lan && await available(remote, pcToken, signal, 2500)) return true;
    return false;
  }
  return { configured, configure, current, kind, snapshot, health: () => health, hasLocal, hasServer, resolve, resolveTarget, acquireTarget, invalidate, start, switchToLocal, switchToPc, probePc };
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
  onStatus: detail => {
    window.dispatchEvent(new CustomEvent('mc:connection-health', { detail }));
    if (detail?.code === 'PC_CONNECTION_LOST') {
      window.dispatchEvent(new CustomEvent('mc:pc-connection-lost', { detail }));
    }
  },
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
export const connectionSnapshot = () => connection.snapshot();
export const connectionHealth = () => connection.health();
export const hasEmbeddedServer = () => connection.hasLocal();
export const startConnection = () => connection.start({ window, document, navigator });
export const invalidateConnection = () => connection.invalidate();
export const connectionFailed = target => {
  if (native() && (!target || target.server === connection.current())) {
    if (connection.kind() === 'pc') {
      window.dispatchEvent(new CustomEvent('mc:pc-connection-lost', { detail: { server: target?.server } }));
    }
    connection.invalidate();
  }
};
export const switchToLocal = signal => connection.switchToLocal(signal);
export const switchToPc = signal => connection.switchToPc(signal);
export const probePc = signal => connection.probePc(signal);
