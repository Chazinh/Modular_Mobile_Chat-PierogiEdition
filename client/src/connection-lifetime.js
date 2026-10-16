// Cancel a consumer's wait without cancelling the shared native startup itself.
export async function waitForOperation(promise, { signal, timeoutMs, timers = globalThis, timeoutError } = {}) {
  signal?.throwIfAborted();
  let abort, timer;
  const cancelled = new Promise((_, reject) => {
    if (signal) {
      abort = () => reject(signal.reason || new DOMException('Operação cancelada.', 'AbortError'));
      signal.addEventListener('abort', abort, { once: true });
    }
    if (timeoutMs != null) timer = timers.setTimeout(() => reject(timeoutError || Error('Tempo limite esgotado.')), timeoutMs);
  });
  try { return await Promise.race([promise, cancelled]); }
  finally {
    if (timer != null) timers.clearTimeout(timer);
    if (abort) signal.removeEventListener('abort', abort);
  }
}

// Coalesce equivalent reads, abort superseded ones, and reject replies from another destination/epoch.
export function createLatestRequest(snapshot) {
  let current = null, sequence = 0;
  function cancel() { sequence++; current?.controller.abort(); current = null; }
  function run(key, work) {
    const destination = snapshot();
    if (current?.key === key && current.destination === destination) return current.promise;
    cancel();
    const controller = new AbortController(), version = sequence;
    const entry = { key, destination, controller };
    const isCurrent = () => version === sequence && !controller.signal.aborted && snapshot() === destination;
    current = entry;
    entry.promise = Promise.resolve().then(() => {
      if (controller.signal.aborted) return;
      return work(controller.signal, isCurrent);
    }).finally(() => { if (current === entry) current = null; });
    return entry.promise;
  }
  return { run, cancel };
}

// One native listener for all mounts, including an asynchronous StrictMode mount/cleanup/mount.
export function createSharedListener(install) {
  let users = 0, registration = null;
  return function subscribe() {
    users++;
    if (!registration) {
      const job = Promise.resolve().then(install);
      registration = job;
      job.then(handle => {
        if (!users && registration === job) { registration = null; void handle?.remove(); }
      }, () => { if (registration === job) registration = null; });
    }
    let stopped = false;
    return () => {
      if (stopped) return;
      stopped = true;
      if (--users === 0) {
        const job = registration;
        // Retain an unresolved registration so the next StrictMode mount can share it.
        job?.then(handle => {
          if (!users && registration === job) { registration = null; void handle?.remove(); }
        }, () => {});
      }
    };
  };
}

// Health only opens the promotion attempt. Success requires a synchronized PC target.
export function startReconnectionWait({ probe, reconnect, onProgress, onError, onClose,
  document: doc, window: win, timers = globalThis, now = Date.now, durationMs = 300000 }) {
  const deadline = now() + durationMs;
  let stopped = false, failed = false, succeeded = false, version = 0, controller;
  let tickTimer, probeTimer, closeTimer;
  const visible = () => doc.visibilityState !== 'hidden';
  const remaining = () => Math.max(0, Math.ceil((deadline - now()) / 1000));
  function cancel() {
    version++;
    controller?.abort();
    for (const timer of [tickTimer, probeTimer, closeTimer]) if (timer != null) timers.clearTimeout(timer);
  }
  function tick() {
    if (stopped || !visible() || failed || succeeded) return;
    const timeLeft = remaining();
    onProgress({ timeLeft });
    if (!timeLeft) { cancel(); onProgress({ probing: false }); return; }
    tickTimer = timers.setTimeout(tick, Math.min(1000, deadline - now()));
  }
  async function check() {
    if (stopped || !visible() || !remaining() || failed || succeeded) return;
    const cycle = version, ac = controller = new AbortController();
    const current = () => !stopped && visible() && cycle === version && !ac.signal.aborted && remaining() > 0;
    let promoting = false;
    onProgress({ probing: true });
    try {
      const online = await probe(ac.signal);
      if (!current()) return;
      if (online) {
        promoting = true;
        const target = await reconnect(ac.signal);
        if (!current()) return;
        if (target?.kind !== 'pc') throw Error('A reconexão não selecionou o PC. Tente novamente.');
        succeeded = true;
        if (tickTimer != null) timers.clearTimeout(tickTimer);
        onProgress({ reconnected: true });
        closeTimer = timers.setTimeout(() => { if (!stopped && visible() && cycle === version) onClose(); }, 800);
      }
    } catch (error) {
      if (current() && promoting) {
        failed = true;
        if (tickTimer != null) timers.clearTimeout(tickTimer);
        onError(error);
      }
    } finally {
      if (!stopped && cycle === version && !ac.signal.aborted) {
        onProgress({ probing: false });
        if (visible() && remaining() && !failed && !succeeded) probeTimer = timers.setTimeout(check, 5000);
      }
    }
  }
  function resume() {
    cancel();
    if (stopped) return;
    onProgress({ probing: false, timeLeft: remaining() });
    if (!visible() || failed) return;
    if (succeeded) { closeTimer = timers.setTimeout(() => { if (!stopped && visible()) onClose(); }, 800); return; }
    tick();
    if (remaining()) probeTimer = timers.setTimeout(check, 1500);
  }
  doc.addEventListener('visibilitychange', resume);
  win?.addEventListener('mc:resume', resume);
  resume();
  return { stop() {
    if (stopped) return;
    stopped = true; cancel();
    doc.removeEventListener('visibilitychange', resume);
    win?.removeEventListener('mc:resume', resume);
  } };
}
