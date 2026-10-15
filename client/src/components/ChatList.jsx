import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage } from '../i18n.js';
import React, { memo, useEffect, useRef, useState } from 'react';
import { Plus, Settings, RefreshCw, WifiOff, Pin, Archive, ArchiveRestore, Copy, Trash2, Download, Pencil } from 'lucide-react';
import BrainIcon from './BrainIcon.jsx';
import { api } from '../api.js';
import { Avatar, timeAgo, useLongPress } from '../ui.jsx';
import { plainText } from '../format.js';
import { Sheet, SheetButton } from './Sheet.jsx';
import AvatarViewerModal from './AvatarViewerModal.jsx';
import RenameModal from './RenameModal.jsx';
import HomeDrawer from './HomeDrawer.jsx';
import { useDrawerGesture } from '../gestures.js';
import { resetPull } from '../drawer-motion.js';
import { useLayer } from '../nav.js';

function preview(text) {
  const t = plainText(text || '').replace(/\s+/g, ' ');
  return t || tr('Sem mensagens ainda.');
}

const PULL_TRIGGER = 96; // finger travel (px) that triggers the refresh when released

export default function ChatList({ hostStatus, chats, offline, status, onRefresh, error, archivedView, switchingArchived, query, onQuery, onToggleArchived, onOpen, onCreate, onChanged, fontSize, onFontSize, onResetServer }) {
  const confirm = useConfirm();
  useLanguage();
  const [sheet, setSheet] = useState(null);
  const [toast, setToast] = useState('');
  const [viewerAvatar, setViewerAvatar] = useState(null);
  const [renameChat, setRenameChat] = useState(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const scrollRef = useRef(null);
  const pullRef = useRef(null);
  const [spinning, setSpinning] = useState(false);
  const spinningRef = useRef(false);
  const refreshNow = useRef(onRefresh);
  refreshNow.current = onRefresh;

  const openDrawer = () => { resetPull(); setDrawerOpen(true); };
  useLayer(drawerOpen, () => setDrawerOpen(false));
  const drawerGesture = useDrawerGesture(openDrawer, !drawerOpen && !sheet && !viewerAvatar && !renameChat, { edgePx: 48, edgeOnly: false });

  async function handleCreate() {
    if (!await confirm(tr("Deseja criar uma nova conversa?"))) return;
    onCreate();
  }

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
  const statusLabel = status === 'online' ? tr('Conectado ao servidor ativo') : status === 'offline' ? tr('Sem conexão com o servidor ativo') : tr('Verificando conexão');

  const flash = (t) => {
    setToast(t);
    setTimeout(() => setToast(''), 2200);
  };

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
    <div {...drawerGesture} className="mc-list fixed inset-0 z-30 flex flex-col bg-bgDark">
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
              onClick={onToggleArchived}
              disabled={switchingArchived}
              aria-busy={switchingArchived}
              className="mc-glass-pill mc-bubble pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg"
              aria-label={tr("Arquivadas")}
            >
              {archivedView ? <ArchiveRestore size={19} /> : <Archive size={19} />}
            </button>
            <button
              onClick={openDrawer}
              className="mc-glass-pill mc-bubble pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg"
              aria-label={tr("Configurações")}
            >
              <Settings size={19} />
            </button>
            <button
              onClick={handleCreate}
              className="mc-bubble mc-glass-accent pointer-events-auto flex h-10 w-10 items-center justify-center rounded-full shadow-lg"
              aria-label={tr("Novo chat")}
            >
              <Plus size={20} />
            </button>
          </div>
        </div>
      </header>

      <div
        ref={pullRef}
        aria-hidden="true"
        className={`mc-pull mc-glass-pill pointer-events-none fixed left-1/2 z-20 flex h-9 w-9 items-center justify-center rounded-full shadow-lg ${spinning ? 'mc-pull-spinning' : ''}`}
        style={{ top: 'calc(var(--sat) + 64px)' }}
      >
        <RefreshCw size={18} className={spinning ? 'animate-spin' : ''} />
      </div>

      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto pb-6"
        style={{
          paddingTop: 'calc(var(--sat) + 68px)',
        }}
      >
        <div className="mc-col">
          {hostStatus && <div className="shrink-0 mb-3">{hostStatus}</div>}
          {offline && (
            <div className="px-4 pb-2">
              <div className="flex items-center gap-2 rounded-xl bg-yellow-500/10 px-3 py-2 text-xs text-yellow-300">
                <WifiOff size={14} />{tr("Servidor ativo fora do alcance. Mostrando a cópia salva no aparelho (leitura e rascunhos).")}</div>
            </div>
          )}
          {error && !offline && <div className="px-4 pb-2 text-xs text-red-300">{tr(error)}</div>}
          {chats.length === 0 && (
            <div className="px-8 pt-24 text-center text-textMuted">
              <p className="mb-4">{archivedView ? tr("Nada arquivado.") : tr("Nenhuma conversa ainda.")}</p>
              {!archivedView && (
                <button onClick={handleCreate} className="mc-bubble mc-glass-accent pointer-events-auto rounded-full px-6 py-3 font-semibold text-white shadow-lg">{tr("\n                  Criar primeiro chat\n                ")}</button>
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

      {drawerOpen && (
        <HomeDrawer
          hostStatus={hostStatus}
          fontSize={fontSize}
          onFontSize={onFontSize}
          onClose={() => setDrawerOpen(false)}
          onCreate={onCreate}
          onChanged={onChanged}
          onResetServer={onResetServer}
        />
      )}
    </div>
  );
}

// Memoized: the list only re-renders conversations whose content changed (see App.jsx, which reuses equal objects).
const ChatRow = memo(function ChatRow({ chat, onOpen, onMenu, onViewAvatar }) {
  useLanguage();
  const press = useLongPress(() => onMenu(chat), 550);
  return (
    <button {...press} className="mc-list-row flex w-full items-center gap-3 px-3 py-2 text-left active:bg-cardDarkHover" onClick={(e) => onOpen(chat.id, e.currentTarget)}>
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
              <BrainIcon size={11} />{tr(" memória pendente (")}{chat.pendingJobs})
            </span>
          )}
        </div>
      </div>
    </button>
  );
});
