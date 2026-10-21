import React, { useEffect, useRef, useState } from 'react';
import { api, config, isNative } from '../api.js';
import { tr, useLanguage } from '../i18n.js';
import { downloadProgress, startStatusPolling } from '../autohost-status.js';
import { connectionHealth } from '../server-connection.js';
import PcConnectionButton from './PcConnectionButton.jsx';

const phases = { pending: 'Aguardando download', downloading: 'Baixando', verifying: 'Verificando arquivo', ready: 'Arquivo disponível', error: 'Erro no download', cancelled: 'Download pausado' };
const toMB = bytes => ((Number(bytes) || 0) / (1024 * 1024)).toLocaleString(undefined, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
function DownloadStatus({ model, title }) {
  const progress = downloadProgress(model);
  return <div>
    <div className="flex flex-wrap justify-between gap-x-2"><span>{title} · {tr(phases[progress.phase] || 'Status desconhecido')}</span>
      <span className="tabular-nums">{toMB(progress.downloaded)} / {toMB(progress.total)} MB{progress.percent != null && ` · ${progress.percent}%`}</span></div>
    <progress aria-label={title} value={progress.downloaded} max={progress.total || 1} className="mt-1 h-1 w-full accent-[#7c3aed]" />
    {progress.error && <p className="break-words text-red-300">{tr(progress.error)}</p>}
  </div>;
}

// Polling state stays here; neither App nor ChatView receives two-second updates.
export default function AutoHostStatus() {
  useLanguage();
  const [status, setStatus] = useState(null);
  const [kind, setKind] = useState(() => config.kind);
  const [health, setHealth] = useState(() => connectionHealth());
  const [error, setError] = useState('');
  const [retrying, setRetrying] = useState(false);
  const polling = useRef(null), action = useRef(null);
  useEffect(() => {
    if (!isNative() || !config.embeddedReady) return undefined;
    const watcher = startStatusPolling({ getStatus: options => api.localStatus(options),
      enabled: () => isNative() && config.embeddedReady && config.kind !== 'pc', document, window,
      onStatus: next => { setStatus(next); setKind(config.kind); setError(''); },
      onError: () => setError(tr('Não foi possível consultar o host Android.')),
      onConnection: (type, detail) => {
        if (type === 'mc:connection-change') setKind(detail.kind || config.kind);
        else setHealth(detail.code === 'SYNCED' ? null : detail);
      } });
    polling.current = watcher;
    return () => { watcher.stop(); action.current?.abort(); polling.current = null; };
  }, []);
  async function retry() {
    if (retrying || config.kind !== 'local') return;
    const ac = action.current = new AbortController();
    setRetrying(true);
    try {
      await api.localActivate({ signal: ac.signal });
      if (!ac.signal.aborted) polling.current?.refresh();
    } catch {
      if (!ac.signal.aborted) setError(tr('Não foi possível ativar os modelos. Os dados foram preservados.'));
    } finally { if (!ac.signal.aborted) setRetrying(false); }
  }
  if (!isNative() || !config.embeddedReady || kind === 'pc' || config.kind === 'pc') return null;
  const busy = health?.code === 'BUSY', conflict = health?.code === 'CONFLICT';
  const syncLabel = health?.code === 'SYNCING' ? 'Sincronizando com o PC…' : busy ? 'BUSY · aguardando operação terminar' : conflict ? 'Conflito de sincronização · dados preservados' : null;
  const downloadError = ['chat', 'embed'].some(k => downloadProgress(status?.[k]).phase === 'error');
  const standby = kind === 'pc' && status?.activated === false;
  const resident = status?.chat?.running || status?.embed?.running;
  return <aside aria-label={tr('Status do AutoHost')} className="mc-col px-3 pb-1 text-[11px] leading-snug"
    onPointerDown={e => e.stopPropagation()} onTouchStart={e => e.stopPropagation()}>
    <div className="max-h-36 overflow-y-auto rounded-2xl border border-white/10 bg-cardDark px-3 py-2">
      <div className="flex items-center justify-between gap-2 font-semibold">
        <span>{kind === 'pc' ? tr('Host ativo: PC') : kind === 'local' ? tr('Host ativo: Android') : tr('Verificando host ativo…')}</span>
        {kind === 'local' && (downloadError || !status?.activated) && <button type="button" disabled={retrying} onClick={retry} className="rounded-full bg-composer px-3 py-1 font-medium disabled:opacity-40">{tr(!status?.activated ? 'Ativar host' : downloadError ? 'Tentar ativar host' : 'Reconectar')}</button>}
      </div>
      {syncLabel && <p role="status" className={conflict ? 'text-yellow-300' : 'text-textMuted'}>{tr(syncLabel)}</p>}
      {health?.message && health.message !== syncLabel && <p className="text-yellow-300">{tr(health.message)}</p>}
      <PcConnectionButton />
      {standby && <p className="text-textMuted">{tr(resident ? 'Android em standby · descarregando modelos' : 'Android em standby · nenhum modelo residente')}</p>}
      {status && (['downloading', 'verifying'].includes(downloadProgress(status.chat).phase) || ['downloading', 'verifying'].includes(downloadProgress(status.embed).phase)) && (
        <div className="mt-1 space-y-1 text-textMuted">
          {['downloading', 'verifying'].includes(downloadProgress(status.chat).phase) && (
            <DownloadStatus model={status.chat} title={tr('Modelo de conversa')} />
          )}
          {['downloading', 'verifying'].includes(downloadProgress(status.embed).phase) && (
            <DownloadStatus model={status.embed} title={tr('Biblioteca de embeddings')} />
          )}
        </div>
      )}
      {downloadError && <p className="mt-1 text-textMuted">{tr('A ativação não reinicia um download já ativo. Se o erro persistir, confira o runtime Android.')}</p>}
      {error && <p role="status" className="mt-1 text-red-300">{error}</p>}
    </div>
  </aside>;
}
