import { Capacitor, registerPlugin } from '@capacitor/core';

const LocalHost = registerPlugin('LocalHost');
const BASE_URL = 'http://127.0.0.1:3011';

// The Keystore owns persistence. This adapter keeps the returned credential only in memory.
export function createLocalHost({ native, available, plugin, fetch: send }) {
  let ready = null, starting = null, lifecycle = 'unknown', lifecycleEpoch = 0, queue = Promise.resolve();
  const supported = () => native() && available();
  async function ensureStarted() {
    if (!supported()) return null;
    if (ready) return ready;
    if (starting) return starting;
    starting = (async () => {
      const result = await plugin.ensureStarted();
      if (result?.baseUrl !== BASE_URL || result.status !== 2 || typeof result.token !== 'string'
          || !result.token.trim() || /[\r\n]/.test(result.token)) {
        throw Object.assign(Error('O host Android não retornou um endereço e uma credencial válidos.'), { code: 'INVALID_LOCAL_HOST' });
      }
      ready = Object.freeze({ server: BASE_URL, token: result.token, kind: 'local' });
      return ready;
    })();
    try { return await starting; } finally { starting = null; }
  }
  async function request(path, method = 'GET', { signal, body } = {}) {
    if (!ready) throw Object.assign(Error('O host Android ainda não está pronto.'), { code: 'LOCAL_HOST_NOT_READY' });
    const target = ready;
    const timeout = AbortSignal.timeout(15000);
    const response = await send(`${target.server}/api/local/${path}`, {
      method, redirect: 'error', credentials: 'omit', cache: 'no-store',
      headers: { Authorization: `Bearer ${target.token}`, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    });
    const data = await response.json();
    if (!response.ok) throw Object.assign(Error(data.error || `Host Android: HTTP ${response.status}`), { status: response.status, code: data.code, data });
    return data;
  }
  function transition(next, signal) {
    const task = queue.catch(() => {}).then(async () => {
      signal?.throwIfAborted();
      if (lifecycle === next) return;
      const epoch = lifecycleEpoch;
      const data = await request(next, 'POST', { signal });
      if (epoch === lifecycleEpoch) lifecycle = next;
      return data;
    });
    queue = task;
    return task;
  }
  return {
    supported, ensureStarted, target: () => ready,
    invalidateLifecycle: () => { lifecycle = 'unknown'; lifecycleEpoch++; },
    activate: signal => transition('activate', signal),
    standby: signal => transition('standby', signal),
    status: signal => request('status', 'GET', { signal }),
  };
}

export const localHost = createLocalHost({
  native: () => Capacitor.isNativePlatform(),
  available: () => Capacitor.isPluginAvailable('LocalHost'),
  plugin: LocalHost, fetch: (...args) => fetch(...args),
});
