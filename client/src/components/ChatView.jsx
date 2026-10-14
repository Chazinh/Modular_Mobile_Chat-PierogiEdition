import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage } from '../i18n.js';
import React, { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react';
import {
  ChevronLeft, ChevronRight, MoreHorizontal, FastForward, RotateCcw, SendHorizontal, Square, Copy, Pencil, Trash2,
  ArrowDown, Keyboard, Layers, Info, WifiOff, Check, AlertTriangle,
} from 'lucide-react';
import { api, uuid } from '../api.js';
import { useLayer } from '../nav.js';
import { Avatar, SceneText, UserText, useLongPress, copyText, useEvent } from '../ui.jsx';
import { plainText } from '../format.js';
import { Sheet, SheetButton } from './Sheet.jsx';
import SettingsDrawer from './SettingsDrawer.jsx';
import CastScreen from './CastScreen.jsx';
import MemoryScreen from './MemoryScreen.jsx';
import BrainIcon from './BrainIcon.jsx';
import { flushStyleSettings } from '../style-settings.js';
import { useResume } from '../lifecycle.js';
import { useReadingGesture, useDrawerGesture, useTopBar } from '../gestures.js';
import AvatarViewerModal from './AvatarViewerModal.jsx';
import RenameModal from './RenameModal.jsx';
import { resetPull } from '../drawer-motion.js';

const STATUS_NOTE = {
  interrupted: 'Interrompida. O trecho recebido foi salvo.',
  length: 'Atingiu o limite de tamanho. Use Continuar para seguir.',
  error: 'A geração falhou depois de começar. O trecho recebido foi salvo.',
};

// Only a reply the provider really truncated (token limit) gets a notice; the server's own character ceiling
// already ends the text at its last complete sentence, so no warning is shown for it.
function lengthNotice(m) {
  const finish = m.usage?.finish;
  if (finish === 'length' || finish === 'max_tokens') return tr("O provedor encerrou a resposta pelo limite de tokens. Toque em Continuar para completá-la.");
  return null;
}

export const fmtTokens = (n) => (n == null ? '–' : n >= 10000 ? `${(n / 1000).toFixed(1)}k` : String(n));

const sameJson = (a, b) => a === b || JSON.stringify(a) === JSON.stringify(b);
const withoutHistory = (c) => ({ ...c, history: undefined });

// Reuses message objects that did not change (and the chat itself when nothing changed): messages are
// memoized by identity, so an identical reload does not re-render the history.
function mergeChat(prev, next) {
  if (!prev) return { chat: next, historyChanged: true };
  const old = new Map(prev.history.map((m) => [m.id, m]));
  let same = prev.history.length === next.history.length;
  const history = next.history.map((m, i) => {
    const p = old.get(m.id);
    const keep = p && sameJson(p, m) ? p : m;
    if (keep !== prev.history[i]) same = false;
    return keep;
  });
  if (same && sameJson(withoutHistory(prev), withoutHistory(next))) return { chat: prev, historyChanged: false };
  return { chat: { ...next, history: same ? prev.history : history }, historyChanged: !same };
}

export default function ChatView({ chatId, openSettingsInitially, status, onBack, onChanged, onCreate, onDeleted }) {
  const confirm = useConfirm();
  useLanguage();
  const [chat, setChat] = useState(null);
  const [offline, setOffline] = useState(false);
  const [stream, setStream] = useState(null); // Presentation: phase/ack only. The SSE buffer lives in ctxRef.
  const [error, setError] = useState('');
  const [drawer, setDrawer] = useState(!!openSettingsInitially);
  const [castOpen, setCastOpen] = useState(false);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [menuMsg, setMenuMsg] = useState(null);
  const [editMsg, setEditMsg] = useState(null);
  const [versionsFor, setVersionsFor] = useState(null);
  const [detailFor, setDetailFor] = useState(null);
  const [toast, setToast] = useState('');
  const [stuck, setStuck] = useState(true);
  const [newContent, setNewContent] = useState(false);
  const [presets, setPresets] = useState(null);
  const [cast, setCast] = useState([]);
  const [modelOnline, setModelOnline] = useState(null);
  const [viewerAvatar, setViewerAvatar] = useState(null);
  const [renameOpen, setRenameOpen] = useState(false);

  const chatRef = useRef(null);
  const streamRef = useRef(null);
  const scrollRef = useRef(null);
  const composerRef = useRef(null);
  const listRef = useRef(null);
  const readingRef = useRef(null);
  const openedRef = useRef(false);
  const abortRef = useRef(null);
  const busyRef = useRef(false);
  const gidRef = useRef(null);
  const reloadTimer = useRef(null);
  const recoveringRef = useRef(false);
  const cacheSaveTimer = useRef(null);
  chatRef.current = chat;
  streamRef.current = stream;
  useEffect(() => setModelOnline(null), [chat?.routeId, chat?.effort]);

  const busy = !!stream;
  const flash = (t) => {
    setToast(t);
    setTimeout(() => setToast(''), 1800);
  };

  useLayer(drawer, () => setDrawer(false));
  // The bar is not part of the layer stack (nav.js): collapsing an older layer would close the menu opened from it.
  const topBar = useTopBar();
  // opening a panel from the bar also collapses it
  const { hide: hideBar } = topBar;
  useEffect(() => {
    if (drawer || memoryOpen || renameOpen || viewerAvatar) hideBar();
  }, [drawer, memoryOpen, renameOpen, viewerAvatar, hideBar]);

  // ---------- loading / sync ----------
  const load = useCallback(async () => {
    try {
      const c = await api.getChat(chatId);
      // Partial snapshots persisted for recovery are not finished messages.
      c.history = c.history.filter(m => m.status !== 'generating');
      const base = chatRef.current;
      const merged = mergeChat(base, c);
      if (openedRef.current && merged.historyChanged) setNewContent(true);
      setChat((cur) => (cur === base ? merged.chat : mergeChat(cur, c).chat)); // no content change keeps the reference: nothing re-renders
      setOffline(false);
      return c;
    } catch (e) {
      setOffline(true);
      if (e.status && e.status !== 0) setError(e.message);
      return null;
    }
  }, [chatId]);

  useEffect(() => {
    let alive = true;
    api.cachedChat(chatId).then((c) => alive && c && !chatRef.current && setChat({ ...c, history: c.history.filter(m => m.status !== 'generating') }));
    api.cachedPresets().then((cp) => alive && setPresets((cur) => cur || cp));
    api.presets().then((p) => alive && setPresets(p)).catch(() => {});
    load().then((c) => {
      if (!alive || !c || busyRef.current) return;
      if (c.generation) attach(c.generation);
      else if (api.getPending(chatId)) recoverPending(api.getPending(chatId));
    });
    return () => {
      alive = false;
      if (ctxRef.current) ctxRef.current.cancelled = true;
      abortRef.current?.abort();
      clearTimeout(reloadTimer.current);
      resetPull();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId]);

  // Debounce writes of the encrypted cache (WebCrypto + IndexedDB) to avoid
  // repeated CPU/I/O cycles on every quick state change.
  useEffect(() => {
    if (!chat) return;
    clearTimeout(cacheSaveTimer.current);
    cacheSaveTimer.current = setTimeout(() => {
      api.saveChatCache(chat);
    }, 800);
  }, [chat]);

  useEffect(() => {
    return () => {
      clearTimeout(cacheSaveTimer.current);
      if (chatRef.current) api.saveChatCache(chatRef.current);
    };
  }, []);

  // other devices / later processing (memory, review) -> reload when the revision advances
  useEffect(() => {
    const on = (e) => {
      const ev = e.detail;
      if (ev.convId !== chatId || busyRef.current) return;
      const cur = chatRef.current;
      if (ev.revision != null && cur && ev.revision <= cur.revision && ev.type !== 'flags') return;
      clearTimeout(reloadTimer.current);
      reloadTimer.current = setTimeout(load, 350);
    };
    window.addEventListener('mc:event', on);
    return () => {
      window.removeEventListener('mc:event', on);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId, load]);
  const resume = useCallback(async () => {
    if (busyRef.current && gidRef.current) return reattachNow();
    if (recoveringRef.current) return;
    recoveringRef.current = true;
    try {
      {
        if (busyRef.current) {
          if (ctxRef.current) ctxRef.current.cancelled = true;
          abortRef.current?.abort();
          busyRef.current = false;
        }
        const c = await load();
        if (c?.generation) await attach(c.generation);
        else if (c && api.getPending(chatId)) await recoverPending(api.getPending(chatId));
      }
    } finally { recoveringRef.current = false; }
  }, [chatId, load]);
  useResume(resume);

  // ---------- history scrolling and paging ----------
  const history = chat?.history || [];
  const visibleHistory = stream?.mode === 'regenerate' && stream?.partId
    ? history.filter(m => m.id !== stream.partId) : history;
  const [renderedCount, setRenderedCount] = useState(40);
  const prevLenRef = useRef(visibleHistory.length);
  useEffect(() => {
    if (visibleHistory.length > prevLenRef.current) {
      setRenderedCount((c) => c + (visibleHistory.length - prevLenRef.current));
    }
    prevLenRef.current = visibleHistory.length;
  }, [visibleHistory.length]);
  const hasMore = visibleHistory.length > renderedCount;
  const displayedHistory = hasMore ? visibleHistory.slice(visibleHistory.length - renderedCount) : visibleHistory;
  useReadingGesture(scrollRef, !drawer && !editMsg && !menuMsg);
  const openDrawer = () => { resetPull(); setDrawer(true); };
  const drawerGesture = useDrawerGesture(() => setDrawer(true), !drawer && !editMsg && !menuMsg && !castOpen && !memoryOpen && !versionsFor && !detailFor && !renameOpen && !viewerAvatar);
  // Reading anchor: first message whose bottom is below the visible top. Binary search over the list's
  // children (ordered by layout): ~9 geometry reads instead of one per message.
  const rememberReading = () => {
    const el = scrollRef.current;
    if (!el) return;
    const top = el.getBoundingClientRect().top;
    const ey = Number(el.dataset.elasticY) || 0; // elastic stretch in progress (content translateY)
    const kids = listRef.current?.children;
    let anchor = null;
    if (kids?.length) {
      let lo = 0, hi = kids.length;
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (kids[mid].getBoundingClientRect().bottom - ey > top) hi = mid; else lo = mid + 1;
      }
      while (lo < kids.length && !kids[lo].dataset.messageId) lo++; // skip notices/cards without an id
      anchor = kids[lo] || null;
    }
    readingRef.current = { top: el.scrollTop, id: anchor?.dataset.messageId, offset: anchor ? anchor.getBoundingClientRect().top - top - ey : 0 };
  };
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || !chat) return;
    if (!openedRef.current) {
      openedRef.current = true;
      el.scrollTop = el.scrollHeight; // A single jump to the end when the chat opens.
    } else if (readingRef.current) {
      const r = readingRef.current;
      const anchor = r.id ? el.querySelector(`[data-message-id="${CSS.escape(r.id)}"]`) : null;
      el.scrollTop = anchor ? el.scrollTop + anchor.getBoundingClientRect().top - el.getBoundingClientRect().top - (Number(el.dataset.elasticY) || 0) - r.offset : r.top;
    }
    rememberReading();
  }, [chat, stream, error, offline, renderedCount]);

  function onScroll() {
    const el = scrollRef.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 140;
    setStuck(near);
    rememberReading(); // binary search: cheap with a clean layout, and the anchor never goes stale for the next commit
  }
  const toBottom = () => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
    setStuck(true);
    setNewContent(false);
  };

  const updateChat = (fn) =>
    setChat((prev) => (prev ? fn(prev) : prev));

  // ---------- generation ----------
  function makeHandler(ctx) {
    return (ev) => {
      if (ctxRef.current !== ctx || ctx.cancelled) return;
      if (ev.ack) {
        ctx.acked = true;
        gidRef.current = ev.ack.generationId;
        ctx.mode = ev.ack.mode;
        ctx.partId = ev.ack.partId;
        api.setPending(chatId, { ...ctx.payload, generationId: ev.ack.generationId, mode: ctx.mode, partId: ctx.partId }).catch(()=>setError(tr("Falha ao proteger a confirmação no cache; a resposta continua salva no PC.")));
        setOffline(false);
        if (ev.ack.userMessage && (ev.ack.mode === 'send' || ev.ack.mode === 'edit')) {
          updateChat((c) => {
            const exists = c.history.some((m) => m.id === ev.ack.userMessage.id);
            if (exists) return c;
            if (ev.ack.mode === 'edit') {
              const idx = c.history.findIndex((m) => m.interactionId === ctx.targetInteraction && m.role === 'user');
              return { ...c, revision: ev.ack.revision, history: [...(idx >= 0 ? c.history.slice(0, idx) : c.history), ev.ack.userMessage] };
            }
            return { ...c, revision: ev.ack.revision, history: [...c.history, ev.ack.userMessage] };
          });
          // the server received the message: the draft can go now
          if (ctx.fromComposer) {
            composerRef.current?.clear();
          }
        }
        if (ev.ack.mode === 'edit') setEditMsg(null);
        setNewContent(true);
        setStream((s) => ({ ...(s || {}), mode: ctx.mode, partId: ctx.partId, phase: 'waiting', ack: ev.ack }));
      }
      if (ev.context) setStream((s) => (s?.ack ? { ...s, ack: { ...s.ack, context: ev.context } } : s));
      if (ev.delta) {
        ctx.text += ev.delta; // on reconnect the server resends only what was missing
      }
      if (ev.done) {
        ctx.done = ev;
        if (ev.message) setModelOnline(!ev.error && ev.message.status !== 'error');
        else if (ev.error) setModelOnline(false);
        setNewContent(true);
        if (ev.message) {
          updateChat((c) => {
            const has = c.history.some((m) => m.id === ev.message.id);
            const history2 = has ? c.history.map((m) => (m.id === ev.message.id ? ev.message : m)) : [...c.history, ev.message];
            return { ...c, revision: ev.revision ?? c.revision, history: history2 };
          });
        }
        if (ev.error) setError(ev.error);
      }
      if (ev.error && !ev.done) { setError(ev.error); setModelOnline(false); }
    };
  }

  async function finishRun(ctx) {
    if (ctxRef.current !== ctx || ctx.cancelled || ctx.finishing) return;
    ctx.finishing = true;
    // Reconcile before re-enabling the buttons and removing the waiting card.
    const reconciled = await load();
    if (ctxRef.current !== ctx || ctx.cancelled) return;
    if (!reconciled) { ctx.finishing = false; return; }
    await api.setPending(chatId, null);
    setStream(null);
    busyRef.current = false;
    gidRef.current = null;
    abortRef.current = null;
    // reconcile with the server (branches, versions, flags)
    onChanged();
  }

  async function reattach(ctx, tries = 0) {
    const gid = gidRef.current;
    if (!gid || ctx.cancelled || ctx.done || ctxRef.current !== ctx) return 'superseded';
    const epoch = (ctx.epoch || 0) + 1;
    ctx.epoch = epoch;
    abortRef.current?.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    try {
      await api.attachGeneration(chatId, gid, ctx.text.length, ev => {
        if (ctx.epoch === epoch) makeHandler(ctx)(ev);
      }, ctrl.signal);
    } catch (e) {
      if (ctrl.signal.aborted || ctx.epoch !== epoch || ctx.cancelled) return 'superseded';
      if (tries < 6 && !ctx.done) {
        await new Promise((r) => setTimeout(r, Math.min(8000, 1000 * 2 ** tries)));
        if (!ctx.done && busyRef.current && ctx.epoch === epoch && !ctx.cancelled) return reattach(ctx, tries + 1);
      }
    }
    if(ctx.cancelled || ctx.epoch !== epoch)return 'superseded';
    return ctx.done ? 'done' : 'failed';
  }

  const ctxRef = useRef(null);
  async function reattachNow() {
    const ctx = ctxRef.current;
    if (!ctx || ctx.cancelled) return;
    if (ctx.done) return finishRun(ctx);
    setError('');
    const result=await reattach(ctx);
    if (result==='done') await finishRun(ctx);
    else if (result==='failed') { setOffline(true); setError(tr("A resposta continua salva no PC. Reconecte para recebê-la.")); }
  }

  async function attach(gen) {
    busyRef.current = true;
    gidRef.current = gen.id;
    const ctx = { text: '', acked: true, done: null, mode: gen.mode, partId: gen.partId, payload: api.getPending(chatId) || {} };
    ctxRef.current = ctx;
    setStream({ phase: 'waiting', mode: gen.mode, partId: gen.partId });
    const result=await reattach(ctx);
    if (result==='done') await finishRun(ctx);
    else if (result==='failed') { setOffline(true); setError(tr("Conexão interrompida. A resposta continua no PC; volte ao app para reconectar.")); }
  }

  function recoverPending(pending) {
    return pending.generationId
      ? attach({ id: pending.generationId, mode: pending.mode, partId: pending.partId })
      : run(pending, { recovery: true }); // same opId: the server replays, without duplicating
  }

  async function run(payload, { fromComposer = false, recovery = false } = {}) {
    if (busyRef.current || (offline && !recovery)) return false;
    busyRef.current = true;
    setError('');
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    payload = { ...payload, opId: payload.opId || uuid() };
    const target = payload.mode === 'regenerate' ? [...(chatRef.current?.history || [])].reverse().find(m => m.role === 'assistant') : null;
    const ctx = { text: '', acked: false, done: null, fromComposer: fromComposer || (recovery && payload.mode === 'send'),
      targetInteraction: payload.messageId, payload, mode: payload.mode, partId: target?.id, epoch: 0 };
    ctxRef.current = ctx;
    setStream({ phase: 'sending', mode: payload.mode, partId: target?.id });
    try { await flushStyleSettings(); } catch (e) {
      if (ctx.cancelled) return false;
      busyRef.current = false;
      setStream(null);
      setError('Não foi possível salvar o estilo antes de gerar. ' + e.message);
      return false;
    }
    if (ctx.cancelled || ctrl.signal.aborted) return false;
    try { await api.setPending(chatId, payload); } catch(e) {
      busyRef.current=false;setStream(null);setError(tr("Não foi possível proteger o envio no aparelho."));return false;
    }
    try {
      await api.sendMessage(chatId, payload, ev => { if (ctx.epoch === 0) makeHandler(ctx)(ev); }, ctrl.signal);
    } catch (e) {
      if (ctx.cancelled) return false;
      if (e.name === 'AbortError' && ctx.acked && !ctx.done) {
        /* manual reconnection in progress */
      } else if (!ctx.acked) {
        // nothing was saved: the draft stays in the composer
        setError(e.status === 422 ? e.message : e.message);
        if (e.status > 0) await api.setPending(chatId, null);
        else setOffline(true);
        setStream(null);
        busyRef.current = false;
        abortRef.current = null;
        return false;
      }
    }
    if (ctx.epoch > 0 && !ctx.done) return ctx.acked;
    if (ctx.acked && !ctx.done && ctx.epoch === 0) {
      const result=await reattach(ctx);
      if(result==='superseded')return ctx.acked;
    }
    if (ctx.cancelled) return false;
    if (ctx.epoch > 1 && !ctx.done) return ctx.acked;
    if (ctx.done) await finishRun(ctx);
    else { setOffline(true); setError(tr("Conexão interrompida. A resposta continua no PC; volte ao app para reconectar.")); }
    return ctx.acked;
  }

  async function stop() {
    const gid = gidRef.current;
    if (!gid) {
      abortRef.current?.abort();
      return;
    }
    try {
      await api.stopGeneration(chatId, gid);
    } catch (e) {
      setError(e.message);
    }
  }

  function send(text) {
    const content = text.trim();
    if (!content || busy) return;
    if (text.length > 6000) return setError(tr("Sua mensagem pode ter no máximo 6.000 caracteres."));
    run({ mode: 'send', content }, { fromComposer: true });
  }

  let lastAsst = null;
  for (let i = history.length - 1; i >= 0; i--) if (history[i].role === 'assistant') { lastAsst = history[i]; break; }
  // Stable callbacks: messages are memoized and only render when their own content changes.
  const openMenu = useEvent((m) => { if (!busy) setMenuMsg(m); });
  const onBranch = useEvent((m, d) => switchBranch(m, d));
  const onVersions = useEvent((m) => setVersionsFor(m));
  const onContinue = useEvent(() => run({ mode: 'continue' }));
  const onRegenerate = useEvent(() => run({ mode: 'regenerate' }));
  const onStop = useEvent(() => stop());
  const lastIsAssistant = history.length > 0 && history[history.length - 1].role === 'assistant';

  async function copyMsg(m) {
    setMenuMsg(null);
    flash((await copyText(m.content)) ? tr("Copiado") : tr("Não foi possível copiar"));
  }

  async function deleteMsg(m) {
    setMenuMsg(null);
    if (!await confirm(m.role === 'user' ? tr("Apagar esta mensagem e as respostas dela? (fica na lixeira por 30 dias)") : tr("Apagar esta resposta? (fica na lixeira por 30 dias)"))) return;
    try {
      await api.deleteMessage(chatId, m.id);
      await load();
      onChanged();
    } catch (e) {
      setError(e.message);
    }
  }

  async function saveEdit(m, content) {
    const text = content.trim();
    if (!text || text === m.content) { setEditMsg(null); return; }
    if (m.role === 'user') {
      const accepted = await run({ mode: 'edit', messageId: m.interactionId, content: text });
      if (!accepted) throw new Error(tr("A correção não foi enviada. Confira a conexão e tente novamente."));
      return;
    }
    try {
      const saved = await api.editMessage(chatId, m.id, text);
      updateChat((c) => ({ ...c, revision: saved.revision, history: c.history.map((x) => (x.id === m.id ? { ...saved } : x)) }));
      onChanged();
      setEditMsg(null);
    } catch (e) {
      throw e;
    }
  }

  async function switchBranch(m, delta) {
    const b = m.branch;
    if (!b) return;
    const target = b.ids[(b.index - 1 + delta + b.count) % b.count];
    try {
      const c = await api.activateBranch(chatId, target);
      setChat(c);
      onChanged();
    } catch (e) {
      setError(e.message);
    }
  }

  const loadCast = useCallback(() => api.cast(chatId).then((r) => setCast((prev) => (sameJson(prev, r.cast) ? prev : r.cast))).catch(() => {}), [chatId]);
  useEffect(() => {
    loadCast();
  }, [loadCast, chat?.characterRev]);
  useEffect(() => {
    const on = (e) => e.detail.convId === chatId && ['cast', 'memory', 'character'].includes(e.detail.type) && loadCast();
    window.addEventListener('mc:event', on);
    return () => window.removeEventListener('mc:event', on);
  }, [chatId, loadCast]);

  const presetInfo = presets?.presets?.find((p) => p.id === chat?.presetId);
  const recentReply = history.at(-1);
  const recoveredOnline = recentReply?.role === 'assistant' && recentReply.status !== 'error'
    && (!recentReply.model || recentReply.model === chat?.routeId)
    && (recentReply.effort == null || recentReply.effort === chat?.effort);
  const connected = !offline && (modelOnline ?? recoveredOnline);
  const serverOnline = !offline && (status ? status === 'online' : true);
  const isChecking = status === 'checking';
  const handleTitle = serverOnline
    ? tr("Conectado ao PC · toque ou puxe para baixo")
    : isChecking
      ? tr("Verificando conexão · toque ou puxe para baixo")
      : tr("Sem conexão com o PC · toque ou puxe para baixo");

  return (
    <div {...drawerGesture} className="mc-chat fixed inset-0 z-30 flex flex-col bg-bgDark">
      {/* Collapsed, the top bar is only the status dot at the top center (tap or pull down to open). */}
      <button
        ref={topBar.handleRef}
        onClick={topBar.onHandleClick}
        className={`mc-bar-handle ${topBar.open ? 'mc-bar-open' : ''}`}
        style={{ top: 'var(--sat)' }}
        aria-label={tr("Mostrar barra superior")}
        aria-expanded={topBar.open}
        title={handleTitle}
      >
        <span className={`mc-bar-dot ${serverOnline ? 'mc-on' : ''} ${isChecking ? 'mc-dot-checking' : ''}`} />
      </button>
      {/* the bubble that swells from the dot and the pop ring (moved by top-bar-motion.js) */}
      {/* Each one is a plain box (moved every frame by top-bar-motion.js) around the face that holds the heavy glass style:
          writing transform/opacity on the glass itself re-resolved its gradients, shadows and pseudo-elements on every frame. */}
      <span ref={topBar.bubbleRef} aria-hidden="true" className="mc-bar-bubble" style={{ top: 'calc(var(--sat) + 15px)' }}><span className="mc-glass-pill mc-bar-bubble-face absolute inset-0" /></span>
      <span ref={topBar.ringRef} aria-hidden="true" className="mc-bar-ring" style={{ top: 'calc(var(--sat) + 15px)' }}><span className="mc-bar-ring-face" /></span>
      {/* Detached floating header with individual liquid-glass islands */}
      <header ref={topBar.barRef} className={`${topBar.settled ? 'pointer-events-auto' : 'pointer-events-none mc-bar-animating'} fixed inset-x-0 top-0 z-30 px-3`} style={{ paddingTop: 'calc(var(--sat) + 8px)' }}>
        <div className="mc-col flex items-center justify-between gap-2">
          {/* Left side: round back button + capsule with avatar and name */}
          <div className="flex items-center gap-2 min-w-0 flex-1">
            <div data-bar-item className="shrink-0">
              <button
                onClick={onBack}
                className="mc-glass-pill pointer-events-auto flex h-11 w-11 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
                aria-label={tr("Voltar")}
              >
                <ChevronLeft size={24} />
              </button>
            </div>
            <div data-bar-item className="flex min-w-0"><div className="mc-glass-pill mc-identity-pill pointer-events-auto flex min-w-0 items-center gap-2 h-11 rounded-full pl-1 pr-3.5 shadow-lg">
              <button
                onClick={() => setViewerAvatar({ name: chat?.name, avatar: chat?.avatar })}
                className="shrink-0 active:scale-95 transition-transform"
                aria-label={tr("Ver avatar")}
              >
                <Avatar name={chat?.name} avatar={chat?.avatar} size={32} round />
              </button>
              <div className="min-w-0 leading-tight">
                <button
                  onClick={() => setRenameOpen(true)}
                  className="flex items-center gap-1.5 text-left max-w-full active:opacity-80"
                  aria-label={tr("Renomear conversa")}
                >
                  <span className="truncate text-[14px] font-semibold">{chat?.name || '...'}</span>
                  <Pencil size={12} className="shrink-0 text-textMuted opacity-70" />
                </button>
                <button
                  onClick={openDrawer}
                  className="flex max-w-full items-center gap-1.5 text-[11px] text-textMuted text-left"
                  aria-label={tr("Opções do modelo")}
                >
                  <span className="truncate">{presetInfo?.label || chat?.routeId?.split('/').pop()}{chat?.effort ? ` · ${chat.effort}` : ''}</span>
                  <span
                    title={connected ? tr("Modelo conectado: resposta recebida") : tr("Conexão do modelo ainda não confirmada")}
                    aria-label={connected ? tr("Modelo conectado") : tr("Modelo sem conexão confirmada")}
                    className={`inline-block h-2 w-2 shrink-0 rounded-full ${connected ? 'bg-emerald-400' : 'bg-gray-500'}`}
                  />
                </button>
              </div>
            </div></div>
          </div>

          {/* Right side: independent round memory and options buttons */}
          <div className="flex items-center gap-2 shrink-0">
            <div data-bar-item>
              <button
                onClick={() => setMemoryOpen(true)}
                className="mc-glass-pill pointer-events-auto relative flex h-11 w-11 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
                aria-label={tr("Memória")}
              >
                <BrainIcon size={22} />
                {chat?.memorySummary?.pending > 0 && <span className="absolute right-1.5 top-1.5 h-2 w-2 rounded-full bg-accentPurpleLight ring-2 ring-black/40" />}
              </button>
            </div>
            <div data-bar-item>
              <button
                onClick={openDrawer}
                className="mc-glass-pill pointer-events-auto flex h-11 w-11 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
                aria-label={tr("Opções")}
              >
                <MoreHorizontal size={24} />
              </button>
            </div>
          </div>
        </div>
      </header>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        onPointerDown={topBar.open ? topBar.hide : undefined}
        className="mc-scroll relative min-h-0 flex-1 overflow-y-auto px-3"
        style={{
          paddingTop: 'calc(var(--sat) + 34px)',
          paddingBottom: 'calc(var(--sab) + 84px)',
        }}
      >
        <div ref={listRef} className="mc-col space-y-3 mc-read">
          {offline && (
            <div className="flex items-center gap-2 rounded-xl bg-yellow-500/10 px-3 py-2 text-xs text-yellow-300 mb-2">
              <WifiOff size={14} />{tr(" Sem conexão com o PC. Você pode ler e escrever rascunhos; o envio precisa do servidor.\n              ")}<button className="ml-auto underline" onClick={resume}>{tr("Tentar de novo")}</button>
            </div>
          )}
          {history.length === 0 && !busy && (
            <p className="pt-16 text-center text-sm text-textMuted">{tr("\n              Comece a conversa. Personalize o personagem em ")}<b>{tr("⋯ → Personalização")}</b>.
            </p>
          )}
          {hasMore && (
            <div className="py-2 text-center">
              <button
                type="button"
                onClick={() => setRenderedCount((c) => Math.min(visibleHistory.length, c + 40))}
                className="rounded-full bg-surface px-4 py-1.5 text-xs font-medium text-textMuted ring-1 ring-white/10 active:opacity-80"
              >{tr("\n                Carregar mensagens anteriores (")}{visibleHistory.length - displayedHistory.length})
              </button>
            </div>
          )}
          {displayedHistory.map((m) => {
            if (m.role === 'user') return <UserBubble key={m.id} m={m} onLongPress={openMenu} onBranch={onBranch} />;
            const isLast = m.id === lastAsst?.id && history[history.length - 1]?.id === m.id;
            return (
              <AssistantCard
                key={m.id}
                m={m}
                isLast={isLast}
                busy={isLast && busy}
                offline={isLast && offline}
                onLongPress={openMenu}
                onVersions={onVersions}
                onContinue={onContinue}
                onRegenerate={onRegenerate}
              />
            );
          })}
          {busy && <StreamCard onStop={onStop} idle={offline} />}
          {error && (
            <div className="flex items-start gap-2 rounded-xl bg-red-500/10 px-3 py-2 text-sm text-red-300">
              <span className="flex-1 break-words">{tr(error)}</span>
              <button onClick={() => setError('')} className="shrink-0 text-xs underline">{tr("fechar")}</button>
            </div>
          )}
          {!busy && history.length > 0 && !lastIsAssistant && (
            <div className="mc-assistant">
              <ChatActions offline={offline} retry onRegenerate={onRegenerate} />
            </div>
          )}
        </div>
        {(!stuck || newContent) && (
          <button
            onClick={toBottom}
            className="mc-glass-pill fixed bottom-[calc(var(--sab)+82px)] right-4 z-20 flex h-11 w-11 items-center justify-center rounded-full active:scale-90"
            aria-label={tr("Ir para o fim")}
          >
            <ArrowDown size={20} />
          </button>
        )}
      </div>

      <Composer ref={composerRef} chatId={chatId} busy={busy} offline={offline} onSend={send} />
      {drawer && chat && (
        <SettingsDrawer
          chat={chat}
          presets={presets}
          busy={busy}
          onClose={() => setDrawer(false)}
          onSaved={(patch) => {
            updateChat((c) => ({ ...c, ...patch }));
            onChanged();
          }}
          onHistoryCleared={() => {
            load();
            onChanged();
          }}
          onOpenCast={() => setCastOpen(true)}
          onCreate={onCreate}
          onDeleted={onDeleted}
        />
      )}
      {castOpen && <CastScreen chatId={chatId} persona={chat?.persona} onClose={() => setCastOpen(false)} onChanged={loadCast} />}
      {memoryOpen && <MemoryScreen chatId={chatId} onClose={() => setMemoryOpen(false)} />}

      {menuMsg && (
        <Sheet onClose={() => setMenuMsg(null)} title={plainText(menuMsg.content).slice(0, 120)}>
          <SheetButton icon={<Copy size={20} />} label={tr("Copiar")} onClick={() => copyMsg(menuMsg)} />
          <SheetButton
            icon={<Pencil size={20} />}
            label={menuMsg.role === 'user' ? tr("Editar e reenviar (novo ramo)") : tr("Editar")}
            onClick={() => {
              setEditMsg(menuMsg);
              setMenuMsg(null);
            }}
          />
          {menuMsg.role === 'assistant' && menuMsg.versionCount > 1 && (
            <SheetButton icon={<Layers size={20} />} label={tr("Alternativas ({0})", [menuMsg.versionCount])} onClick={() => { setVersionsFor(menuMsg); setMenuMsg(null); }} />
          )}
          {menuMsg.role === 'assistant' && (
            <SheetButton icon={<Info size={20} />} label={tr("Detalhes e tokens")} onClick={() => { setDetailFor(menuMsg); setMenuMsg(null); }} />
          )}
          <SheetButton icon={<Trash2 size={20} />} label={tr("Apagar")} danger onClick={() => deleteMsg(menuMsg)} />
        </Sheet>
      )}

      {editMsg && <EditSheet msg={editMsg} onCancel={() => setEditMsg(null)} onSave={(t) => saveEdit(editMsg, t)} />}
      {versionsFor && (
        <VersionsSheet
          chatId={chatId}
          msg={versionsFor}
          onClose={() => setVersionsFor(null)}
          onSelected={(saved) => {
            updateChat((c) => ({ ...c, revision: saved.revision, history: c.history.map((x) => (x.id === saved.id ? saved : x)) }));
            setVersionsFor(null);
            onChanged();
          }}
        />
      )}
      {detailFor && <DetailSheet msg={detailFor} chat={chat} onClose={() => setDetailFor(null)} />}

      {toast && (
        <div className="pointer-events-none fixed inset-x-0 bottom-28 z-[60] flex justify-center">
          <span className="rounded-full bg-white/90 px-4 py-2 text-sm font-medium text-black">{tr(toast)}</span>
        </div>
      )}

      {viewerAvatar && (
        <AvatarViewerModal item={viewerAvatar} onClose={() => setViewerAvatar(null)} />
      )}

      {renameOpen && (
        <RenameModal
          initial={chat?.name || ''}
          onClose={() => setRenameOpen(false)}
          onSave={async (newName) => {
            await api.updateChat(chatId, { name: newName });
            updateChat((c) => ({ ...c, name: newName }));
            onChanged();
          }}
        />
      )}
    </div>
  );
}

export function VerdictDot({ verdict }) {
  useLanguage();
  const map = {
    forwarded: ['bg-emerald-400', 'Esforço confirmado no pedido enviado ao provedor'],
    not_forwarded: ['bg-red-400', 'O provedor recebeu um esforço diferente do preset'],
    unverified: ['bg-gray-500', 'Não foi possível conferir no registro do provedor'],
    confirmed: ['bg-emerald-400', 'Esforço confirmado nesta rota'],
    sent_unconfirmed: ['bg-yellow-400', 'Esforço enviado, mas a rota não permite confirmar'],
    ignored: ['bg-orange-400', 'Esforço enviado, sem efeito mensurável nos testes (não validado)'],
    not_validated: ['bg-gray-500', 'Rota ainda não validada'],
    error: ['bg-red-400', 'Falha na validação'],
  };
  const [cls, label] = map[verdict] || map.not_validated;
  return <span title={tr(label)} aria-label={tr(label)} className={`inline-block h-2 w-2 shrink-0 rounded-full ${cls}`} />;
}

const UserBubble = memo(function UserBubble({ m, onLongPress, onBranch }) {
  useLanguage();
  const press = useLongPress(() => onLongPress(m));
  return (
    <div data-message-id={m.id} className="flex flex-col items-end">
      <div {...press} className="mc-msg max-w-[88%] rounded-2xl rounded-br-md bg-accentPurple/25 px-4 py-3">
        <UserText text={m.content} />
        {m.edited && <span className="mt-1 block text-right text-[11px] text-textMuted">{tr("editada")}</span>}
      </div>
      {m.branch && (
        <div className="mt-1 flex items-center gap-1 text-xs text-textMuted">
          <button onClick={() => onBranch(m, -1)} className="p-1" aria-label={tr("Ramo anterior")}><ChevronLeft size={14} /></button>{tr("\n          ramo ")}{m.branch.index}/{m.branch.count}
          <button onClick={() => onBranch(m, 1)} className="p-1" aria-label={tr("Próximo ramo")}><ChevronRight size={14} /></button>
        </div>
      )}
    </div>
  );
});

const AssistantCard = memo(function AssistantCard({ m, isLast, busy, offline, onContinue, onRegenerate, onLongPress, onVersions }) {
  useLanguage();
  const press = useLongPress(() => onLongPress(m));
  const note = m.status === 'length' ? null : STATUS_NOTE[m.status];
  return (
    <div data-message-id={m.id} className="mc-assistant">
      <div className="rounded-2xl bg-cardDark px-3 py-3">
        <div {...press} className="mc-msg">
        <SceneText text={m.content} />
        {m.edited && <span className="mt-1 block text-right text-[11px] text-textMuted">{tr("editada")}</span>}
        </div>
        {m.status === 'length' && lengthNotice(m) && (
          <div role="status" className="mt-2 flex items-start gap-2 rounded-xl bg-yellow-500/10 px-3 py-2 text-xs leading-snug text-yellow-200">
            <AlertTriangle size={14} className="mt-px shrink-0" />
            <span>{lengthNotice(m)}</span>
          </div>
        )}
        {isLast && !busy && <ChatActions offline={offline} onContinue={onContinue} onRegenerate={onRegenerate} />}
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 px-1 text-xs text-textMuted">
        {m.versionCount > 1 && (
          <button onClick={() => onVersions(m)} className="flex items-center gap-1 underline-offset-2 hover:underline">
            <Layers size={12} /> {m.versionCount}{tr(" versões\n          ")}</button>
        )}
        {note && <span className={m.status === 'error' ? 'text-red-300' : 'text-yellow-300'}>{tr(note)}</span>}
        {m.usage?.provider?.effortOk === false && (
          <span className="text-red-300">
            {m.usage.provider.thinkingEnabled === false ? tr("⚠ não foi possível confirmar o pensamento desligado no provedor") : tr("⚠ provedor recebeu esforço {0} (esperado {1})", [m.usage.provider.effortSent ?? tr("nenhum"), m.usage.provider.effortExpected])}
          </span>
        )}
        {isLast && m.usage && !busy && <UsageLine usage={m.usage} />}
      </div>
    </div>
  );
});

// Composer with its own state: typing re-renders only this component, not the history. The draft is saved
// with a delay (and on exit/when the app is hidden), since every write encrypts the whole private storage.
const Composer = forwardRef(function Composer({ chatId, busy, offline, onSend }, ref) {
  useLanguage();
  const [value, setValue] = useState(() => api.getDraft(chatId));
  const [collapsed, setCollapsed] = useState(true);
  const collapsedRef = useRef(true);
  const shellRef = useRef(null);
  const openHeight = useRef(0);
  const textRef = useRef(null);
  const expand = () => {
    openHeight.current = window.visualViewport?.height || window.innerHeight;
    collapsedRef.current = false; setCollapsed(false);
    requestAnimationFrame(() => textRef.current?.focus({ preventScroll: true }));
  };
  const valueRef = useRef(value);
  const draftTimer = useRef(0);
  const autosize = useRef(typeof CSS === 'undefined' || !CSS.supports?.('field-sizing', 'content'));
  valueRef.current = value;

  const flushDraft = useCallback(() => {
    if (!draftTimer.current) return;
    clearTimeout(draftTimer.current);
    draftTimer.current = 0;
    api.setDraft(chatId, valueRef.current);
  }, [chatId]);

  const collapse = useCallback(() => {
    if (collapsedRef.current) return;
    collapsedRef.current = true;
    setCollapsed(true);
    textRef.current?.blur();
    flushDraft();
  }, [flushDraft]);
  useLayer(!collapsed, collapse);
  useEffect(() => {
    if (collapsed) return;
    const outside = event => { if (!shellRef.current?.contains(event.target)) collapse(); };
    let keyboardSeen = false;
    const viewport = () => {
      const height = window.visualViewport?.height || window.innerHeight;
      if (height < openHeight.current - 120) keyboardSeen = true;
      else if (keyboardSeen && height >= openHeight.current - 60) collapse();
    };
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('resize', viewport);
    window.visualViewport?.addEventListener('resize', viewport);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('resize', viewport);
      window.visualViewport?.removeEventListener('resize', viewport);
    };
  }, [collapsed, collapse]);

  useImperativeHandle(ref, () => ({
    clear() {
      clearTimeout(draftTimer.current);
      draftTimer.current = 0;
      valueRef.current = '';
      setValue('');
      api.setDraft(chatId, '');
    },
  }), [chatId]);

  useEffect(() => {
    const hide = () => { if (document.visibilityState === 'hidden') flushDraft(); };
    document.addEventListener('visibilitychange', hide);
    window.addEventListener('pagehide', flushDraft);
    return () => {
      document.removeEventListener('visibilitychange', hide);
      window.removeEventListener('pagehide', flushDraft);
      flushDraft();
    };
  }, [flushDraft]);

  // field-sizing: content (Chrome 123+) avoids measuring scrollHeight; without it, measure once per text change.
  useLayoutEffect(() => {
    const t = textRef.current;
    if (!t || !autosize.current) return;
    t.style.height = 'auto';
    t.style.height = `${Math.min(t.scrollHeight, 160)}px`;
  }, [value]);

  const change = (e) => {
    const v = e.target.value;
    valueRef.current = v;
    setValue(v);
    clearTimeout(draftTimer.current);
    draftTimer.current = setTimeout(flushDraft, 600);
  };

  return (
    <>
      <div className="pointer-events-none fixed inset-x-0 bottom-0 z-30 px-3" style={{ paddingBottom: 'calc(var(--sab) + 8px)' }}>
        <div className="mc-col">
          <div ref={shellRef} className={`mc-composer-shell mc-glass-pill pointer-events-auto mx-auto rounded-[28px] shadow-xl ${collapsed ? 'mc-composer-collapsed' : ''}`}>
            <button onClick={expand} className="mc-composer-capsule" aria-label={tr('Expandir mensagem')} aria-expanded={!collapsed} tabIndex={collapsed ? 0 : -1} aria-hidden={!collapsed}>
              <Keyboard size={22} />
            </button>
            <div className="mc-composer-content" inert={collapsed ? true : undefined} aria-hidden={collapsed}>
            <textarea
              ref={textRef}
              rows={1}
              value={value}
              maxLength={6000}
              placeholder={tr("Digite uma mensagem")}
              enterKeyHint="enter"
              onChange={change}
              onBlur={flushDraft}
              spellCheck={true}

              autoCorrect="on"
              autoCapitalize="sentences"
              className="mc-composer-input min-w-0 max-h-[160px] flex-1 resize-none bg-transparent py-2 text-[16px] outline-none placeholder:text-textMuted"
            />
            <button
              onClick={() => onSend(value)}
              disabled={!value.trim() || busy || offline || value.length > 6000}
              className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-accentPurple text-white transition-[transform,opacity] active:scale-90 disabled:opacity-30 mb-0.5"
              aria-label={tr("Enviar")}
            >
              <SendHorizontal size={20} />
            </button>
            </div>
          </div>
          {value.length > 4800 && (
            <p className="pointer-events-auto px-4 pt-1 text-right text-xs text-textMuted">
              {value.length.toLocaleString('pt-BR')}{tr(" / 6.000 caracteres\n            ")}</p>
          )}
        </div>
      </div>
    </>
  );
});

export function UsageLine({ usage }) {
  useLanguage();
  // preference: real provider count (request log); then the one returned by the gateway; otherwise an estimate (~)
  const real = usage.provider?.usage;
  const est = (k) => (usage.method?.[k] === 'estimated' ? '~' : '');
  const input = real?.input ?? usage.input;
  const output = real?.output ?? usage.output;
  const reasoning = real?.reasoning ?? usage.reasoning;
  const p = usage.provider;
  return (
    <span title={tr("Sem ~ = contado pelo provedor; ~ = estimado localmente")}>
      ↑{real ? '' : est('input')}{fmtTokens(input)} ↓{real ? '' : est('output')}{fmtTokens(output)}
      {reasoning != null ? tr(" · raciocínio {0}{1}", [real || usage.method?.reasoning === 'measured' ? '' : '~', fmtTokens(reasoning)]) : ''}
      {real?.cacheRead ? tr(" · cache {0}", [fmtTokens(real.cacheRead)]) : ''}
      {p?.thinkingEnabled === false ? tr(" · pensamento desligado{0}", [p.thinkingOk ? ' ✓' : '']) : p?.effortOk ? tr(" · esforço {0} ✓", [p.effortSent]) : ''}
    </span>
  );
}

// The footer is a sibling of the long-press text: tapping the icons never opens the message menu.
function ChatActions({ offline, retry, onContinue, onRegenerate, onStop, stopping }) {
  useLanguage();
  const isolate = (e) => e.stopPropagation();
  const retryLabel = retry ? 'Tentar responder de novo' : 'Refazer a última resposta';
  return (
    <div className="mc-chat-actions" onTouchStart={isolate} onPointerDown={isolate} onClick={isolate} onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); }}>
      {onStop ? (
        <button type="button" onClick={onStop} disabled={stopping} className={`mc-chat-action ${stopping ? 'mc-stopping' : ''}`} aria-label={tr("Parar geração")} aria-busy={stopping || undefined} title={tr("Parar geração")}>
          <Square size={16} aria-hidden="true" />
        </button>
      ) : (
        <>
          {!retry && <button type="button" disabled={offline} onClick={onContinue} className="mc-chat-action" aria-label={tr("Continuar a cena")} title={tr("Continuar a cena")}>
            <FastForward size={22} aria-hidden="true" />
          </button>}
          <button type="button" disabled={offline} onClick={onRegenerate} className="mc-chat-action" aria-label={retryLabel} title={retryLabel}>
            <RotateCcw size={20} aria-hidden="true" />
          </button>
        </>
      )}
    </div>
  );
}

// "Thinking" dots: advanced by JavaScript (~3 Hz) instead of an infinite CSS animation, which asked for a frame on every
// vsync (60-120 per second) during the whole wait. Paused when the connection dropped, with the tab hidden or with
// "reduce motion": an idle screen produces no frames.
function ThinkingDots({ idle }) {
  useLanguage();
  const [lit, setLit] = useState(1);
  useEffect(() => {
    if (idle) { setLit(3); return undefined; }
    const calm = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    let step = 0;
    const id = setInterval(() => {
      if (document.visibilityState !== 'visible') return;
      setLit(calm?.matches ? 3 : 1 + (++step % 3));
    }, 360);
    return () => clearInterval(id);
  }, [idle]);
  return (
    <span className="mc-thinking-dots">
      {[1, 2, 3].map((n) => <span key={n} className={n <= lit ? 'on' : undefined}>.</span>)}
    </span>
  );
}

function StreamCard({ onStop, idle }) {
  useLanguage();
  const [stopping, setStopping] = useState(false);
  const timer = useRef(0);
  useEffect(() => () => clearTimeout(timer.current), []);
  // Immediate feedback: vibrates, shows "Stopping…" and keeps the button pressed until the reply stops (or 6 s, to retry).
  const stop = async () => {
    if (stopping) return;
    setStopping(true);
    try { navigator.vibrate?.(12); } catch { /* no vibration */ }
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setStopping(false), 6000);
    try { await onStop(); } catch { setStopping(false); }
  };
  return (
    <div className="mc-assistant">
      <div className="mc-generation">
          <div role="status" aria-live="polite" aria-label={stopping ? tr("Parando...") : tr("Pensando...")} className="mc-preparing text-sm text-textMuted">
            <span aria-hidden="true">{stopping ? tr("Parando") : tr("Pensando")}<ThinkingDots idle={idle || stopping} /></span>
          </div>
        <ChatActions onStop={stop} stopping={stopping} />
      </div>
    </div>
  );
}

function EditSheet({ msg, onCancel, onSave }) {
  useLanguage();
  const [v, setV] = useState(msg.content);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const isUser = msg.role === 'user';
  useLayer(true, onCancel);
  return (
    <div className="mc-editor fixed inset-0 z-50 flex flex-col bg-bgDark" style={{ paddingTop: 'calc(var(--sat) + 8px)', paddingBottom: 'calc(var(--sab) + 12px)' }}>
      <div className="mc-col flex items-center justify-between px-4 pb-3">
        <button onClick={onCancel} className="py-1 text-textMuted">{tr("Cancelar")}</button>
        <span className="font-semibold">{isUser ? tr("Editar sua mensagem") : tr("Editar resposta")}</span>
        <button onClick={async () => {
          setSaving(true); setError('');
          try { await onSave(v); } catch (e) { setError(e.message); }
          finally { setSaving(false); }
        }} disabled={saving || !v.trim() || v.length > (isUser ? 6000 : 10800)} className="py-1 font-semibold text-accentPurpleLight disabled:opacity-30">
          {saving ? tr("Salvando…") : isUser ? tr("Reenviar") : tr("Salvar")}
        </button>
      </div>
      {isUser && <p className="mc-col mb-2 px-4 text-xs text-textMuted">{tr("Cria um novo ramo: o caminho atual continua guardado e você pode voltar a ele.")}</p>}
      {error && <p role="alert" className="mc-col px-4 pb-2 text-sm text-red-300">{tr(error)}</p>}
      <textarea
        autoFocus
        value={v}
        maxLength={isUser ? 6000 : 10800}
        onChange={(e) => setV(e.target.value)}
        aria-label={tr("Texto da mensagem")}
        spellCheck={true}

        autoCorrect="on"
        autoCapitalize="sentences"
        className="mc-edit-text mx-auto min-h-0 flex-1 resize-none rounded-2xl bg-composer p-4 text-[16px] leading-relaxed outline-none"
      />
    </div>
  );
}

function VersionsSheet({ chatId, msg, onClose, onSelected }) {
  useLanguage();
  const [list, setList] = useState(null);
  const [err, setErr] = useState('');
  useEffect(() => {
    api.versions(chatId, msg.id).then((r) => setList(r.versions)).catch((e) => setErr(e.message));
  }, [chatId, msg.id]);
  async function pick(v) {
    try {
      onSelected(await api.selectVersion(chatId, msg.id, v.id));
    } catch (e) {
      setErr(e.message);
    }
  }
  const origin = { generate: 'Original', regenerate: 'Refeita', continue: 'Continuação', edit: 'Editada', legacy: 'Importada' };
  return (
    <Sheet onClose={onClose} title={tr("Alternativas desta resposta")} full>
      {err && <p className="px-3 text-sm text-red-300">{tr(err)}</p>}
      {!list && !err && <p className="px-3 text-sm text-textMuted">{tr("Carregando…")}</p>}
      {list?.map((v, i) => (
        <button key={v.id} onClick={() => pick(v)} className="mb-2 block w-full rounded-2xl bg-cardDark px-4 py-3 text-left active:bg-cardDarkHover">
          <div className="mb-1 flex items-center gap-2 text-xs text-textMuted">
            <span>#{i + 1} · {tr(origin[v.origin] || v.origin)}{v.status !== 'complete' ? ` · ${v.status}` : ''}</span>
            {v.selected && <span className="flex items-center gap-1 text-emerald-300"><Check size={12} />{tr(" em uso")}</span>}
          </div>
          <p className="line-clamp-4 text-sm">{plainText(v.content)}</p>
        </button>
      ))}
    </Sheet>
  );
}

function DetailSheet({ msg, chat, onClose }) {
  useLanguage();
  const u = msg.usage;
  return (
    <Sheet onClose={onClose} title={tr("Detalhes da resposta")}>
      <div className="space-y-1.5 px-3 pb-2 text-sm">
        <Row k={tr("Rota")} v={msg.model || chat?.routeId} />
        <Row k={u?.length?.thinkingEnabled === false ? tr("Esforço guardado (quando ligado)") : tr("Esforço pedido")} v={msg.effort || '—'} />
        {u?.length?.thinkingEnabled != null && <Row k={tr("Pensamento pedido")} v={u.length.thinkingEnabled ? tr("Ligado") : tr("Desligado")} />}
        {u?.length && <Row k={tr("Tamanho enviado")} v={tr("{0} caracteres · {1}", [u.length.maxCharacters, u.length.mode === 'target' ? tr("alvo aproximado") : tr("máximo")])} />}
        {u?.length && <Row k={tr("Orçamento da API")} v={tr("{0} tokens · ~{1} de texto + margem e raciocínio", [u.length.apiMaxTokens, u.length.visibleTokens])} />}
        {u?.style?.after && <Row k={tr("Tamanho recebido")} v={tr("{0} caracteres", [u.style.after.characters])} />}
        {u?.style?.after?.maxMonologueCharacters > 0 && <Row k={tr("Maior sequência de fala")} v={tr("{0} / {1} caracteres{2}", [u.style.after.longestMonologueCharacters, u.style.after.maxMonologueCharacters, u.style.after.monologueOk ? '' : tr(" · excedeu o ajuste")])} />}
        {u?.style?.attempted && <Row k={tr("Ajuste da resposta")} v={u.style.accepted ? tr("Reescrita aplicada") : tr("Original preservada; reescrita não aceita")} />}
        {u && <Row k={tr("Tokens de entrada")} v={`${u.method?.input === 'estimated' ? '~' : ''}${u.input} (${u.method?.input === 'estimated' ? tr("estimado") : tr("medido")})`} />}
        {u && <Row k={tr("Tokens de saída")} v={`${u.method?.output === 'estimated' ? '~' : ''}${u.output} (${u.method?.output === 'estimated' ? tr("estimado") : tr("medido")})`} />}
        {u && <Row k={tr("Raciocínio")} v={u.reasoning == null ? tr("não informado") : `${u.method?.reasoning === 'measured' ? '' : '~'}${u.reasoning} (${u.method?.reasoning === 'measured' ? tr("medido") : tr("estimado")})`} />}
        {u?.finish && <Row k={tr("Término")} v={u.finish} />}
        {u?.provider?.usage && <Row k={tr("Entrada real (provedor)")} v={`${u.provider.usage.input}${u.provider.usage.cacheRead ? tr(" · {0} do cache", [u.provider.usage.cacheRead]) : ''}`} />}
        {u?.provider && !u.provider.notFound && (u.provider.thinkingEnabled === false ? <Row k={tr("Pensamento no provedor")} v={u.provider.thinkingOk ? tr("Desligado ✓") : tr("Desligamento não confirmado")} /> : <Row k={tr("Esforço no provedor")} v={`${u.provider.effortSent ?? tr("nenhum")} ${u.provider.effortOk ? tr("✓ confere com o preset") : tr("✗ esperado {0}", [u.provider.effortExpected])}`} />)}
        {u?.provider?.notFound && <Row k={tr("Esforço no provedor")} v={tr('não encontrado no registro do provedor')} />}
        <Row k={tr("Estado")} v={msg.status} />
        <p className="pt-2 text-xs text-textMuted">{tr("O raciocínio interno do modelo nunca é exibido nem guardado; só a contagem aparece aqui.")}</p>
        {msg.status && <p className="text-xs text-textMuted">{tr("Controle de esforço desta rota: ")}{confirm[msg.effort ? 'sent_unconfirmed' : 'not_applicable'] ? '' : ''}{tr("veja o ponto colorido no cabeçalho.")}</p>}
      </div>
    </Sheet>
  );
}

const Row = ({ k, v }) => (
  <div className="flex justify-between gap-4">
    <span className="text-textMuted">{k}</span>
    <span className="break-all text-right">{v}</span>
  </div>
);
