import { useSyncExternalStore } from 'react';
import { Capacitor, registerPlugin } from '@capacitor/core';
import english from './locales/en.json' with { type: 'json' };

const KEY = 'mc.language';
const listeners = new Set();
const NativeLanguage = registerPlugin('AppLanguage');
const supported = value => value === 'en' ? 'en' : 'pt-BR';
let language = 'pt-BR';
try { language = supported(localStorage.getItem(KEY)); } catch { /* preferência local opcional */ }

export const currentLanguage = () => language;
const subscribe = listener => { listeners.add(listener); return () => listeners.delete(listener); };
export const useLanguage = () => useSyncExternalStore(subscribe, currentLanguage, () => 'pt-BR');

function applyLanguage() {
  if (typeof document !== 'undefined') document.documentElement.lang = language;
}

export function setLanguage(value) {
  const next = supported(value);
  try { localStorage.setItem(KEY, next); } catch { /* continua funcionando durante a sessão */ }
  if (Capacitor.isNativePlatform()) NativeLanguage.set({ language: next }).catch(() => {});
  if (next === language) return;
  language = next;
  applyLanguage();
  for (const listener of listeners) listener();
}

export function syncNativeLanguage() {
  if (Capacitor.isNativePlatform()) return NativeLanguage.set({ language }).catch(() => {});
}

// Somente textos da interface chamam tr(). Os valores interpolados (nomes, histórico, etc.) são preservados.
export function tr(source, values = []) {
  if (source == null) return source;
  const text = String(source);
  const key = text.trim().replace(/\s+/g, ' ');
  const translated = language === 'en' ? english[key] : undefined;
  const result = translated == null ? text : text.slice(0, text.length - text.trimStart().length) + translated + text.slice(text.trimEnd().length);
  return result.replace(/\{(\d+)\}/g, (match, index) => index < values.length ? String(values[index] ?? '') : match);
}

applyLanguage();

if (typeof window !== 'undefined') window.addEventListener('storage', event => {
  if (event.key !== KEY) return;
  const next = supported(event.newValue);
  if (next === language) return;
  language = next;
  applyLanguage();
  for (const listener of listeners) listener();
});
