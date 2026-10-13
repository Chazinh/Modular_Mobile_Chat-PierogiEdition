// Key/value cache on the device (IndexedDB, with localStorage as a fallback). Failing here never
// breaks the app: the PC is the source of truth and the cache only allows opening without the server.
import { encodePrivate, decodePrivate, privacyEnabled } from './private-storage.js';
const DB = 'modularchat';
const STORE = 'kv';

let dbPromise = null;
function open() {
  if (!dbPromise) {
    dbPromise = new Promise((resolve, reject) => {
      try {
        const req = indexedDB.open(DB, 1);
        req.onupgradeneeded = () => req.result.createObjectStore(STORE);
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      } catch (e) {
        reject(e);
      }
    });
  }
  return dbPromise;
}

function idb(mode, fn) {
  return open().then(
    (db) =>
      new Promise((resolve, reject) => {
        const tx = db.transaction(STORE, mode);
        const req = fn(tx.objectStore(STORE));
        tx.oncomplete = () => resolve(req?.result);
        tx.onerror = () => reject(tx.error);
      }),
  );
}

export const kv = {
  async migratePrivacy() {
    if(!privacyEnabled())return;
    const db=await open();
    const entries=await new Promise((resolve,reject)=>{
      const tx=db.transaction(STORE,'readonly'),store=tx.objectStore(STORE);
      const keys=store.getAllKeys(),values=store.getAll();
      tx.oncomplete=()=>resolve(keys.result.map((key,i)=>[key,values.result[i]]));
      tx.onerror=()=>reject(tx.error);
    });
    for(const [key,value] of entries)if(!value?.sealed)await kv.set(key,value);
    const oldKeys=[];
    for(let i=0;i<localStorage.length;i++) {const key=localStorage.key(i);if(key.startsWith('mc.kv.'))oldKeys.push(key);}
    for(const key of oldKeys) {
      const raw=JSON.parse(localStorage.getItem(key));
      await kv.set(key.slice(7),await decodePrivate(raw));
      localStorage.removeItem(key);
    }
  },
  async get(key, fallback = null) {
    try {
      const v = await idb('readonly', (s) => s.get(key));
      if(v===undefined)return fallback;
      const data=await decodePrivate(v);
      if(privacyEnabled()&&!v?.sealed)await kv.set(key,data);
      return data;
    } catch {
      try {
        const raw = localStorage.getItem(`mc.kv.${key}`);
        const parsed=raw?JSON.parse(raw):null;
        return parsed ? await decodePrivate(parsed) : fallback;
      } catch {
        return fallback;
      }
    }
  },
  async set(key, value) {
    const protectedValue=privacyEnabled()?await encodePrivate(JSON.stringify(value)):value;
    try {
      await idb('readwrite', (s) => s.put(protectedValue, key));
    } catch {
      try {
        localStorage.setItem(`mc.kv.${key}`, JSON.stringify(protectedValue));
      } catch {
        /* cota cheia */
      }
    }
  },
  async del(key) {
    try {
      await idb('readwrite', (s) => s.delete(key));
    } catch {
      try {
        localStorage.removeItem(`mc.kv.${key}`);
      } catch {
        /* nada */
      }
    }
  },
};
