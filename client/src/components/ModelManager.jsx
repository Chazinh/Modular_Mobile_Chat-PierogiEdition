import React, { useEffect, useRef, useState } from 'react';
import {
  Download, Trash2, Search, RotateCw, Check, Globe, Cpu, Layers,
  HardDrive, ExternalLink, Sparkles, X, AlertCircle
} from 'lucide-react';
import { api, config, isNative } from '../api.js';
import { tr, useLanguage } from '../i18n.js';
import { useConfirm } from './ConfirmProvider.jsx';
import { downloadProgress } from '../autohost-status.js';
import { CURATED_MODELS, fetchHuggingFaceModels, formatBytes } from '../huggingface.js';

function MeterBar({ label, value, max = 5 }) {
  const pct = Math.min(100, Math.max(0, (value / max) * 100));
  return (
    <div className="flex items-center justify-end gap-2 text-[11px] text-textMuted">
      <span className="w-14 text-right">{label}</span>
      <div className="h-1.5 w-16 overflow-hidden rounded-full bg-white/10">
        <div
          className="h-full rounded-full bg-gradient-to-r from-[#ec4899] to-[#f472b6]"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

export default function ModelManager({ onClose }) {
  useLanguage();
  const confirm = useConfirm();

  const [query, setQuery] = useState('');
  const [filterType, setFilterType] = useState('all'); // all | chat | embed
  const [models, setModels] = useState(CURATED_MODELS);
  const [loadingHf, setLoadingHf] = useState(false);
  const [localStatus, setLocalStatus] = useState(null);
  const [busyAction, setBusyAction] = useState('');
  const [message, setMessage] = useState('');

  const pollingRef = useRef(null);

  // Poll local status while open
  useEffect(() => {
    let active = true;

    async function loadStatus() {
      try {
        const s = await api.localStatus();
        if (active) setLocalStatus(s);
      } catch {
        // PC mode or offline
      }
    }

    loadStatus();
    pollingRef.current = setInterval(loadStatus, 2500);

    return () => {
      active = false;
      if (pollingRef.current) clearInterval(pollingRef.current);
    };
  }, []);

  // Fetch / search Hugging Face models
  useEffect(() => {
    let cancel = false;
    const ac = new AbortController();

    async function search() {
      setLoadingHf(true);
      try {
        const results = await fetchHuggingFaceModels(query, { signal: ac.signal });
        if (!cancel) setModels(results);
      } catch {
        // Handled gracefully in fetchHuggingFaceModels
      } finally {
        if (!cancel) setLoadingHf(false);
      }
    }

    const timer = setTimeout(search, query ? 350 : 0);
    return () => {
      cancel = true;
      ac.abort();
      clearTimeout(timer);
    };
  }, [query]);

  async function handleRefresh() {
    setLoadingHf(true);
    try {
      const results = await fetchHuggingFaceModels(query);
      setModels(results);
      const s = await api.localStatus().catch(() => null);
      if (s) setLocalStatus(s);
    } finally {
      setLoadingHf(false);
    }
  }

  async function handleDownload(modelKind) {
    setBusyAction(`download-${modelKind}`);
    setMessage('');
    try {
      await api.downloadLocalModel(modelKind);
      setMessage(tr('Download iniciado em segundo plano.'));
      const s = await api.localStatus().catch(() => null);
      if (s) setLocalStatus(s);
    } catch (e) {
      setMessage(tr('Não foi possível iniciar o download: {0}', [e.message || e]));
    } finally {
      setBusyAction('');
    }
  }

  async function handleDelete(modelKind, modelName) {
    if (!await confirm(tr('Deseja excluir o modelo {0}? O arquivo será removido para liberar espaço em disco.', [modelName]))) {
      return;
    }
    setBusyAction(`delete-${modelKind}`);
    setMessage('');
    try {
      await api.deleteLocalModel(modelKind);
      setMessage(tr('Modelo excluído com sucesso.'));
      const s = await api.localStatus().catch(() => null);
      if (s) setLocalStatus(s);
    } catch (e) {
      setMessage(tr('Não foi possível excluir o modelo: {0}', [e.message || e]));
    } finally {
      setBusyAction('');
    }
  }

  // Determine downloaded / downloading status from localStatus
  const chatProgress = downloadProgress(localStatus?.chat);
  const embedProgress = downloadProgress(localStatus?.embed);

  const chatDownloaded = localStatus?.chat?.ok || chatProgress.phase === 'ready';
  const embedDownloaded = localStatus?.embed?.ok || embedProgress.phase === 'ready';

  const chatDownloading = ['downloading', 'verifying', 'pending'].includes(chatProgress.phase) && !chatDownloaded;
  const embedDownloading = ['downloading', 'verifying', 'pending'].includes(embedProgress.phase) && !embedDownloaded;

  // Build the list of downloaded items
  const downloadedList = [];
  if (chatDownloaded || chatDownloading) {
    downloadedList.push({
      kind: 'chat',
      name: 'Qwen 2.5 1.5B Instruct',
      badge: chatDownloaded ? tr('Ativo') : tr('Baixando'),
      active: chatDownloaded,
      description: tr('Modelo oficial de conversa e extração contextual para celular.'),
      languages: tr('28 idiomas'),
      tag: tr('Streaming'),
      size: '1,1 GB',
      precision: 4,
      speed: 4,
      progress: chatDownloading ? chatProgress : null,
    });
  }
  if (embedDownloaded || embedDownloading) {
    downloadedList.push({
      kind: 'embed',
      name: 'EmbeddingGemma 300M',
      badge: embedDownloaded ? tr('Ativo') : tr('Baixando'),
      active: embedDownloaded,
      description: tr('Modelo de embeddings vetoriais para memória de longo prazo no aparelho.'),
      languages: tr('Multilíngue'),
      tag: tr('Embeddings'),
      size: '334 MB',
      precision: 5,
      speed: 5,
      progress: embedDownloading ? embedProgress : null,
    });
  }

  // Filter available models
  const availableModels = models.filter((m) => {
    if (filterType !== 'all' && m.type !== filterType) return false;
    // Hide if already fully downloaded
    if (m.isOfficialChat && chatDownloaded) return false;
    if (m.isOfficialEmbed && embedDownloaded) return false;
    return true;
  });

  return (
    <div className="mc-bubble mx-[10px] space-y-4 rounded-[26px] p-4 text-sm">
      {/* Header Info */}
      <div className="space-y-1">
        <p className="text-xs text-textMuted">
          {tr('Selecione um modelo de IA ou baixe modelos adicionais do Hugging Face. O download é opcional: diferentes modelos oferecem diferentes níveis de precisão, velocidade e consumo de memória.')}
        </p>
      </div>

      {/* Optional Download Note */}
      <div className="flex items-start gap-2.5 rounded-2xl bg-composer/70 p-3 text-xs text-textMuted">
        <Sparkles size={16} className="mt-0.5 shrink-0 text-[#f472b6]" />
        <div>
          <span className="font-semibold text-white/90">{tr('Download opcional')}</span>:{' '}
          {tr('O aplicativo funciona normalmente com modelos em nuvem (DeepSeek, 9Router, OpenRouter). Baixar modelos permite executar conversas e busca de memória localmente sem depender de internet.')}
        </div>
      </div>

      {/* Search Input matching screenshot */}
      <div className="relative">
        <Search size={16} className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-textMuted" />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={tr('Buscar modelos por nome...')}
          className="w-full rounded-2xl bg-composer py-2.5 pl-10 pr-10 text-sm outline-none placeholder:text-textMuted/70 focus:ring-1 focus:ring-[#f472b6]/60"
        />
        {query && (
          <button
            type="button"
            onClick={() => setQuery('')}
            className="absolute right-3 top-1/2 -translate-y-1/2 p-1 text-textMuted hover:text-white"
          >
            <X size={15} />
          </button>
        )}
      </div>

      {/* Filter and Refresh Bar */}
      <div className="flex flex-wrap items-center justify-between gap-2 pt-1 text-xs">
        <div className="flex items-center gap-1.5 overflow-x-auto pb-1">
          {[
            { id: 'all', label: tr('Todos') },
            { id: 'chat', label: tr('Conversa') },
            { id: 'embed', label: tr('Memória') },
          ].map((tab) => (
            <button
              key={tab.id}
              type="button"
              onClick={() => setFilterType(tab.id)}
              className={`rounded-full px-3 py-1 font-medium transition-colors ${
                filterType === tab.id
                  ? 'bg-[#ec4899]/20 text-[#f472b6] border border-[#ec4899]/40'
                  : 'bg-composer text-textMuted hover:text-white'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>

        <button
          type="button"
          disabled={loadingHf}
          onClick={handleRefresh}
          className="flex items-center gap-1.5 rounded-full bg-composer px-3 py-1 text-textMuted hover:text-white disabled:opacity-50"
          title={tr('Recarregar do Hugging Face')}
        >
          <RotateCw size={13} className={loadingHf ? 'animate-spin text-[#f472b6]' : ''} />
          <span>{tr('Hugging Face')}</span>
        </button>
      </div>

      {message && (
        <div className="flex items-center gap-2 rounded-xl bg-cardDark p-2.5 text-xs text-[#f472b6]">
          <AlertCircle size={15} className="shrink-0" />
          <span>{message}</span>
        </div>
      )}

      {/* SECTION: Modelos baixados */}
      <div className="space-y-2.5 pt-2">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold tracking-wide text-white/90">
            {tr('Modelos baixados')}
          </h3>
          <span className="text-xs text-textMuted">
            {downloadedList.filter((m) => m.active).length} {tr('ativo(s)')}
          </span>
        </div>

        {downloadedList.length === 0 ? (
          <div className="rounded-2xl border border-white/5 bg-composer/40 p-3.5 text-center text-xs text-textMuted">
            {tr('Nenhum modelo baixado no aparelho. O download é opcional; o app funciona normalmente com APIs em nuvem.')}
          </div>
        ) : (
          downloadedList.map((item) => (
            <div
              key={item.kind}
              className={`space-y-2.5 rounded-2xl border p-3.5 transition-all ${
                item.active
                  ? 'border-[#ec4899]/40 bg-cardDark/90 shadow-[0_4px_16px_rgba(236,72,153,0.08)]'
                  : 'border-white/10 bg-cardDark/60'
              }`}
            >
              {/* Title, Badge and Meters */}
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-semibold text-white/95">{item.name}</span>
                    <span className="inline-flex items-center gap-1 rounded-full border border-[#ec4899]/40 bg-[#ec4899]/20 px-2 py-0.5 text-[11px] font-semibold text-[#f472b6]">
                      <Check size={11} />
                      {item.badge}
                    </span>
                  </div>
                  <p className="mt-1 text-xs text-textMuted line-clamp-2">{item.description}</p>
                </div>

                {/* Precision and Speed meters */}
                <div className="shrink-0 space-y-1">
                  <MeterBar label={tr('precisão')} value={item.precision} />
                  <MeterBar label={tr('velocidade')} value={item.speed} />
                </div>
              </div>

              {/* Progress bar if downloading */}
              {item.progress && (
                <div className="space-y-1 rounded-xl bg-composer/80 p-2.5 text-xs">
                  <div className="flex justify-between text-[11px] text-textMuted">
                    <span>{tr('Baixando')} · {item.progress.phase}</span>
                    <span className="tabular-nums">
                      {formatBytes(item.progress.downloaded)} / {formatBytes(item.progress.total)}
                      {item.progress.percent != null && ` · ${item.progress.percent}%`}
                    </span>
                  </div>
                  <progress
                    value={item.progress.downloaded}
                    max={item.progress.total || 1}
                    className="h-1 w-full overflow-hidden rounded-full accent-[#ec4899]"
                  />
                  {item.progress.error && (
                    <p className="text-red-300">{item.progress.error}</p>
                  )}
                </div>
              )}

              {/* Bottom Row info matching screenshot */}
              <div className="flex flex-wrap items-center justify-between gap-2 pt-1 border-t border-white/5 text-xs text-textMuted">
                <div className="flex items-center gap-3">
                  <span className="flex items-center gap-1">
                    <Globe size={13} className="text-textMuted" />
                    <span>{item.languages}</span>
                  </span>
                  <span className="flex items-center gap-1">
                    <Layers size={13} className="text-textMuted" />
                    <span>{item.tag}</span>
                  </span>
                  <span className="flex items-center gap-1">
                    <HardDrive size={13} className="text-textMuted" />
                    <span>{item.size}</span>
                  </span>
                </div>

                <button
                  type="button"
                  disabled={busyAction === `delete-${item.kind}`}
                  onClick={() => handleDelete(item.kind, item.name)}
                  className="flex items-center gap-1 text-red-400 hover:text-red-300 active:opacity-75 disabled:opacity-50"
                  title={tr('Excluir modelo')}
                >
                  <Trash2 size={13} />
                  <span>{tr('Excluir')}</span>
                </button>
              </div>
            </div>
          ))
        )}
      </div>

      {/* SECTION: Disponíveis para download */}
      <div className="space-y-2.5 pt-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold tracking-wide text-white/90">
            {tr('Disponíveis para download')}
          </h3>
          <span className="text-xs text-textMuted">
            {availableModels.length} {tr('opções')}
          </span>
        </div>

        {availableModels.map((model) => {
          const isOfficial = model.isOfficialChat || model.isOfficialEmbed;
          const targetKind = model.isOfficialChat ? 'chat' : model.isOfficialEmbed ? 'embed' : null;
          const isBusy = busyAction === `download-${targetKind}`;

          return (
            <div
              key={model.id}
              className="space-y-2.5 rounded-2xl border border-white/10 bg-cardDark/70 p-3.5 transition-all hover:border-white/20"
            >
              {/* Model Header & Badges */}
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-semibold text-white/95">{model.name}</span>
                    <span className="rounded-full border border-[#ec4899]/30 bg-[#ec4899]/15 px-2 py-0.5 text-[11px] font-medium text-[#f472b6]">
                      {tr(model.badge)}
                    </span>
                  </div>
                  <p className="mt-1 text-xs text-textMuted line-clamp-2">{model.description}</p>
                </div>

                {/* Precision and Speed meters */}
                <div className="shrink-0 space-y-1">
                  <MeterBar label={tr('precisão')} value={model.precision} />
                  <MeterBar label={tr('velocidade')} value={model.speed} />
                </div>
              </div>

              {/* Bottom Specs & Action */}
              <div className="flex flex-wrap items-center justify-between gap-2 pt-1 border-t border-white/5 text-xs text-textMuted">
                <div className="flex flex-wrap items-center gap-3">
                  <span className="flex items-center gap-1">
                    <Globe size={13} className="text-textMuted" />
                    <span>{model.languages}</span>
                  </span>
                  <span className="flex items-center gap-1">
                    <Layers size={13} className="text-textMuted" />
                    <span>{model.format}</span>
                  </span>
                  {model.downloads && (
                    <span className="text-[11px] text-textMuted/70">
                      ↓ {model.downloads}
                    </span>
                  )}
                </div>

                {/* Action button */}
                {isOfficial ? (
                  <button
                    type="button"
                    disabled={isBusy}
                    onClick={() => handleDownload(targetKind)}
                    className="flex items-center gap-1.5 rounded-full bg-[#ec4899]/20 px-3 py-1.5 font-medium text-[#f472b6] border border-[#ec4899]/40 transition-colors hover:bg-[#ec4899]/30 active:scale-95 disabled:opacity-50"
                  >
                    <Download size={13} className={isBusy ? 'animate-bounce' : ''} />
                    <span>{model.sizeFormatted}</span>
                  </button>
                ) : (
                  <a
                    href={model.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="flex items-center gap-1.5 rounded-full bg-composer px-3 py-1 text-textMuted hover:text-white"
                  >
                    <span>{model.sizeFormatted}</span>
                    <ExternalLink size={12} />
                  </a>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
