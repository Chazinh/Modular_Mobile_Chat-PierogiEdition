import { tr, useLanguage } from './i18n.js';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { api, config, ApiError } from './api.js';
import { invalidateConnection } from './server-connection.js';
import { useLayer, installNativeBack, closeTopLayer } from './nav.js';
import ChatList from './components/ChatList.jsx';
import ChatView from './components/ChatView.jsx';
import GlobalSettings from './components/GlobalSettings.jsx';
import ServerSetup from './components/ServerSetup.jsx';
import { useResume, useNativeLifecycle } from './lifecycle.js';
import { privacyNeedsPairing } from './private-storage.js';
import { useEvent } from './ui.jsx';

// Keeps the reference of items that did not change between two lists from the server (rows are memoized).
function shareItems(prev, next) {
  if (!prev.length) return next;
  const old = new Map(prev.map((c) => [c.id, c]));
  const merged = next.map((c) => {
    const p = old.get(c.id);
    return p && JSON.stringify(p) === JSON.stringify(c) ? p : c;
  });
  return merged.length === prev.length && merged.every((c, i) => c === prev[i]) ? prev : merged;
}

export function useFontSize() {
  const [size, setSize] = useState(() => {
    const v = Number(localStorage.getItem('mc.font'));
    return v >= 14 && v <= 22 ? v : 16;
  });
  useEffect(() => {
    document.documentElement.style.setProperty('--mc-font', `${size}px`);
    try {
      localStorage.setItem('mc.font', String(size));
    } catch {
      /* no storage */
    }
  }, [size]);
  return [size, setSize];
}

export default function App() {
  useLanguage();
  useNativeLifecycle();
  const [chats, setChats] = useState([]);
  const [archivedView, setArchivedView] = useState(false);
  const [query, setQuery] = useState('');
  const [offline, setOffline] = useState(false);
  const [error, setError] = useState('');
  // Home status dot: 'checking' until the first answer, then 'online'/'offline'; a manual check returns to 'checking'.
  const [status, setStatus] = useState('checking');
  const [openId, setOpenId] = useState(null);
  const [openSettings, setOpenSettings] = useState(false);
  const [globalOpen, setGlobalOpen] = useState(false);
  const [needsServer, setNeedsServer] = useState(config.needsServer);
  const [unpaired, setUnpaired] = useState(privacyNeedsPairing);
  const [fontSize, setFontSize] = useFontSize();
  const refreshTimer = useRef(null);

  useLayer(!!openId, () => setOpenId(null));
  useLayer(globalOpen, () => setGlobalOpen(false));
  useEffect(() => {
    installNativeBack();
  }, []);

  const refresh = useCallback(async () => {
    try {
      const list = await api.listChats(archivedView, query);
      setChats((prev) => shareItems(prev, list));
      setOffline(false);
      setStatus('online');
      setError('');
    } catch (e) {
      setOffline(true);
      setStatus('offline');
      if (e.status && e.status !== 401) setError(e.message);
      if (!archivedView && !query) setChats((prev) => (prev.length ? prev : []));
    }
  }, [archivedView, query]);

  // first paint from the local cache; then the server
  useEffect(() => {
    api.cachedChats().then((c) => c?.length && setChats((prev) => (prev.length ? prev : c)));
  }, []);

  useEffect(() => {
    if (needsServer) return undefined;
    refresh();
  }, [refresh, needsServer]);
  const resumeList = useCallback(() => { if (!needsServer) refresh(); }, [needsServer, refresh]);
  useResume(resumeList);

  useEffect(() => {
    const on = () => setUnpaired(true);
    window.addEventListener('mc:unpaired', on);
    return () => window.removeEventListener('mc:unpaired', on);
  }, []);

  // server events: sync the list and notify the open chat
  useEffect(() => {
    if (needsServer || unpaired) return undefined;
    let ac;
    const subscribe = () => {
    ac?.abort();
    ac = new AbortController();
    api.subscribeEvents((ev) => {
      if (ev.type === 'hello') return;
      window.dispatchEvent(new CustomEvent('mc:event', { detail: ev }));
      clearTimeout(refreshTimer.current);
      refreshTimer.current = setTimeout(refresh, 400);
    }, ac.signal);
    };
    const visible = () => { if (document.visibilityState === 'visible') subscribe(); };
    subscribe();
    document.addEventListener('visibilitychange', visible);
    window.addEventListener('mc:resume', subscribe);
    window.addEventListener('online', subscribe);
    return () => {
      ac?.abort();
      clearTimeout(refreshTimer.current);
      document.removeEventListener('visibilitychange', visible);
      window.removeEventListener('mc:resume', subscribe);
      window.removeEventListener('online', subscribe);
    };
  }, [needsServer, unpaired, refresh]);

  // Pull to refresh: discard the cached endpoint choice, verify the connection again and reload the list.
  const checkNow = useEvent(async () => {
    invalidateConnection();
    setStatus('checking');
    await Promise.all([refresh(), new Promise((r) => setTimeout(r, 450))]);
  });

  const openChat = useEvent((id) => {
    setOpenSettings(false);
    setOpenId(id);
  });
  const toggleArchived = useEvent(() => setArchivedView((v) => !v));
  const openGlobal = useEvent(() => setGlobalOpen(true));

  async function create() {
    try {
      const chat = await api.createChat({});
      await refresh();
      setOpenSettings(true);
      setOpenId(chat.id);
    } catch (e) {
      setError(tr("Não foi possível criar o chat: {0}", [e.message]));
    }
  }

  if (needsServer || unpaired) {
    return (
      <ServerSetup
        needsServer={needsServer}
        onDone={() => {
          setNeedsServer(false);
          setUnpaired(false);
          refresh();
        }}
      />
    );
  }

  return (
    <>
      <div className={openId ? 'hidden' : 'flex h-full flex-col'}>
        <ChatList
          chats={chats}
          offline={offline}
          status={status}
          onRefresh={checkNow}
          error={error}
          archivedView={archivedView}
          query={query}
          onQuery={setQuery}
          onToggleArchived={toggleArchived}
          onOpen={openChat}
          onCreate={create}
          onChanged={refresh}
          onGlobal={openGlobal}
        />
      </div>
      {openId && (
        <ChatView
          key={openId}
          chatId={openId}
          openSettingsInitially={openSettings}
          onBack={closeTopLayer}
          onChanged={refresh}
          onCreate={create}
          onDeleted={() => {
            setOpenId(null);
            refresh();
          }}
        />
      )}
      {globalOpen && (
        <GlobalSettings
          fontSize={fontSize}
          onFontSize={setFontSize}
          onClose={closeTopLayer}
          onChanged={refresh}
          onResetServer={() => {
            config.server = '';
            config.token = '';
            setGlobalOpen(false);
            setNeedsServer(config.needsServer);
            setUnpaired(!config.needsServer);
          }}
        />
      )}
    </>
  );
}

export { ApiError };
