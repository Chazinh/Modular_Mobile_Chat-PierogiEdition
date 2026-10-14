import { tr, useLanguage } from './i18n.js';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { api, config, ApiError, isNative } from './api.js';
import { invalidateConnection, startConnection } from './server-connection.js';
import { useLayer, installNativeBack, closeTopLayer } from './nav.js';
import ChatList from './components/ChatList.jsx';
import ChatView from './components/ChatView.jsx';
import GlobalSettings from './components/GlobalSettings.jsx';
import ServerSetup from './components/ServerSetup.jsx';
import { useResume, useNativeLifecycle } from './lifecycle.js';
import { privacyNeedsPairing } from './private-storage.js';
import { useEvent } from './ui.jsx';
import { transitionTo } from './press-feedback.js';
import AutoHostStatus from './components/AutoHostStatus.jsx';
import { shouldShowServerSetup } from './mobile-settings.js';

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
  const [switchingArchived, setSwitchingArchived] = useState(false);
  const switchingRef = useRef(false);
  const [query, setQuery] = useState('');
  const [offline, setOffline] = useState(false);
  const [error, setError] = useState('');
  const [connectionError, setConnectionError] = useState('');
  // Home status dot: 'checking' until the first answer, then 'online'/'offline'; a manual check returns to 'checking'.
  const [status, setStatus] = useState('checking');
  const [openId, setOpenId] = useState(null);
  const [openSettings, setOpenSettings] = useState(false);
  const [globalOpen, setGlobalOpen] = useState(false);
  const [needsServer, setNeedsServer] = useState(config.needsServer);
  const [unpaired, setUnpaired] = useState(() => !config.embeddedReady && privacyNeedsPairing());
  const [manualServerSetup, setManualServerSetup] = useState(false);
  const [fontSize, setFontSize] = useFontSize();
  const refreshTimer = useRef(null);
  const listViewRef = useRef(null);
  listViewRef.current = { archivedView, query };

  useLayer(!!openId, () => setOpenId(null));
  useLayer(globalOpen, () => setGlobalOpen(false));
  useLayer(manualServerSetup, () => {
    setManualServerSetup(false);
    setNeedsServer(config.needsServer);
    setUnpaired(!config.embeddedReady && privacyNeedsPairing());
  });
  useEffect(() => {
    installNativeBack();
  }, []);
  useEffect(() => startConnection(), []);

  const refresh = useCallback(async () => {
    try {
      const list = await api.listChats(archivedView, query);
      if (listViewRef.current.archivedView !== archivedView || listViewRef.current.query !== query) return;
      setChats((prev) => shareItems(prev, list));
      setOffline(false);
      setStatus('online');
      setError(''); setConnectionError('');
    } catch (e) {
      if (listViewRef.current.archivedView !== archivedView || listViewRef.current.query !== query) return;
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
    const on = e => {
      if (e.detail?.kind === 'local') { setConnectionError('O host Android recusou sua credencial. Reabra o app para recuperar o acesso protegido.'); return; }
      if (config.embeddedReady) {
        invalidateConnection();
        setConnectionError('O pareamento com o PC foi revogado. O host Android continua disponível.');
        refresh();
      } else setUnpaired(true);
    };
    const health = e => {
      if (e.detail.message) setConnectionError(e.detail.message);
      else if (e.detail.code === 'SYNCED') setConnectionError('');
    };
    window.addEventListener('mc:unpaired', on);
    window.addEventListener('mc:connection-health', health);
    return () => {
      window.removeEventListener('mc:unpaired', on);
      window.removeEventListener('mc:connection-health', health);
    };
  }, [refresh]);

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
    const connectionChanged = () => {
      setNeedsServer(config.needsServer);
      if (config.embeddedReady) setUnpaired(false); setConnectionError('');
      refresh();
      subscribe();
      window.dispatchEvent(new CustomEvent('mc:event', { detail: { type: 'connection' } }));
    };
    subscribe();
    document.addEventListener('visibilitychange', visible);
    window.addEventListener('mc:resume', subscribe);
    window.addEventListener('online', subscribe);
    window.addEventListener('mc:connection-change', connectionChanged);
    return () => {
      ac?.abort();
      clearTimeout(refreshTimer.current);
      document.removeEventListener('visibilitychange', visible);
      window.removeEventListener('mc:resume', subscribe);
      window.removeEventListener('online', subscribe);
      window.removeEventListener('mc:connection-change', connectionChanged);
    };
  }, [needsServer, unpaired, refresh]);

  // Pull to refresh: discard the cached endpoint choice, verify the connection again and reload the list.
  const checkNow = useEvent(async () => {
    invalidateConnection();
    setStatus('checking'); setError(''); setConnectionError('');
    await Promise.all([refresh(), new Promise((r) => setTimeout(r, 450))]);
  });

  const openChat = useEvent((id, origin) => {
    transitionTo(() => { setOpenSettings(false); setOpenId(id); }, '.mc-chat', origin);
  });
  const toggleArchived = useEvent(async (event) => {
    if (switchingRef.current) return;
    const origin = event?.currentTarget;
    const next = !archivedView;
    switchingRef.current = true;
    setSwitchingArchived(true);
    try {
      const list = await api.listChats(next, query);
      transitionTo(() => {
        setArchivedView(next);
        setChats(prev => shareItems(prev, list));
        setOffline(false); setStatus('online'); setError('');
      }, '.mc-list', origin);
    } catch (e) { setError(e.message); }
    finally { switchingRef.current = false; setSwitchingArchived(false); }
  });
  const openGlobal = useEvent((event) => transitionTo(() => setGlobalOpen(true), '.mc-screen', event?.currentTarget));

  async function create(event) {
    const origin = event?.currentTarget;
    try {
      const chat = await api.createChat({});
      await refresh();
      transitionTo(() => { setOpenSettings(true); setOpenId(chat.id); }, '.mc-chat', origin);
    } catch (e) {
      setError(tr("Não foi possível criar o chat: {0}", [e.message]));
    }
  }

  if (shouldShowServerSetup({ manual: manualServerSetup, needsServer, unpaired, embeddedReady: config.embeddedReady })) {
    return (
      <ServerSetup
        needsServer={needsServer}
        onCancel={manualServerSetup ? closeTopLayer : undefined}
        onDone={() => {
          if (manualServerSetup) closeTopLayer();
          setNeedsServer(false);
          setUnpaired(false);
          setConnectionError('');
          refresh();
        }}
      />
    );
  }

  const hostStatus = isNative() && config.embeddedReady ? <AutoHostStatus /> : null;
  return (
    <>
      <div className={openId ? 'hidden' : 'flex h-full flex-col'}>
        <ChatList
          hostStatus={!openId && !globalOpen ? hostStatus : null}
          chats={chats}
          offline={offline}
          status={status}
          onRefresh={checkNow}
           error={error || (status === 'online' ? '' : connectionError)}
          archivedView={archivedView}
          switchingArchived={switchingArchived}
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
          hostStatus={!globalOpen ? hostStatus : null}
          key={openId}
          chatId={openId}
          openSettingsInitially={openSettings}
          status={status}
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
            setGlobalOpen(false);
            setManualServerSetup(true);
          }}
        />
      )}
    </>
  );
}

export { ApiError };
