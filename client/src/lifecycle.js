// Android may freeze the WebView. The native resume also works when it
// does not deliver visibilitychange, and restarts sockets that went silent.
import { useEffect } from 'react';
import { isNative } from './api.js';

export function useResume(callback) {
  useEffect(() => {
    const visible = () => { if (document.visibilityState === 'visible') callback(); };
    document.addEventListener('visibilitychange', visible);
    window.addEventListener('mc:resume', callback);
    window.addEventListener('online', callback);
    return () => {
      document.removeEventListener('visibilitychange', visible);
      window.removeEventListener('mc:resume', callback);
      window.removeEventListener('online', callback);
    };
  }, [callback]);
}

export function useNativeLifecycle() {
  useEffect(() => {
    if (!isNative()) return;
    let alive = true, listener;
    import('@capacitor/app').then(async ({ App }) => {
      listener = await App.addListener('appStateChange', ({ isActive }) => {
        if (isActive) window.dispatchEvent(new Event('mc:resume'));
      });
      if (!alive) listener.remove();
    }).catch(() => {});
    return () => { alive = false; listener?.remove(); };
  }, []);
}
