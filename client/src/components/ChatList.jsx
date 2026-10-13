import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage } from '../i18n.js';
import React, { memo, useEffect, useRef, useState } from 'react';
import { Plus, Search, Settings, RefreshCw, WifiOff, Pin, Archive, ArchiveRestore, Copy, Trash2, Download, X, Brain, Pencil } from 'lucide-react';
import { api } from '../api.js';
import { Avatar, timeAgo, useLongPress } from '../ui.jsx';
import { plainText } from '../format.js';
import { Sheet, SheetButton } from './Sheet.jsx';
import AvatarViewerModal from './AvatarViewerModal.jsx';
import RenameModal from './RenameModal.jsx';

function preview(text) {
  const t = plainText(text || '').replace(/\s+/g, ' ');
  return t || tr('Sem mensagens ainda.');
}

const PULL_TRIGGER = 96; // finger travel (px) that triggers the refresh when released

export default function ChatList({ chats, offline, status, onRefresh, error, archivedView, query, onQuery, onToggleArchived, onOpen, onCreate, onChanged, onGlobal }) {
  const confirm = useConfirm();
  useLanguage();
  const [searching, setSearching] = useState(false);
  const [sheet, setSheet] = useState(null);
  const [hits, setHits] = useState(null);
  const [toast, setToast] = useState('');
  const [viewerAvatar, setViewerAvatar] = useState(null);
  const [renameChat, setRenameChat] = useState(null);
  const [text, setText] = useState(query); // the field responds immediately; server search/filtering waits for a pause in typing
  const searchTimer = useRef(0);
  const searchSeq = useRef(0);
  useEffect(() => () => clearTimeout(searchTimer.current), []);
  const scrollRef = useRef(null);
  const pullRef = useRef(null);
  const [spinning, setSpinning] = useState(false);
  const spinningRef = useRef(false);
  const refreshNow = useRef(onRefresh);
  refreshNow.current = onRefresh;

  // Pull to refresh. Touch listeners are passive (native scrolling and the elastic stretch stay intact); the
  // indicator is styled directly so the gesture never re-renders the list.
  useEffect(() => {
    const el = scrollRef.current;
    const ind = pullRef.current;
    if (!el || !ind) return undefined;
    let startY = null;
    let dist = 0;
    const paint = (p) => {
      const k = Math.min(p, 1);
      ind.style.opacity = p > 0 ? String(k) : '';
      ind.style.transform = p > 0 ? `translate(-50%, ${(k * 18 - 10).toFixed(1)}px) scale(${(0.6 + 0.4 * k).toFixed(3)})` : '';
      ind.firstChild.style.transform = p > 0 ? `rotate(${Math.round(p * 300)}deg)` : '';
      ind.classList.toggle('mc-pull-ready', p >= 1);
      ind.classList.toggle('mc-pull-drag', p > 0);
    };
    const start = (e) => {
      startY = e.touches.length === 1 && el.scrollTop <= 0 && !spinningRef.current ? e.touches[0].clientY : null;
      dist = 0;
    };
    const move = (e) => {
      if (startY === null) return;
      const d = e.touches[0].clientY - startY;
      dist = el.scrollTop <= 0 && d > 0 ? d : 0;
      paint(dist / PULL_TRIGGER);
    };
    const finish = (trigger) => () => {
      if (startY === null) return;
      startY = null;
      const go = trigger && dist >= PULL_TRIGGER;
      dist = 0;
      paint(0);
      if (!go) return;
      spinningRef.current = true;
      setSpinning(true);
      Promise.resolve(refreshNow.current?.()).finally(() => {
        spinningRef.current = false;
        setSpinning(false);
      });
    };
    const end = finish(true);
    const cancel = finish(false);
    el.addEventListener('touchstart', start, { passive: true });
    el.addEventListener('touchmove', move, { passive: true });
    el.addEventListener('touchend', end, { passive: true });
    el.addEventListener('touchcancel', cancel, { passive: true });
    return () => {
      el.removeEventListener('touchstart', start);
      el.removeEventListener('touchmove', move);
      el.removeEventListener('touchend', end);
      el.removeEventListener('touchcancel', cancel);
    };
  }, []);
  const statusLabel = status === 'online' ? tr('Conectado ao PC') : status === 'offline' ? tr('Sem conexão com o PC') : tr('Verificando conexão');

  const flash = (t) => {
    setToast(t);
    setTimeout(() => setToast(''), 2200);
  };

  async function textSearch(q) {
    onQuery(q);
    const seq = ++searchSeq.current;
    if (q.trim().length < 3) return setHits(null);
    try {
      const results = (await api.search(q)).results;
      if (seq === searchSeq.current) setHits(results);
    } catch {
      if (seq === searchSeq.current) setHits(null);
    }
  }
  function typeQuery(q) {
    setText(q);
    clearTimeout(searchTimer.current);
    if (!q) textSearch('');
    else searchTimer.current = setTimeout(() => textSearch(q), 280);
  }

  async function act(fn, ok) {
    setSheet(null);
    try {
      await fn();
      if (ok) flash(ok);
      onChanged();
    } catch (e) {
      flash(e.message);
    }
  }

  async function exportChat(chat) {
    setSheet(null);
    try {
      const data = await api.exportAll(chat.id);
      const blob = new Blob([JSON.stringify(data)], { type: 'application/json' });
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = `${chat.name.replace(/[^\w-]+/g, '_')}.json`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 5000);
    } catch (e) {
      flash(e.message);
    }
  }

  return (
    <div className="mc-list fixed inset-0 z-30 flex flex-col bg-bgDark">
      {/* Detached floating header with individual liquid-glass bubbles */}
      <header className="pointer-events-none fixed inset-x-0 top-0 z-30 px-3" style={{ paddingTop: 'calc(var(--sat) + 8px)' }}>
        <div className="mc-col flex items-center justify-between gap-2">
          {/* Title isolated in its own bubble */}
          <div className="mc-glass-pill pointer-events-auto flex items-center gap-2.5 rounded-full px-4 py-2 shadow-lg">
            <h1 className="text-[19px] font-extrabold tracking-tight">{archivedView ? tr("Arquivadas") : tr("Conversas")}</h1>
            <span
              role="status"
              title={statusLabel}
              aria-label={statusLabel}
              className={`mc-bar-dot shrink-0 ${status === 'online' ? 'mc-on' : ''} ${status === 'checking' || spinning ? 'mc-dot-checking' : ''}`}
            />
          </div>

          {/* Action icons in individual round bubbles */}
          <div className="flex items-center gap-2">
            <button
              onClick={() => setSearching((v) => !v)}
              className="mc-glass-pill pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
              aria-label={tr("Buscar")}
            >
              <Search size={19} />
            </button>
            <button
              onClick={onToggleArchived}
              className="mc-glass-pill pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
              aria-label={tr("Arquivadas")}
            >
              {archivedView ? <ArchiveRestore size={19} /> : <Archive size={19} />}
            </button>
            <button
              onClick={onGlobal}
              className="mc-glass-pill pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
              aria-label={tr("Configurações")}
            >
              <Settings size={19} />
            </button>
            <button
              onClick={onCreate}
              className="mc-glass-pill mc-glass-accent pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
              aria-label={tr("Novo chat")}
            >
              <Plus size={20} />
            </button>
          </div>
        </div>

        {searching && (
          <div className="mc-col mt-2">
            <div className="mc-glass-pill pointer-events-auto flex items-center gap-2 rounded-full px-4 py-2 shadow-lg">
              <Search size={16} className="text-textMuted" />
              <input
                autoFocus
                value={text}
                onChange={(e) => typeQuery(e.target.value)}
                placeholder={tr("Buscar conversas e mensagens")}
                spellCheck={true}

                autoCorrect="on"
                autoCapitalize="sentences"
                className="flex-1 bg-transparent text-[16px] outline-none placeholder:text-textMuted"
              />
              {text && (
                <button onClick={() => typeQuery('')} aria-label={tr("Limpar")}>
                  <X size={16} />
                </button>
              )}
            </div>
          </div>
        )}
      </header>

      <div
        ref={pullRef}
        aria-hidden="true"
        className={`mc-pull mc-glass-pill pointer-events-none fixed left-1/2 z-20 flex h-9 w-9 items-center justify-center rounded-full shadow-lg ${spinning ? 'mc-pull-spinning' : ''}`}
        style={{ top: searching ? 'calc(var(--sat) + 112px)' : 'calc(var(--sat) + 64px)' }}
      >
        <RefreshCw size={18} className={spinning ? 'animate-spin' : ''} />
      </div>

      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto pb-6"
        style={{
          paddingTop: searching ? 'calc(var(--sat) + 116px)' : 'calc(var(--sat) + 68px)',
        }}
      >
        <div className="mc-col">
          {offline && (
            <div className="px-4 pb-2">
              <div className="flex items-center gap-2 rounded-xl bg-yellow-500/10 px-3 py-2 text-xs text-yellow-300">
                <WifiOff size={14} />{tr(" PC fora do alcance. Mostrando a cópia salva no aparelho (leitura e rascunhos).\n              ")}</div>
            </div>
          )}
          {error && !offline && <div className="px-4 pb-2 text-xs text-red-300">{tr(error)}</div>}
          {hits && hits.length > 0 && (
            <div className="px-4 pb-2 pt-3">
              <p className="mb-1 text-xs font-semibold uppercase tracking-wide text-textMuted">{tr("Mensagens encontradas")}</p>
              {hits.map((h, i) => (
                <button key={i} onClick={() => onOpen(h.convId)} className="mb-1 block w-full rounded-xl bg-cardDark px-3 py-2 text-left active:bg-cardDarkHover">
                  <span className="text-xs text-textMuted">{h.convName}</span>
                  <span className="block text-sm">{h.snippet.replace(/[«»]/g, '')}</span>
                </button>
              ))}
            </div>
          )}
          {chats.length === 0 && (
            <div className="px-8 pt-24 text-center text-textMuted">
              <p className="mb-4">{archivedView ? tr("Nada arquivado.") : tr("Nenhuma conversa ainda.")}</p>
              {!archivedView && (
                <button onClick={onCreate} className="rounded-full bg-accentPurple px-6 py-3 font-semibold text-white">{tr("\n                  Criar primeiro chat\n                ")}</button>
              )}
            </div>
          )}
          {chats.map((c) => (
            <ChatRow key={c.id} chat={c} onOpen={onOpen} onMenu={setSheet} onViewAvatar={setViewerAvatar} />
          ))}
        </div>
      </div>

      {sheet && (
        <Sheet onClose={() => setSheet(null)} title={sheet.name}>
          <SheetButton icon={<Pin size={20} />} label={sheet.pinned ? tr("Desafixar") : tr("Fixar no topo")} onClick={() => act(() => api.updateChat(sheet.id, { pinned: !sheet.pinned }))} />
          <SheetButton
            icon={<Pencil size={20} />}
            label={tr("Renomear conversa")}
            onClick={() => {
              setRenameChat(sheet);
              setSheet(null);
            }}
          />
          <SheetButton
            icon={sheet.archived ? <ArchiveRestore size={20} /> : <Archive size={20} />}
            label={sheet.archived ? tr("Desarquivar") : tr("Arquivar")}
            onClick={() => act(() => api.updateChat(sheet.id, { archived: !sheet.archived }))}
          />
          <SheetButton icon={<Copy size={20} />} label={tr("Duplicar personagem (conversa limpa)")} onClick={() => act(() => api.duplicateChat(sheet.id), tr("Conversa duplicada"))} />
          <SheetButton icon={<Download size={20} />} label={tr("Exportar conversa")} onClick={() => exportChat(sheet)} />
          <SheetButton
            icon={<Trash2 size={20} />}
            danger
            label={tr("Excluir (recuperável por 30 dias)")}
            onClick={async () => {
              if (await confirm(tr("Excluir \"{0}\"? Ela fica na lixeira por 30 dias.", [sheet.name]))) act(() => api.deleteChat(sheet.id), tr("Movida para a lixeira"));
              else setSheet(null);
            }}
          />
        </Sheet>
      )}

      {viewerAvatar && (
        <AvatarViewerModal item={viewerAvatar} onClose={() => setViewerAvatar(null)} />
      )}

      {renameChat && (
        <RenameModal
          title={tr("Renomear \"{0}\"", [renameChat.name])}
          initial={renameChat.name}
          onClose={() => setRenameChat(null)}
          onSave={async (newName) => {
            await act(() => api.updateChat(renameChat.id, { name: newName }), tr("Conversa renomeada"));
          }}
        />
      )}

      {toast && (
        <div className="pointer-events-none fixed inset-x-0 bottom-8 z-[60] flex justify-center">
          <span className="rounded-full bg-white/90 px-4 py-2 text-sm font-medium text-black">{tr(toast)}</span>
        </div>
      )}
    </div>
  );
}

// Memoized: the list only re-renders conversations whose content changed (see App.jsx, which reuses equal objects).
const ChatRow = memo(function ChatRow({ chat, onOpen, onMenu, onViewAvatar }) {
  useLanguage();
  const press = useLongPress(() => onMenu(chat), 550);
  return (
    <button {...press} className="mc-list-row flex w-full items-center gap-3 px-4 py-3 text-left active:bg-cardDarkHover" onClick={() => onOpen(chat.id)}>
      <span
        role="button"
        tabIndex={0}
        onClick={(e) => {
          e.stopPropagation();
          onViewAvatar({ name: chat.name, avatar: chat.avatar });
        }}
        className="shrink-0 active:scale-95 transition-transform"
        aria-label={tr("Ver foto de {0}", [chat.name])}
      >
        <Avatar name={chat.name} avatar={chat.avatar} size={60} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2">
          <span className="flex min-w-0 items-center gap-1.5 text-[17px] font-bold">
            {chat.pinned && <Pin size={13} className="shrink-0 text-accentPurpleLight" />}
            <span className="truncate">{chat.name}</span>
          </span>
          <span className="shrink-0 text-sm text-textMuted">{timeAgo(chat.updatedAt)}</span>
        </div>
        <p className="mt-0.5 line-clamp-2 text-[14px] leading-snug text-gray-300">{preview(chat.lastMessage)}</p>
        <div className="mt-1 flex items-center gap-2 text-[11px] text-textMuted">
          {chat.generating && (
            <span className="flex items-center gap-1 text-accentPurpleLight">
              <span className="h-1.5 w-1.5 rounded-full bg-accentPurpleLight" />{tr(" respondendo\n            ")}</span>
          )}
          {chat.pendingJobs > 0 && (
            <span className="flex items-center gap-1">
              <Brain size={11} />{tr(" memória pendente (")}{chat.pendingJobs})
            </span>
          )}
        </div>
      </div>
    </button>
  );
});
