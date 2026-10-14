import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage, currentLanguage } from '../i18n.js';
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ChevronRight, ChevronLeft, Trash2, MessageSquarePlus, Upload, Check, Users, Camera, Copy, Cpu, UserRound, Pencil, FileText, Layers, SlidersHorizontal, History } from 'lucide-react';
import { api } from '../api.js';
import { useLayer } from '../nav.js';
import { Avatar, fileToAvatar } from '../ui.jsx';
import { VerdictDot } from './ChatView.jsx';
import StyleForm from './StyleForm.jsx';
import PersonaManager from './PersonaManager.jsx';
import { usePanelDrag } from '../gestures.js';
import { PanelMotion, attachMotion, detachMotion } from '../drawer-motion.js';
import '../drawer.css';

const VERDICT_TEXT = {
  forwarded: 'Esforço fixo deste preset, conferido no pedido enviado ao modelo. Cada resposta é conferida de novo.',
  not_forwarded: 'O provedor recebeu um esforço diferente do preset nesta rota. Não use esta combinação.',
  unverified: 'Não deu para conferir no registro do provedor; o esforço é enviado, mas sem confirmação.',
  confirmed: 'Esforço confirmado nesta rota',
  sent_unconfirmed: 'Esforço enviado, mas a rota não expõe como confirmar. Não é um preset validado.',
  ignored: 'A rota aceita o parâmetro (HTTP 200), mas os testes reais não mostraram efeito mensurável no raciocínio. O esforço é enviado, porém não confirmado: não é um preset validado.',
  not_validated: 'Rota ainda não validada (rode scripts/validate-routes.mjs).',
  error: 'A validação desta rota falhou.',
};

// The menu items arrive in a staircase (drawer-motion.js): the chain writes transform/opacity on each item every
// frame. Written on the glass bubbles themselves, every write re-resolved the heavy styles of the bubble AND of its
// ::before/::after (gradients and shadows full of var()/calc(): ~70 us per element, ~70 elements per frame = ~4 ms of
// style recalculation on every frame of the animation). Each item sits in a plain box instead, so the per-frame
// writes land on elements with a trivial style and the bubbles are never restyled while they slide.
function stairItems(children, prefix = '') {
  const out = [];
  React.Children.toArray(children).forEach((child) => {
    if (React.isValidElement(child) && child.type === React.Fragment) out.push(...stairItems(child.props.children, `${prefix}${child.key}/`));
    else out.push(<div key={`${prefix}${child?.key ?? out.length}`} className="mc-stair">{child}</div>);
  });
  return out;
}
function StairList({ className = '', children }) {
  return <div className={`mc-elastic-wrapper ${className}`}>{stairItems(children)}</div>;
}

// Side drawer with the conversation settings.
export default function SettingsDrawer({ chat, presets, busy, onClose, onSaved, onHistoryCleared, onOpenCast, onCreate, onDeleted }) {
  const confirm = useConfirm();
  useLanguage();
  const [page, setPage] = useState(null); // null | model | name | persona | prompt | history | context | style
  const [gs, setGs] = useState(() => api.peekGlobalSettings()); // global settings (summary on the "Style and limits" row)
  const [temp, setTemp] = useState(chat.temperature ?? 0.8);
  const [error, setError] = useState('');
  const [thinkingSaving, setThinkingSaving] = useState(false);
  const [personaOpen, setPersonaOpen] = useState(false);
  const fileRef = useRef(null);
  const tempTimer = useRef(null);
  const rootRef = useRef(null);
  const panelRef = useRef(null);
  const scrimRef = useRef(null);
  const motionRef = useRef(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  // The panel is moved directly in the DOM (spring, rubber band, deformation): see drawer-motion.js.
  useLayoutEffect(() => {
    const panel = panelRef.current;
    // stagger: the bubbles of the current page arrive in a staircase, top to bottom, following the panel's progress
    const motion = new PanelMotion({
      panel,
      scrim: scrimRef.current,
      axis: 'x',
      closed: panel.offsetWidth + 24,
      onClosed: () => onCloseRef.current(),
      stagger: () => panel.querySelector('.mc-elastic-wrapper')?.children,
    });
    motionRef.current = motion;
    // if the finger was already pulling from the edge, the panel takes over the drag; otherwise it springs open
    if (!attachMotion(motion)) motion.open();
    return () => {
      detachMotion(motion);
      motion.destroy();
      motionRef.current = null;
    };
  }, []);
  const close = () => (motionRef.current ? motionRef.current.close() : onClose());
  usePanelDrag(rootRef, motionRef, { axis: 'x', enabled: !personaOpen });

  useLayer(page !== null, () => setPage(null));

  const preset = presets?.presets?.find((p) => p.id === chat.presetId);
  const thinkingOn = chat.thinkingEnabled ?? chat.settings?.thinkingEnabled !== false;

  async function save(patch) {
    setError('');
    try {
      const updated = await api.updateChat(chat.id, patch);
      onSaved(updated);
      return updated;
    } catch (e) {
      setError(e.message);
      return null;
    }
  }

  function changeTemp(v) {
    setTemp(v);
    clearTimeout(tempTimer.current);
    tempTimer.current = setTimeout(() => save({ temperature: v }), 400);
  }

  async function clearHistory() {
    if (!await confirm(tr("Apagar TODO o histórico desta conversa? Fica na lixeira por 30 dias e a memória derivada é invalidada."))) return;
    try {
      await api.clearHistory(chat.id);
      onHistoryCleared();
    } catch (e) {
      setError(e.message);
    }
  }

  async function deleteChat() {
    if (!await confirm(tr("Excluir \"{0}\"? Ela fica na lixeira por 30 dias.", [chat.name]))) return;
    try {
      await api.deleteChat(chat.id);
      onDeleted(chat.id);
    } catch (e) {
      setError(e.message);
    }
  }

  async function pickAvatar(file) {
    try {
      await save({ avatar: await fileToAvatar(file) });
    } catch (e) {
      setError(e.message);
    }
  }

  useEffect(() => {
    // already known and fresh: no request and no re-render while the panel is still sliding in
    if (api.peekGlobalSettings()) return;
    api.globalSettingsCached().then((r) => setGs(r.settings)).catch(() => {});
  }, []);

  const ctxPairs = chat.settings?.context?.preferredPairs ?? 30;

  return (
    <div ref={rootRef} className="mc-drawer">
      <div ref={scrimRef} className="mc-scrim" onClick={close} />
      <div ref={panelRef} className="mc-drawer-panel">
        {page === null && (
          <>
            <div className="mc-drawer-main-scroller min-h-0 flex-1 overflow-y-auto">
              <StairList className="min-h-full">
                <div className="mc-bubble mx-[10px] mb-[10px] mt-1 flex items-center gap-3 rounded-[30px] py-3 pl-3 pr-[18px]">
                  <button onClick={() => fileRef.current?.click()} className="relative shrink-0" aria-label={tr("Trocar avatar")}>
                    <GlassAvatar name={chat.name} avatar={chat.avatar} />
                    <span className="mc-tile-mini absolute -bottom-1 -right-1">
                      <Camera size={12} />
                    </span>
                  </button>
                  <div className="min-w-0">
                    <p className="truncate font-semibold">{chat.name}</p>
                    <p className="text-xs text-textMuted">{tr("Avatar guardado só neste aparelho e no PC")}</p>
                  </div>
                  <input ref={fileRef} type="file" accept="image/*" className="hidden" onChange={(e) => e.target.files?.[0] && pickAvatar(e.target.files[0])} />
                </div>
                <Row lead={<Cpu size={20} />} label={tr("Modelo e esforço")} value={`${preset?.label || chat.routeId}${chat.effort ? ` · ${chat.effort}` : ''}`} onClick={() => setPage('model')} />
                {preset?.thinkingCanDisable && <div className="mc-bubble mx-[10px] mb-[10px] rounded-[26px] px-[18px] py-3.5">
                  <button role="switch" aria-label={tr("Pensamento do modelo")} aria-checked={thinkingOn} disabled={busy || thinkingSaving} onClick={async () => { setThinkingSaving(true); try { await save({ settings: { thinkingEnabled: !thinkingOn } }); } finally { setThinkingSaving(false); } }} className="flex w-full items-center justify-between gap-3 text-left disabled:opacity-50">
                    <span><span className="block font-medium">{tr("Pensamento do modelo")}</span><span className="block text-xs text-textMuted">{thinkingSaving ? tr("Salvando…") : thinkingOn ? tr("Ligado") : tr("Desligado")}{tr(" · nesta conversa")}</span></span>
                    <Switch on={thinkingOn} />
                  </button>
                  <p className="mt-2 text-xs text-textMuted">{tr("Vale na próxima resposta. O esforço escolhido fica guardado para quando você ligar novamente.")}</p>
                </div>}
                {error && <p role="alert" className="mx-4 text-xs text-red-400">{tr(error)}</p>}
                <Row lead={<UserRound size={20} />} label={tr("Persona")} value={chat.persona || '—'} onClick={() => setPersonaOpen(true)} />
                <Row lead={<FileText size={20} />} label={tr("Personalização (.txt)")} value={chat.prompt?.trim() ? tr("definida") : tr("vazia")} onClick={() => setPage('prompt')} />
                <Row lead={<Users size={20} />} label={tr("Elenco")} value={tr("conferir e corrigir")} onClick={onOpenCast} />
                <Row lead={<Layers size={20} />} label={tr("Contexto")} value={tr("{0} pares", [ctxPairs])} onClick={() => setPage('context')} />
                <Row lead={<SlidersHorizontal size={20} />} label={tr("Estilo e limites (global)")} value={gs ? tr('{0}% falas · {1} car.', [gs.dialoguePct, Number(gs.botChars).toLocaleString(currentLanguage())]) : '—'} onClick={() => setPage('style')} />
                <Row lead={<History size={20} />} label={tr("Histórico")} value={tr("{0} msgs", [chat.history?.length || 0])} onClick={() => setPage('history')} />

                <AutoBubble className="mx-[10px] mb-[10px] rounded-[26px]">
                  <SamplingInfo preset={preset} chat={chat} temp={temp} thinkingOn={thinkingOn} onTemp={changeTemp} />
                </AutoBubble>
                {error && <p className="mx-4 mt-3 text-sm text-red-400">{tr(error)}</p>}
                <button onClick={onCreate} className="mc-new-chat-button mc-bubble mc-glass-accent mx-[10px] mt-1 flex items-center justify-center gap-2 overflow-hidden rounded-full py-[18px] font-semibold">
                  <MessageSquarePlus size={20} />{tr(" Novo Chat\n              ")}</button>
              </StairList>
            </div>
          </>
        )}

        {page === 'model' && (
          <ModelPage chat={chat} presets={presets} error={error} onBack={() => setPage(null)} onPick={async (patch) => (await save(patch)) && setPage(null)} />
        )}
        {personaOpen && (
          <PersonaManager
            chat={chat}
            onClose={() => setPersonaOpen(false)}
            onChosen={(updated) => {
              onSaved(updated);
              setPersonaOpen(false);
            }}
          />
        )}
        {page === 'persona' && (
          <TextPage title="Persona (seu nome na conversa)" initial={chat.persona || ''} placeholder="Ex.: Alex" onBack={() => setPage(null)} onSave={async (v) => (await save({ persona: v })) && setPage(null)} />
        )}
        {page === 'prompt' && <PromptPage chat={chat} onBack={() => setPage(null)} onSaved={onSaved} onClosePage={() => setPage(null)} />}
        {page === 'context' && (
          <SubPage title={tr("Contexto enviado ao modelo")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-3 rounded-[26px] px-4 py-4 text-sm text-textMuted">
              <p>{tr("\n                A conversa inteira fica guardada no PC. A cada resposta o modelo recebe: regras, personalização, persona, cena atual, quem sabe o quê, threads, até 8 memórias relevantes e os últimos ")}<b className="text-white">{ctxPairs}</b>{tr(" pares completos.\n              ")}</p>
              <label className="block">
                <span className="mb-1 flex justify-between">
                  <span>{tr("Pares recentes preferidos")}</span>
                  <span className="font-mono text-accentPurpleLight">{ctxPairs}</span>
                </span>
                <input
                  type="range" min="20" max="50" step="1" value={ctxPairs}
                  onChange={(e) => save({ settings: { context: { ...(chat.settings?.context || {}), preferredPairs: Number(e.target.value) } } })}
                  className="w-full accent-[#7c3aed]"
                />
              </label>
              <label className="flex items-center justify-between gap-3 rounded-[20px] bg-composer px-3.5 py-3 text-white">
                <span>{tr("\n                  Enviar regras na primeira mensagem\n                  ")}<span className="block text-xs text-textMuted">{tr("Ligado: inclui as regras do app na primeira mensagem enviada ao modelo, antes da conversa. Desligado: o app escolhe o formato compatível com a rota. Suas mensagens salvas não são alteradas.")}</span>
                </span>
                <input
                  type="checkbox"
                  checked={chat.settings?.rulesPlacement === 'first_turn'}
                  onChange={(e) => save({ settings: { rulesPlacement: e.target.checked ? 'first_turn' : null } })}
                />
              </label>
              <p>{tr("Alvo 96 mil tokens, limite suave 110 mil, teto 128 mil. Sob pressão, primeiro reduzo memórias e episódios; só depois os pares recentes (mínimo 20), sem cortar mensagens.")}</p>
            </div>
          </SubPage>
        )}
        {page === 'style' && (
          <SubPage title={tr("Estilo e limites (global)")} onBack={() => setPage(null)}>
            <StyleForm onChange={setGs} />
          </SubPage>
        )}
        {page === 'history' && (
          <SubPage title={tr("Histórico")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-2 rounded-[26px] px-4 py-3.5 text-sm text-textMuted">
              <p>{chat.history?.length || 0}{tr(" mensagens no caminho ativo, salvas no PC e em cache neste aparelho.")}</p>
              <p>{tr("Apagar e excluir vão para a lixeira por 30 dias (configurações do app → Lixeira).")}</p>
            </div>
            <div className="mt-[10px] space-y-[10px] px-[10px]">
              <button disabled={busy} onClick={clearHistory} className="mc-bubble flex w-full items-center justify-center gap-2 rounded-full py-4 font-semibold text-red-400 disabled:opacity-40">
                <Trash2 size={18} />{tr(" Apagar histórico desta conversa\n              ")}</button>
              <button
                disabled={busy}
                onClick={async () => {
                  try {
                    await api.duplicateChat(chat.id);
                    onClose();
                    onHistoryCleared();
                  } catch (e) {
                    setError(e.message);
                  }
                }}
                className="mc-bubble flex w-full items-center justify-center gap-2 rounded-full py-4 text-sm disabled:opacity-40"
              >
                <Copy size={16} />{tr(" Duplicar personagem em conversa limpa\n              ")}</button>
              <button disabled={busy} onClick={deleteChat} className="mc-bubble flex w-full items-center justify-center gap-2 rounded-full py-4 text-sm text-gray-300 disabled:opacity-40">{tr("\n                Excluir chat inteiro\n              ")}</button>
            </div>
            {error && <p className="mx-4 mt-3 text-sm text-red-400">{tr(error)}</p>}
          </SubPage>
        )}
      </div>
    </div>
  );
}

// How thinking is enabled for each preset (the server's "thinking" field).
const THINKING_TEXT = {
  enabled: 'pensamento ativado',
  adaptive: 'pensamento adaptativo ativado',
  level: 'pensamento ativado',
  always: 'pensamento sempre ativo',
  none: 'sem seleção de pensamento (o provedor usa o mínimo)',
};
const thinkingText = (p) => p.thinkingCanDisable ? 'pensamento: você pode ligar ou desligar' : THINKING_TEXT[p.thinking] || 'sem modo de pensamento';

function ModelPage({ chat, presets, error, onBack, onPick }) {
  useLanguage();
  const list = (presets?.presets || []).filter(p => p.selectable !== false && p.id !== 'generic');
  const savedThinking = chat.thinkingEnabled ?? chat.settings?.thinkingEnabled !== false;
  const [sel, setSel] = useState({ presetId: chat.presetId, routeId: chat.routeId, effort: chat.effort, thinkingEnabled: savedThinking });
  const cur = list.find((p) => p.id === sel.presetId);
  const dirty = sel.presetId !== chat.presetId || sel.routeId !== chat.routeId || sel.effort !== chat.effort || cur?.thinkingCanDisable && sel.thinkingEnabled !== savedThinking;
  const selectedPatch = () => ({ presetId: sel.presetId, routeId: sel.routeId, effort: sel.effort, ...(cur?.thinkingCanDisable ? { settings: { thinkingEnabled: sel.thinkingEnabled } } : {}) });

  function choosePreset(p) {
    const route = p.routes.find((r) => r.id === sel.routeId) || p.routes.find((r) => p.preferredRoutes?.includes(r.id)) || p.routes[0];
    if (!route) return;
    // presets with their own default (GLM 5.2 without thinking) start at that default; leaving them turns thinking back on
    const prev = list.find((x) => x.id === sel.presetId);
    const thinkingEnabled = p.id === sel.presetId ? sel.thinkingEnabled : p.thinkingDefault === false ? false : prev?.thinkingDefault === false ? true : sel.thinkingEnabled;
    setSel({ presetId: p.id, routeId: route.id, effort: p.effortByRoute?.[route.id] ?? p.defaultEffort, thinkingEnabled });
  }

  const routeVal = cur?.routes.find((r) => r.id === sel.routeId)?.validation;
  const verdict = sel.effort ? routeVal?.efforts?.[sel.effort]?.verdict || routeVal?.control || 'not_validated' : null;

  return (
    <SubPage
      title={tr("Modelo e esforço")}
      onBack={onBack}
      action={<button onClick={() => onPick(selectedPatch())} className="px-3 py-1 font-semibold text-accentPurpleLight">{tr("Salvar")}</button>}
      footer={dirty ? <button onClick={() => onPick(selectedPatch())} className="mc-bubble mc-glass-accent w-full rounded-full px-4 py-3.5 font-semibold">{tr("Usar ")}{cur?.label}</button> : null}
    >
      {list.length === 0 && <p className="px-5 text-sm text-textMuted">{tr("Carregando presets do PC…")}</p>}
      <p className="mc-bubble mx-[10px] mb-[10px] rounded-[22px] px-4 py-2.5 text-xs text-textMuted">{tr("Em uso: ")}{list.find(p => p.id === chat.presetId)?.label || chat.routeId}{tr(". Selecione abaixo e salve para aplicar às próximas respostas.")}</p>
      {error && <p role="alert" className="mx-5 mb-3 text-sm text-red-400">{tr(error)}</p>}
      {list.map((p) => {
        const usable = p.available !== false && p.routes.length > 0;
        return (
          <div key={p.id} className={`mc-bubble mx-[10px] mb-[10px] rounded-[26px] ${sel.presetId === p.id ? 'mc-selected' : ''}`}>
            <button disabled={!usable} onClick={() => choosePreset(p)} className="flex w-full items-center justify-between px-4 py-3 text-left disabled:opacity-50">
              <span>
                <span className="block font-medium">{p.label}</span>
                <span className="block text-xs text-textMuted">
                  {usable ? `${p.efforts.length ? tr("esforço padrão: {0}", [p.defaultEffort]) : tr("sem controle de esforço")} · ${tr(thinkingText(p))}${p.thinkingDefault === false ? tr(" (começa desligado)") : ''}` : tr('rota não exposta pelos provedores')}
                </span>
              </span>
              {sel.presetId === p.id && <Check size={18} className="text-accentPurpleLight" />}
            </button>
            {sel.presetId === p.id && (
              <div className="border-t border-borderDark px-4 pb-3 pt-2">
                {p.thinkingCanDisable ? <button role="switch" aria-label={tr("Pensamento do modelo selecionado")} aria-checked={sel.thinkingEnabled} onClick={() => setSel(s => ({ ...s, thinkingEnabled: !s.thinkingEnabled }))} className="mb-3 flex w-full items-center justify-between gap-3 rounded-full bg-composer px-3.5 py-2 text-left">
                  <span>{tr("Pensamento do modelo")}</span><Switch on={sel.thinkingEnabled} />
                </button> : <p className="mb-3 text-xs text-textMuted">{p.thinking === 'none' ? tr("Esta rota usa o menor pensamento disponível e não permite desligá-lo.") : tr("Controle de desligamento indisponível nesta rota.")}</p>}
                <p className="mb-1 text-xs text-textMuted">{tr("Rota exata")}</p>
                {p.routes.map((r) => (
                  <button key={r.id} onClick={() => setSel((s) => ({ ...s, routeId: r.id, effort: p.effortByRoute?.[r.id] ?? s.effort }))} className="flex w-full items-center gap-2 py-1.5 text-left text-sm">
                    <span className={`h-3.5 w-3.5 rounded-full border ${sel.routeId === r.id ? 'border-accentPurpleLight bg-accentPurple' : 'border-gray-500'}`} />
                    <span className="break-all">{r.id}</span>
                  </button>
                ))}
                {p.efforts.length > 0 && (
                  <>
                    <p className="mb-1 mt-3 text-xs text-textMuted">{p.thinkingCanDisable && !sel.thinkingEnabled ? tr("Esforço guardado para quando o pensamento estiver ligado") : p.effortByRoute ? 'Esforço (cada nível é uma rota própria)' : p.efforts.length > 1 ? tr("Esforço desta conversa") : tr("Esforço (fixo neste preset)")}</p>
                    <div className="flex flex-wrap gap-2">
                      {p.efforts.map((e) => {
                        if (!p.effortByRoute && p.efforts.length === 1) return <span key={e} className="mc-bubble mc-glass-accent rounded-full px-3 py-1.5 text-sm">{e}</span>;
                        if (!p.effortByRoute) return <button key={e} aria-pressed={sel.effort === e} onClick={() => setSel(s => ({ ...s, effort: e }))} className={`rounded-full px-3 py-1.5 text-sm text-white ${sel.effort === e ? 'mc-bubble mc-glass-accent' : 'bg-composer'}`}>{e}</button>;
                        const route = p.routes.find((r) => p.effortByRoute[r.id] === e);
                        const on = sel.effort === e;
                        return (
                          <button
                            key={e}
                            disabled={!route}
                            aria-pressed={on}
                            title={route ? route.id : tr('rota não exposta pelos provedores')}
                            onClick={() => route && setSel((s) => ({ ...s, routeId: route.id, effort: e }))}
                            className={`rounded-full px-3 py-1.5 text-sm text-white disabled:opacity-40 ${on ? 'mc-bubble mc-glass-accent' : 'bg-composer'}`}
                          >
                            {e}
                          </button>
                        );
                      })}
                    </div>
                    {verdict && (
                      <p className="mt-2 flex items-start gap-2 text-xs text-textMuted">
                        <span className="mt-1"><VerdictDot verdict={verdict} /></span>
                        <span>{tr(VERDICT_TEXT[verdict] || verdict)}</span>
                      </p>
                    )}
                  </>
                )}
              </div>
            )}
          </div>
        );
      })}
    </SubPage>
  );
}

// Glass bubble with the icon in a glass drop; label on top and the current value below (no line wrap).
function Row({ lead, label, value, onClick }) {
  useLanguage();
  return (
    <button onClick={onClick} className="mc-row">
      <span className="mc-tile">{lead}</span>
      <span className="min-w-0 flex-1 leading-tight">
        <span className="block truncate text-[15.5px]">{label}</span>
        {value && <span className="mt-0.5 block truncate text-[12.5px] text-textMuted">{value}</span>}
      </span>
      <ChevronRight size={18} className="shrink-0 text-textMuted" />
    </button>
  );
}

function Switch({ on }) {
  useLanguage();
  return (
    <span className={`mc-switch ${on ? 'mc-switch-on' : ''}`} aria-hidden="true">
      <span />
    </span>
  );
}

// Generative bubble: measures its content on every change (ResizeObserver) and animates its own height to it,
// growing or shrinking in real time; the content fades slightly while it changes.
function AutoBubble({ className = '', children }) {
  useLanguage();
  const outer = useRef(null);
  const inner = useRef(null);
  useLayoutEffect(() => {
    const box = outer.current;
    const content = inner.current;
    let timer = 0;
    box.style.height = `${content.offsetHeight}px`;
    const fit = () => {
      const h = `${content.offsetHeight}px`;
      if (box.style.height === h) return;
      box.style.height = h;
      box.classList.add('mc-changing');
      clearTimeout(timer);
      timer = setTimeout(() => box.classList.remove('mc-changing'), 200);
    };
    const ro = new ResizeObserver(fit);
    ro.observe(content);
    return () => {
      ro.disconnect();
      clearTimeout(timer);
    };
  }, []);
  return (
    <div ref={outer} className={`mc-bubble mc-auto-bubble ${className}`}>
      <div ref={inner} className="mc-auto-inner">{children}</div>
    </div>
  );
}

// Content of the generative bubble: what applies to the next request (model, thinking/effort, sampling) and,
// when the preset accepts it, the temperature. Changing the model, thinking or temperature resizes the bubble.
function SamplingInfo({ preset, chat, temp, thinkingOn, onTemp }) {
  useLanguage();
  const usesTemp = preset?.sampling === 'temperature';
  const facts = [
    preset?.label || chat.routeId,
    preset?.thinkingCanDisable && !thinkingOn ? tr('sem pensamento') : chat.effort ? tr('esforço {0}', [chat.effort]) : null,
    usesTemp ? tr('temperatura {0}', [Number(temp).toLocaleString(currentLanguage(), { minimumFractionDigits: 2, maximumFractionDigits: 2 })]) : tr('amostragem da rota'),
  ].filter(Boolean);
  return (
    <div className="px-[18px] py-4">
      <div className="flex flex-wrap gap-1.5">
        {facts.map((f) => (
          <span key={f} className="rounded-full bg-composer px-2.5 py-0.5 text-[11.5px] text-gray-300">{f}</span>
        ))}
      </div>
      {usesTemp ? (
        <div className="mt-3">
          <div className="mb-2 flex items-center justify-between">
            <span className="font-medium">{tr("Temperatura")}</span>
            <span className="rounded-full bg-composer px-2.5 py-0.5 font-mono text-sm text-accentPurpleLight">{Number(temp).toLocaleString(currentLanguage(), { minimumFractionDigits: 2, maximumFractionDigits: 2 })}</span>
          </div>
          <input type="range" min="0" max="2" step="0.05" value={temp} onChange={(e) => onTemp(Number(e.target.value))} className="w-full accent-[#7c3aed]" aria-label={tr("Temperatura")} />
        </div>
      ) : (
        <p className="mt-2 text-xs leading-relaxed text-textMuted">{tr("\n          Este preset usa a amostragem padrão da rota; a temperatura anterior\n          ")}{chat.temperature != null ? ` (${Number(chat.temperature).toFixed(2)})` : ''}{tr(" fica guardada, mas não é enviada.\n        ")}</p>
      )}
    </div>
  );
}

// Glass avatar: photo with a glass rim, or the initial in a translucent tile (no colorful gradient).
function GlassAvatar({ name, avatar }) {
  useLanguage();
  if (avatar) return <Avatar name={name} avatar={avatar} size={56} />;
  const initial = [...(name || '?').trim()][0]?.toUpperCase() || '?';
  return <span className="mc-tile mc-tile-lg text-[22px] font-semibold">{initial}</span>;
}

function SubPage({ title, onBack, children, action, footer }) {
  useLanguage();
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="relative z-10 shrink-0 px-[10px] pb-[10px] pt-1">
        <div className="mc-bubble flex h-11 items-center gap-2 rounded-full px-3">
          <button onClick={onBack} className="p-1" aria-label={tr("Voltar")}><ChevronLeft size={22} /></button>
          <h2 className="flex-1 truncate text-[15px] font-semibold">{title}</h2>
          {action}
        </div>
      </div>
      <div className="mc-subpage-scroller min-h-0 flex-1 overflow-y-auto">
        <StairList className="min-h-full pt-1">
          {children}
        </StairList>
      </div>
      {footer && <div className="mc-subfooter">{footer}</div>}
    </div>
  );
}

function TextPage({ title, initial, placeholder, onBack, onSave }) {
  useLanguage();
  const [v, setV] = useState(initial);
  return (
    <SubPage title={title} onBack={onBack} action={<button onClick={() => onSave(v)} className="px-3 py-1 font-semibold text-accentPurpleLight">{tr("Salvar")}</button>}>
      <div className="mc-bubble mx-[10px] rounded-[30px] p-2">
        <input
          autoFocus
          value={v}
          placeholder={placeholder}
          spellCheck={true}

          autoCorrect="on"
          autoCapitalize="sentences"
          onChange={(e) => setV(e.target.value)}
          className="w-full rounded-full bg-composer px-4 py-3 text-[16px] outline-none"
        />
      </div>
    </SubPage>
  );
}

function PromptPage({ chat, onBack, onSaved, onClosePage }) {
  useLanguage();
  const [v, setV] = useState(chat.prompt || '');
  const [conflict, setConflict] = useState(null);
  const [error, setError] = useState('');
  const fileRef = useRef(null);

  async function submit(force = false) {
    setError('');
    try {
      const updated = await api.updateChat(chat.id, { prompt: v, baseCharacterRev: force ? undefined : chat.characterRev });
      onSaved(updated);
      onClosePage();
    } catch (e) {
      if (e.code === 'CHARACTER_CONFLICT') setConflict(e.data.current);
      else setError(e.message);
    }
  }

  return (
    <SubPage title={tr("Personalização")} onBack={onBack} action={<button onClick={() => submit(false)} className="px-3 py-1 font-semibold text-accentPurpleLight">{tr("Salvar")}</button>}>
      {error && <p role="alert" className="mx-4 mb-3 text-sm text-red-300">{tr(error)}</p>}
      <div className="mc-bubble mx-[10px] mb-2 flex h-full flex-col rounded-[26px] px-3 pb-3 pt-3">
        <p className="mb-2 text-xs text-textMuted">{tr("\n          Características, regras de mundo e personalidade. A ficha fica criptografada no PC. Para editá-la por fora, importe um arquivo .txt; cada alteração cria uma nova revisão.\n        ")}</p>
        {conflict && (
          <div className="mb-2 rounded-xl bg-yellow-500/10 p-3 text-sm text-yellow-200">{tr("\n            O texto foi alterado em outro lugar (revisão ")}{conflict.characterRev}).
            <div className="mt-2 flex gap-2">
              <button className="rounded-full bg-composer px-3 py-1.5" onClick={() => { setV(conflict.prompt); setConflict(null); }}>{tr("Carregar a versão do PC")}</button>
              <button className="rounded-full bg-composer px-3 py-1.5" onClick={() => submit(true)}>{tr("Sobrescrever")}</button>
            </div>
          </div>
        )}
        <textarea
          value={v}
          onChange={(e) => setV(e.target.value)}
          placeholder={'Ex.:\nVocê é Mira, estudante da Academia de Magia...\nPersonalidade: ...\nCenário: ...'}
          spellCheck={true}

          autoCorrect="on"
          autoCapitalize="sentences"
          className="h-[50dvh] min-h-[180px] w-full shrink-0 resize-none overflow-y-auto rounded-[20px] bg-composer p-3 text-[16px] leading-relaxed outline-none"
        />
        <input ref={fileRef} type="file" accept=".txt,text/plain" className="hidden" onChange={async (e) => { const f = e.target.files?.[0]; if (f) setV(await f.text()); e.target.value = ''; }} />
        <button onClick={() => fileRef.current?.click()} className="mt-3 flex items-center justify-center gap-2 rounded-full bg-composer py-3 text-sm text-gray-300">
          <Upload size={16} />{tr(" Importar arquivo .txt\n        ")}</button>
      </div>
    </SubPage>
  );
}
