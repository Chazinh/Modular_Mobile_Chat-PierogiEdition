import { Capacitor, registerPlugin } from '@capacitor/core';
import { createRevisionWriter } from './storage-writes.js';

const native = Capacitor.isNativePlatform();
const NativeStorage = registerPlugin('PrivateStorage');
const AAD = new TextEncoder().encode('ModularChat.PrivateCache.v1');
let enabled = false, waitingForPair = false, browserKey = null, state = {}, staged = false;
const writes = createRevisionWriter(async () => {
  const protectedState = await encodePrivate(JSON.stringify(state));
  localStorage.setItem('mc.private.v1', JSON.stringify(protectedState));
});
const privateKey = k => k === 'mc.token' || k.startsWith('mc.draft.') || k.startsWith('mc.pending.') || k.startsWith('mc.sync.');
const b64 = bytes => {
  let text='';for(let i=0;i<bytes.length;i+=8192)text+=String.fromCharCode(...bytes.subarray(i,i+8192));return btoa(text);
};
const bytes = text => Uint8Array.from(atob(text),c=>c.charCodeAt(0));

export const privacyEnabled = () => enabled;
export const privacyNeedsPairing = () => waitingForPair;
export function privateGet(key) { return enabled || waitingForPair ? state[key] ?? null : localStorage.getItem(key); }
export async function privateSet(key,value) {
  if(key.startsWith('mc.sync.')&&!enabled) return Promise.reject(Error('A sincronização exige armazenamento protegido.'));
  if(!enabled&&!waitingForPair) {
    if(localStorage.getItem(key)!==(value??null)) {
      if(value==null)localStorage.removeItem(key);else localStorage.setItem(key,value);
    }
    return;
  }
  const changed = value == null ? Object.hasOwn(state,key) : state[key] !== value;
  if(value==null)delete state[key];else state[key]=value;
  if(waitingForPair) {
    staged ||= changed;
    throw Error('Conecte o servidor para proteger os dados antes de continuar.');
  }
  return writes.save(changed);
}

export async function encodePrivate(value) {
  if(!enabled)return value;
  const text=typeof value==='string'?value:JSON.stringify(value);
  if(native) return {sealed:1,...await NativeStorage.encrypt({text})};
  const nonce=crypto.getRandomValues(new Uint8Array(12));
  const data=await crypto.subtle.encrypt({name:'AES-GCM',iv:nonce,additionalData:AAD},browserKey,new TextEncoder().encode(text));
  return {sealed:1,nonce:b64(nonce),data:b64(new Uint8Array(data))};
}
export async function decodePrivate(value, json = true) {
  if(!value?.sealed)return value;
  if(!enabled)throw Error('O cache está protegido; conecte o servidor para abri-lo.');
  const text=native ? (await NativeStorage.decrypt(value)).text
    : new TextDecoder().decode(await crypto.subtle.decrypt({name:'AES-GCM',iv:bytes(value.nonce),additionalData:AAD},browserKey,bytes(value.data)));
  return json?JSON.parse(text):text;
}

export async function initializePrivacy(server = '', pairingToken) {
  if(native) { await NativeStorage.ready(); enabled=true; }
  else {
    const token=pairingToken||state['mc.token']||localStorage.getItem('mc.token');
    const response=await fetch(`${server}/api/privacy/cache-key`,{signal:AbortSignal.timeout(10000),credentials:'include',headers:token?{Authorization:`Bearer ${token}`}:{}});
    if(response.status===404)return; // fixtures e backend legado
    if(response.status===401){waitingForPair=true;return;}
    if(!response.ok)throw Error('Não foi possível abrir a proteção do cache no PC.');
    const data=await response.json();
    browserKey=await crypto.subtle.importKey('raw',bytes(data.key),'AES-GCM',false,['encrypt','decrypt']);
    enabled=true;
    waitingForPair=false;
  }
  const existing=localStorage.getItem('mc.private.v1');
  if(existing)state={...await decodePrivate(JSON.parse(existing)),...state};
  const legacy=[];
  for(let i=0;i<localStorage.length;i++) {const k=localStorage.key(i);if(privateKey(k)){state[k]=localStorage.getItem(k);legacy.push(k);}}
  if(legacy.length || staged) {
    if(legacy.length)state['mc.migrated']='1';
    await writes.save();
    staged=false;
    for(const k of legacy)localStorage.removeItem(k);
  }
}
