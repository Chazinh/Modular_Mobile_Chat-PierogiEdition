// Writes report durability so privacy migration never removes the only copy after a failure.
import { encodePrivate, decodePrivate, privacyEnabled } from './private-storage.js';
const DB = 'modularchat';
const STORE = 'kv';

export function createKeyValueStore({ database = () => globalThis.indexedDB, storage = () => globalThis.localStorage,
  keyRange = () => globalThis.IDBKeyRange, enabled = privacyEnabled, encode = encodePrivate, decode = decodePrivate } = {}) {
  let dbPromise = null;
  function open() {
    if (dbPromise) return dbPromise;
    const job = new Promise((resolve, reject) => {
      let settled = false;
      const timer = setTimeout(() => fail(Error('Tempo limite ao abrir o cache.')), 5000);
      function fail(error) {
        settled = true;
        clearTimeout(timer);
        reject(error);
      }
      try {
        const req = database().open(DB, 1);
        req.onupgradeneeded = () => req.result.createObjectStore(STORE);
        req.onerror = () => fail(req.error);
        req.onblocked = () => fail(Error('O cache está bloqueado por outra conexão.'));
        req.onsuccess = () => {
          if (settled) { req.result.close(); return; }
          settled = true;
          clearTimeout(timer);
          const db = req.result;
          const release = () => { if (dbPromise === job) dbPromise = null; };
          db.onversionchange = () => { release(); db.close(); };
          db.onclose = release;
          resolve(db);
        };
      } catch (error) { fail(error); }
    });
    dbPromise = job;
    job.catch(() => { if (dbPromise === job) dbPromise = null; });
    return job;
  }
  async function idb(mode, work, result = request => request?.result) {
    let db = await open(), tx;
    try { tx = db.transaction(STORE, mode); }
    catch (error) {
      if (error.name !== 'InvalidStateError') throw error;
      dbPromise = null;
      db.close();
      db = await open();
      tx = db.transaction(STORE, mode);
    }
    return new Promise((resolve, reject) => {
      let request;
      tx.oncomplete = () => {
        try { resolve(result(request)); } catch (error) { reject(error); }
      };
      tx.onerror = tx.onabort = () => reject(tx.error || Error('Transação do cache interrompida.'));
      try { request = work(tx.objectStore(STORE)); }
      catch (error) { tx.abort(); reject(error); }
    });
  }
  const protect = value => enabled() ? encode(JSON.stringify(value)) : value;
  async function storeProtected(key, value) {
    try {
      await idb('readwrite', s => s.put(value, key));
      return 'idb';
    } catch (idbError) {
      try {
        storage().setItem(`mc.kv.${key}`, JSON.stringify(value));
        return 'localStorage';
      } catch (fallbackError) {
        throw new AggregateError([idbError, fallbackError], 'Não foi possível persistir o cache.');
      }
    }
  }
  const kv = {
    async migratePrivacy() {
      if (!enabled()) return;
      // One cursor entry per completed transaction; crypto never holds a transaction open.
      let available = true;
      try { await open(); } catch { available = false; }
      if (available) {
        let after;
        for (;;) {
          const entry = await idb('readonly', s => s.openCursor(after === undefined ? undefined : keyRange().lowerBound(after, true)),
            request => request.result ? { key: request.result.key, value: request.result.value } : null);
          if (!entry) break;
          if (!entry.value?.sealed) {
            const protectedValue = await protect(entry.value);
            // Only a committed replacement in the source database counts as migrated.
            await idb('readwrite', s => s.put(protectedValue, entry.key));
          }
          after = entry.key;
        }
      }
      const oldKeys = [];
      for (let i = 0; i < storage().length; i++) {
        const key = storage().key(i);
        if (key?.startsWith('mc.kv.')) oldKeys.push(key);
      }
      for (const source of oldKeys) {
        const raw = storage().getItem(source);
        if (raw === null) continue;
        const value = JSON.parse(raw);
        const protectedValue = value?.sealed ? value : await protect(await decode(value));
        const destination = await storeProtected(source.slice(6), protectedValue);
        // The fallback IS this localStorage key; retain it when IDB is unavailable.
        if (destination === 'idb' && storage().getItem(source) === raw) storage().removeItem(source);
      }
    },
    async get(key, fallback = null) {
      let value;
      // A successful fallback write may be newer than the value left in IDB by a failed put.
      try {
        const raw = storage().getItem(`mc.kv.${key}`);
        if (raw !== null) value = JSON.parse(raw);
      } catch { /* IDB may still be readable. */ }
      if (value === undefined) {
        try { value = await idb('readonly', s => s.get(key)); } catch { return fallback; }
      }
      if (value === undefined) return fallback;
      try {
        const data = await decode(value);
        if (enabled() && !value?.sealed) {
          try { await kv.set(key, data); } catch { /* Keep the readable source for retry. */ }
        }
        return data;
      } catch { return fallback; }
    },
    async set(key, value) {
      // Crypto is outside the fallback catch: failure never writes plaintext.
      const protectedValue = await protect(value);
      const destination = await storeProtected(key, protectedValue);
      if (destination === 'idb') {
        const fallbackKey = `mc.kv.${key}`;
        if (storage().getItem(fallbackKey) !== null) storage().removeItem(fallbackKey);
      }
    },
    async del(key) {
      const results = await Promise.allSettled([
        idb('readwrite', s => s.delete(key)),
        Promise.resolve().then(() => storage().removeItem(`mc.kv.${key}`)),
      ]);
      const errors = results.filter(result => result.status === 'rejected').map(result => result.reason);
      if (errors.length) throw new AggregateError(errors, 'Não foi possível remover todas as cópias do cache.');
    },
  };
  return kv;
}

export const kv = createKeyValueStore();
