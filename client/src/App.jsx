import { tr, useLanguage } from './i18n.js';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { api, config, ApiError, isNative } from './api.js';
import { connectionSnapshot, invalidateConnection, startConnection } from './server-connection.js';
import { createLatestRequest } from './connection-lifetime.js';
import { localHost } from './local-host.js';
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
import ConnectionLossModal from './components/ConnectionLossModal.jsx';
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
  const [createdChat, setCreatedChat] = useState(null);
  const creatingRef = useRef(false);
  const [globalOpen, setGlobalOpen] = useState(false);
  const [needsServer, setNeedsServer] = useState(config.needsServer);
  const [unpaired, setUnpaired] = useState(() => !config.embeddedReady && privacyNeedsPairing());
  const [manualServerSetup, setManualServerSetup] = useState(false);
  const [kind, setKind] = useState(() => config.kind);
  const [connectionLossOpen, setConnectionLossOpen] = useState(false);
  const [hostStartup, setHostStartup] = useState(() => localHost.startupState());
  const [retryingHost, setRetryingHost] = useState(false);
  const retryHostController = useRef(null);
  const wasOnPcRef = useRef(config.kind === 'pc');
  const [fontSize, setFontSize] = useFontSize();
  const refreshTimer = useRef(null);
  const listViewRef = useRef(null);
  listViewRef.current = { archivedView, query };
  const listRequests = useRef(null);
  const hasListed = useRef(false);
  if (!listRequests.current) listRequests.current = createLatestRequest(connectionSnapshot);
  const visibleOrigin = useRef(`${config.kind}:${config.server}`);

  useEffect(() => {
    if (config.kind === 'pc') wasOnPcRef.current = true;
  }, [kind]);

  useLayer(!!openId, () => setOpenId(null));
  useLayer(globalOpen, () => setGlobalOpen(false));
  useLayer(manualServerSetup, () => {
    setManualServerSetup(false);
    setNeedsServer(config.needsServer);
    setUnpaired(!config.embeddedReady && privacyNeedsPairing());
  });
  useEffect(() => installNativeBack(), []);
  useEffect(() => startConnection(), []);

  const refresh = useCallback(() => listRequests.current.run(JSON.stringify([archivedView, query]), async (signal, current) => {
    const isCurrent = () => current() && listViewRef.current.archivedView === archivedView && listViewRef.current.query === query;
    try {
      const list = await api.listChats(archivedView, query, { signal, isCurrent });
      if (!isCurrent()) return;
      hasListed.current = true;
      setChats((prev) => shareItems(prev, list));
      setOffline(false);
      setStatus('online');
      setError(''); setConnectionError('');
    } catch (e) {
      if (!isCurrent() || signal.aborted || e.code === 'STALE_CONNECTION') return;
      setOffline(true);
      setStatus('offline');
      if (e.status !== 401) setError(e.message);
      if (!archivedView && !query) setChats((prev) => (prev.length ? prev : []));
      if (isNative() && wasOnPcRef.current && config.embeddedReady) {
        setConnectionLossOpen(true);
      }
    } finally {
      if (isCurrent()) { switchingRef.current = false; setSwitchingArchived(false); }
    }
  }), [archivedView, query]);

  const handleSwitchToLocal = useCallback(async signal => {
    const target = await api.switchToLocal(signal);
    signal?.throwIfAborted();
    wasOnPcRef.current = false;
    setError('');
    setConnectionError('');
    return target;
  }, []);

  const handleReconnected = useCallback(signal => api.switchToPc(signal), []);

  async function retryLocalHost() {
    if (retryHostController.current) return;
    const ac = retryHostController.current = new AbortController();
    setRetryingHost(true); setStatus('checking'); setError(''); setConnectionError('');
    try { await handleSwitchToLocal(ac.signal); }
    catch (failure) {
      if (!ac.signal.aborted) { setError(failure.message); setOffline(true); setStatus('offline'); }
    } finally {
      if (retryHostController.current === ac) { retryHostController.current = null; setRetryingHost(false); }
    }
  }

  useEffect(() => {
    const onLoss = () => {
      if (isNative() && wasOnPcRef.current && config.embeddedReady) {
        setConnectionLossOpen(true);
      }
    };
    window.addEventListener('mc:pc-connection-lost', onLoss);
    return () => window.removeEventListener('mc:pc-connection-lost', onLoss);
  }, []);

  // first paint from the local cache; then the server
  useEffect(() => {
    // The legacy cache has no host identity; never label a PC list as Android history.
    if (isNative()) return undefined;
    let alive = true;
    const destination = connectionSnapshot();
    api.cachedChats().then((c) => alive && !hasListed.current && destination === connectionSnapshot()
      && !listViewRef.current.archivedView && !listViewRef.current.query && c?.length
      && setChats((prev) => (prev.length ? prev : c))).catch(() => {});
    return () => { alive = false; };
  }, []);

  useEffect(() => {
    if (needsServer && !isNative()) return undefined;
    refresh();
  }, [refresh, needsServer]);
  const resumeList = useCallback(() => { if (!needsServer) refresh(); }, [needsServer, refresh]);
  useResume(resumeList);

  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState !== 'hidden') return;
      listRequests.current.cancel();
      retryHostController.current?.abort(); retryHostController.current = null;
      setRetryingHost(false);
      clearTimeout(refreshTimer.current);
    };
    document.addEventListener('visibilitychange', hidden);
    return () => {
      document.removeEventListener('visibilitychange', hidden);
      listRequests.current.cancel(); retryHostController.current?.abort(); retryHostController.current = null;
      clearTimeout(refreshTimer.current);
    };
  }, []);

  useEffect(() => localHost.subscribe(state => {
    setHostStartup(state);
    setNeedsServer(config.needsServer); setKind(config.kind);
    if (state.phase === 'ready') {
      setUnpaired(false);
      if (document.visibilityState !== 'hidden') refresh();
    }
  }), [refresh]);

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
      if (e.detail.selectionFailed) { setOffline(true); setStatus('offline'); setError(e.detail.message); }
    };
    window.addEventListener('mc:unpaired', on);
    window.addEventListener('mc:connection-health', health);
    return () => {
      window.removeEventListener('mc:unpaired', on);
      window.removeEventListener('mc:connection-health', health);
    };
  }, [refresh]);

  // TOP-LEVEL CONNECTION LISTENER: always listen to mc:connection-change regardless of needsServer/unpaired
  useEffect(() => {
    const connectionChanged = () => {
      const origin = `${config.kind}:${config.server}`;
      const changedHost = visibleOrigin.current !== origin;
      visibleOrigin.current = origin;
      listRequests.current.cancel();
      if (changedHost) { setChats([]); setOpenId(null); setCreatedChat(null); }
      if (config.kind === 'pc') wasOnPcRef.current = true;
      setKind(config.kind);
      setNeedsServer(config.needsServer);
      if (config.embeddedReady) setUnpaired(false);
      setConnectionError('');
      setStatus('checking');
      if (document.visibilityState !== 'hidden') refresh();
      if (!changedHost) window.dispatchEvent(new CustomEvent('mc:event', { detail: { type: 'connection' } }));
    };
    window.addEventListener('mc:connection-change', connectionChanged);
    return () => window.removeEventListener('mc:connection-change', connectionChanged);
  }, [refresh]);

  // server events: sync the list and notify the open chat
  useEffect(() => {
    if (needsServer || unpaired || (isNative() && !config.embeddedReady && kind !== 'pc')) return undefined;
    let ac;
    const subscribe = () => {
      ac?.abort();
      if (document.visibilityState === 'hidden') return;
      ac = new AbortController();
      api.subscribeEvents((ev) => {
        if (ev.type === 'hello') return;
        window.dispatchEvent(new CustomEvent('mc:event', { detail: ev }));
        clearTimeout(refreshTimer.current);
        refreshTimer.current = setTimeout(refresh, 400);
      }, ac.signal);
    };
    const visible = () => { subscribe(); if (document.visibilityState === 'hidden') clearTimeout(refreshTimer.current); };
    subscribe();
    document.addEventListener('visibilitychange', visible);
    window.addEventListener('mc:resume', subscribe);
    window.addEventListener('online', subscribe);
    window.addEventListener('mc:connection-change', subscribe);
    return () => {
      ac?.abort();
      clearTimeout(refreshTimer.current);
      document.removeEventListener('visibilitychange', visible);
      window.removeEventListener('mc:resume', subscribe);
      window.removeEventListener('online', subscribe);
      window.removeEventListener('mc:connection-change', subscribe);
    };
  }, [needsServer, unpaired, refresh, kind]);

  // Pull to refresh: discard the cached endpoint choice, verify the connection again and reload the list.
  const checkNow = useEvent(async () => {
    invalidateConnection();
    setStatus('checking'); setError(''); setConnectionError('');
    await Promise.all([refresh(), new Promise((r) => setTimeout(r, 450))]);
  });

  const openChat = useEvent((id, origin) => {
    transitionTo(() => { setOpenSettings(false); setOpenId(id); }, '.mc-chat', origin);
  });
  const toggleArchived = useEvent((event) => {
    if (switchingRef.current) return;
    const origin = event?.currentTarget;
    const next = !archivedView;
    switchingRef.current = true;
    setSwitchingArchived(true);
    listRequests.current.cancel();
    transitionTo(() => {
      setArchivedView(next); setChats([]); setStatus('checking'); setError('');
    }, '.mc-list', origin);
  });
  const openGlobal = useEvent((event) => transitionTo(() => setGlobalOpen(true), '.mc-screen', event?.currentTarget));

  async function create() {
    if (creatingRef.current) return;
    creatingRef.current = true;
    try {
      const chat = await api.createChat({});
      setCreatedChat(chat);
      setOpenSettings(true);
      setOpenId(chat.id);
      refresh();
    } catch (e) {
      setError(tr("Não foi possível criar o chat: {0}", [e.message]));
    } finally {
      creatingRef.current = false;
    }
  }

  if (shouldShowServerSetup({ manual: manualServerSetup, needsServer, unpaired, embeddedReady: config.embeddedReady })
      && (!isNative() || manualServerSetup || config.kind === 'pc')) {
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

  const hostStatus = isNative() && config.embeddedReady && kind !== 'pc' && config.kind !== 'pc' ? <AutoHostStatus /> : null;
  return (
    <>
      <div className={openId ? 'hidden' : 'flex h-full flex-col'}>
        {isNative() && kind !== 'pc' && (status === 'offline' || hostStartup.phase === 'error') && (
          <div role="alert" className="px-4 py-3 text-sm text-amber-200">
            <p>{error || hostStartup.message || tr('O host Android não está disponível. Os dados foram preservados.')}</p>
            <button type="button" disabled={retryingHost} onClick={retryLocalHost} className="mt-2 rounded-xl bg-white/10 px-4 py-2 disabled:opacity-40">
              {retryingHost ? tr('Iniciando Auto-Hospedagem…') : tr('Tentar Auto-Hospedagem novamente')}
            </button>
          </div>
        )}
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
          fontSize={fontSize}
          onFontSize={setFontSize}
          onResetServer={() => setManualServerSetup(true)}
        />
      </div>
      {openId && (
        <ChatView
          hostStatus={!globalOpen ? hostStatus : null}
          key={openId}
          chatId={openId}
          initialChat={createdChat?.id === openId ? createdChat : null}
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
      <ConnectionLossModal
        isOpen={connectionLossOpen}
        onClose={() => setConnectionLossOpen(false)}
        onSwitchToLocal={handleSwitchToLocal}
        onReconnected={handleReconnected}
      />
    </>
  );
}

export { ApiError };
