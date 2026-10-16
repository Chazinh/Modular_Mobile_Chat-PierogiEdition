import { waitForOperation } from './connection-lifetime.js';

// A successful pairing is never replayed. Only the subsequent synchronization
// may wait for a live generation/job to finish, with cancellation and a deadline.
export async function synchronizePairedPc({ connect, signal, onWaiting = () => {},
  timers = globalThis, retryMs = 10000, timeoutMs = 300000 }) {
  const deadline = new AbortController();
  const timeout = timers.setTimeout(() => deadline.abort(Object.assign(
    Error('O pareamento foi salvo, mas o trabalho ativo ainda não terminou. Tente conectar novamente, sem gerar outro código.'),
    { code: 'SYNC_WAIT_TIMEOUT' })), timeoutMs);
  const combined = signal ? AbortSignal.any([signal, deadline.signal]) : deadline.signal;
  try {
    for (;;) {
      combined.throwIfAborted();
      try {
        const target = await waitForOperation(connect(combined), { signal: combined });
        combined.throwIfAborted();
        if (target?.kind !== 'pc') throw Object.assign(Error('A sincronização não selecionou o PC.'), { code: 'INCOMPLETE_SYNC' });
        return target;
      } catch (failure) {
        combined.throwIfAborted();
        if (failure.code !== 'BUSY') throw failure;
        onWaiting();
        await new Promise((resolve, reject) => {
          const abort = () => { timers.clearTimeout(timer); reject(combined.reason); };
          const timer = timers.setTimeout(() => { combined.removeEventListener('abort', abort); resolve(); }, retryMs);
          combined.addEventListener('abort', abort, { once: true });
        });
      }
    }
  } finally { timers.clearTimeout(timeout); }
}
