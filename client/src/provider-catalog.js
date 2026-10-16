import { createLatestRequest } from './connection-lifetime.js';

export const PROVIDER_CATALOG_CHANGED = 'mc:providers-change';
export function notifyProviderCatalog() {
  globalThis.window?.dispatchEvent(new Event(PROVIDER_CATALOG_CHANGED));
}

// Refresh only the selector data. This never loads a chat, changes its route or touches a generation.
export function watchProviderCatalog({ window: win, snapshot, getCached, getPresets, onPresets }) {
  const reads = createLatestRequest(snapshot);
  let stopped = false, received = false;
  const initial = snapshot();
  Promise.resolve().then(getCached).then(value => {
    if (!stopped && !received && snapshot() === initial) onPresets(value);
  }).catch(() => {});
  const refresh = () => reads.run('presets', async (signal, isCurrent) => {
    const value = await getPresets({ signal, isCurrent });
    if (!stopped && isCurrent()) { received = true; onPresets(value); }
  }).catch(() => {});
  const changed = () => { reads.cancel(); void refresh(); };
  win.addEventListener(PROVIDER_CATALOG_CHANGED, changed);
  win.addEventListener('mc:connection-change', changed);
  void refresh();
  return () => {
    stopped = true; reads.cancel();
    win.removeEventListener(PROVIDER_CATALOG_CHANGED, changed);
    win.removeEventListener('mc:connection-change', changed);
  };
}
