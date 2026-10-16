// Best-effort cache: retain one latest value per key, including while its write is in flight.
// A failed key stays readable/retryable, but is attempted only once per drain unless superseded.
export function createCacheWriter(store, timers = globalThis) {
  const pending = new Map();
  let scheduled = null, draining = null;

  function cancelSchedule() {
    if (!scheduled) return;
    const { id, idle } = scheduled;
    scheduled = null;
    if (idle) timers.cancelIdleCallback(id); else timers.clearTimeout(id);
  }
  function flush() {
    cancelSchedule();
    if (draining) return draining;
    draining = Promise.resolve().then(async () => {
      const failed = new Set();
      for (;;) {
        let next;
        for (const key of pending.keys()) { if (!failed.has(key)) { next = key; break; } }
        if (next === undefined) break;
        const entry = pending.get(next);
        try {
          if (entry.deleted) await store.del(next); else await store.set(next, entry.value);
          if (pending.get(next) === entry) pending.delete(next);
        } catch {
          if (pending.get(next) === entry) failed.add(next);
        }
      }
    }).finally(() => { draining = null; });
    return draining;
  }
  function queue(key, value) {
    // An obsolete UI save must not replace a queued deletion.
    if (pending.get(key)?.deleted) return;
    pending.set(key, { value });
    if (scheduled || draining) return;
    const idle = typeof timers.requestIdleCallback === 'function' && typeof timers.cancelIdleCallback === 'function';
    const run = () => { scheduled = null; void flush(); };
    scheduled = { idle, id: idle ? timers.requestIdleCallback(run, { timeout: 2500 }) : timers.setTimeout(run, 400) };
  }
  async function get(key, fallback) {
    const latest = () => pending.get(key)?.deleted ? fallback : pending.get(key)?.value;
    if (pending.has(key)) return latest();
    const stored = await store.get(key, fallback);
    return pending.has(key) ? latest() : stored;
  }
  function remove(key) {
    pending.set(key, { deleted: true });
    return flush();
  }
  return { queue, get, remove, flush };
}

// Durable state: callers carry only a revision, never a captured JSON snapshot.
// Capture state when the writer starts; acknowledge only the revision actually persisted.
export function createRevisionWriter(write) {
  let revision = 0, durable = 0, running = false, waiters = [];
  function save(changed = true) {
    if (changed) revision++;
    if (revision === durable) return Promise.resolve();
    const result = new Promise((resolve, reject) => waiters.push({ revision, resolve, reject }));
    if (!running) {
      running = true;
      void Promise.resolve().then(async () => {
        try {
          while (durable < revision) {
            const covering = revision;
            await write();
            durable = covering;
            const completed = waiters.filter(w => w.revision <= durable);
            waiters = waiters.filter(w => w.revision > durable);
            for (const waiter of completed) waiter.resolve();
          }
        } catch (error) {
          const rejected = waiters;
          waiters = [];
          for (const waiter of rejected) waiter.reject(error);
          // Keep the dirty revision; the next explicit save (even a no-op) retries it.
        } finally { running = false; }
      });
    }
    return result;
  }
  return { save };
}
