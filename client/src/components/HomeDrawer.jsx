import React, { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  ChevronRight, ChevronLeft, Trash2, MessageSquarePlus, Upload, Download, Check, Users,
  SlidersHorizontal, Smartphone, Type, Eye, Sparkles, Languages, RotateCcw, Database, Link2Off, KeyRound
} from 'lucide-react';
import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage, setLanguage, currentLanguage } from '../i18n.js';
import { api, config, isNative } from '../api.js';
import { useLayer } from '../nav.js';
import { PRESETS, applyEyeCare, loadEyeCare, saveEyeCare, colorsFor } from '../eyecare.js';
import { APP_ICONS, getSelectedAppIcon, applyAppIcon, initAppIcon } from '../app-icon.js';
import { PanelMotion, attachMotion, detachMotion } from '../drawer-motion.js';
import { usePanelDrag } from '../gestures.js';
import StyleForm from './StyleForm.jsx';
import ChoiceField from './ChoiceField.jsx';
import ColorField from './ColorField.jsx';
import PersonaManager from './PersonaManager.jsx';
import PairedDevices from './PairedDevices.jsx';
import { StairList, Row, SubPage, Switch } from './SettingsDrawer.jsx';
import '../drawer.css';

const LocalHostSettings = lazy(() => import('./LocalHostSettings.jsx'));

export default function HomeDrawer({ hostStatus, fontSize, onFontSize, onClose, onCreate, onChanged, onResetServer }) {
  const confirm = useConfirm();
  const language = useLanguage();
  const [page, setPage] = useState(null); // null | style | reading | eye | lang | icon | pairing | backup | trash
  const [gs, setGs] = useState(() => api.peekGlobalSettings());
  const [eye, setEye] = useState(loadEyeCare);
  const [appIcon, setAppIcon] = useState(getSelectedAppIcon);
  const [iconMsg, setIconMsg] = useState('');
  const [personasOpen, setPersonasOpen] = useState(false);
  const [localHostOpen, setLocalHostOpen] = useState(false);
  const [code, setCode] = useState(null);
  const [trash, setTrash] = useState(null);
  const [trashError, setTrashError] = useState('');
  const [msg, setMsg] = useState('');
  const fileRef = useRef(null);

  const rootRef = useRef(null);
  const panelRef = useRef(null);
  const scrimRef = useRef(null);
  const motionRef = useRef(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useLayoutEffect(() => {
    const panel = panelRef.current;
    const motion = new PanelMotion({
      panel,
      scrim: scrimRef.current,
      axis: 'x',
      closed: panel.offsetWidth + 24,
      onClosed: () => onCloseRef.current(),
      stagger: () => panel.querySelector('.mc-elastic-wrapper')?.children,
    });
    motionRef.current = motion;
    if (!attachMotion(motion)) motion.open();
    return () => {
      detachMotion(motion);
      motion.destroy();
      motionRef.current = null;
    };
  }, []);

  const close = () => (motionRef.current ? motionRef.current.close() : onClose());
  usePanelDrag(rootRef, motionRef, { axis: 'x', enabled: !personasOpen && !localHostOpen });
  useLayer(page !== null, () => setPage(null));

  useEffect(() => {
    initAppIcon().then(setAppIcon).catch(() => {});
    if (!api.peekGlobalSettings()) {
      api.globalSettingsCached().then((r) => setGs(r.settings)).catch(() => {});
    }
  }, []);

  async function handleCreate() {
    if (!await confirm(tr("Deseja criar uma nova conversa?"))) return;
    close();
    onCreate();
  }

  function changeEye(patch) {
    const next = { ...eye, ...patch };
    setEye(next);
    saveEyeCare(next);
    applyEyeCare(next);
  }

  async function changeAppIcon(id) {
    setAppIcon(id);
    setIconMsg('');
    try {
      const r = await applyAppIcon(id);
      setAppIcon(r.id);
      if (r.pending) setIconMsg(tr("Pronto. O ícone da tela inicial muda assim que você sair do app (pode levar alguns segundos no lançador)."));
    } catch (e) {
      setIconMsg(tr("Não foi possível trocar o ícone: {0}", [e.message || e]));
    }
  }

  async function loadTrash() {
    try {
      setTrash(await api.trash());
      setTrashError('');
    } catch (e) {
      setTrashError(e.message || 'Não foi possível carregar a lixeira.');
    }
  }

  async function runAction(fn, ok) {
    setMsg('');
    try {
      const r = await fn();
      if (ok) setMsg(typeof ok === 'function' ? ok(r) : ok);
      onChanged?.();
      return r;
    } catch (e) {
      setMsg(e.message);
    }
  }

  async function exportAll() {
    const data = await api.exportAll();
    const blob = new Blob([JSON.stringify(data)], { type: 'application/json' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = 'modularchat-backup.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  }

  const eyePreview = colorsFor(eye);

  return (
    <div ref={rootRef} className="mc-drawer">
      <div ref={scrimRef} className="mc-scrim" onClick={close} />
      <div ref={panelRef} className="mc-drawer-panel">
        {page === null && (
          <div className="mc-drawer-main-scroller min-h-0 flex-1 overflow-y-auto">
            <StairList className="min-h-full">
              {/* Botão Novo Chat com animação idêntica ao menu lateral do chat */}
              <button
                onClick={handleCreate}
                className="mc-new-chat-button mc-bubble mc-glass-accent mx-[10px] mt-1 flex items-center justify-center gap-2 overflow-hidden rounded-full py-[18px] font-semibold"
              >
                <MessageSquarePlus size={20} />{tr(" Novo Chat")}
              </button>

              <Row
                lead={<SlidersHorizontal size={20} />}
                label={tr("Estilo e limites (global)")}
                value={gs ? tr('{0}% falas · {1} car.', [gs.dialoguePct, Number(gs.botChars).toLocaleString(currentLanguage())]) : '—'}
                onClick={() => setPage('style')}
              />

              <Row
                lead={<Type size={20} />}
                label={tr("Leitura")}
                value={`${fontSize}px`}
                onClick={() => setPage('reading')}
              />

              <Row
                lead={<Eye size={20} />}
                label={tr("Proteção ocular")}
                value={eye.preset === 'custom' ? tr('personalizada') : tr(PRESETS[eye.preset]?.label || 'neutro')}
                onClick={() => setPage('eye')}
              />

              <Row
                lead={<Languages size={20} />}
                label={tr("Idioma")}
                value={language === 'pt-BR' ? 'Português' : 'English'}
                onClick={() => setPage('lang')}
              />

              <Row
                lead={<Sparkles size={20} />}
                label={tr("Ícone do aplicativo")}
                value={APP_ICONS.find(i => i.id === appIcon)?.name || 'Pierogi'}
                onClick={() => setPage('icon')}
              />

              <Row
                lead={<Users size={20} />}
                label={tr("Perfis de persona")}
                value={tr("gerenciar")}
                onClick={() => setPersonasOpen(true)}
              />

              <Row
                lead={<Smartphone size={20} />}
                label={tr("Pareamento / Conectar PC")}
                value={config.kind === 'local' ? tr('Host local') : config.server ? tr('Conectado') : tr('Configurar')}
                onClick={() => setPage('pairing')}
              />

              {isNative() && config.embeddedReady && (
                <Row
                  lead={<Smartphone size={20} />}
                  label={tr("Host Android")}
                  value={tr("provedores e memória")}
                  onClick={() => setLocalHostOpen(true)}
                />
              )}

              <Row
                lead={<Database size={20} />}
                label={tr("Backup e exportação")}
                value={tr("salvar ou importar")}
                onClick={() => setPage('backup')}
              />

              <Row
                lead={<RotateCcw size={20} />}
                label={tr("Lixeira")}
                value={tr("recuperar conversas")}
                onClick={() => {
                  loadTrash();
                  setPage('trash');
                }}
              />

              {/* Status do Host ativo no fim do menu lateral, conforme solicitado pelo usuário */}
              {hostStatus && <div className="my-2">{hostStatus}</div>}
            </StairList>
          </div>
        )}

        {page === 'style' && (
          <SubPage title={tr("Estilo e limites (global)")} onBack={() => setPage(null)}>
            <StyleForm onChange={setGs} />
          </SubPage>
        )}

        {page === 'reading' && (
          <SubPage title={tr("Leitura")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-4 rounded-[26px] p-4 text-sm">
              <div className="flex items-center justify-between">
                <span>{tr("Tamanho da letra")}</span>
                <span className="font-mono text-accentPurpleLight">{fontSize}px</span>
              </div>
              <input
                type="range" min="14" max="22" step="1"
                value={fontSize}
                onChange={(e) => onFontSize(Number(e.target.value))}
                className="w-full accent-[#7c3aed]"
              />
              <p className="mc-read rounded-xl bg-composer p-3 text-sm">
                <em className="mc-action">{tr('*Sobre a porta, as letras escuras parecem arder.*')}</em><br />
                <span className="mc-name">Dante:</span> Lasciate ogne speranza, voi ch’intrate.
              </p>
            </div>
          </SubPage>
        )}

        {page === 'eye' && (
          <SubPage title={tr("Proteção ocular")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-4 rounded-[26px] p-4 text-sm">
              <label className="block">
                <span className="flex items-center justify-between">
                  <span>{tr("Brilho das letras")}</span>
                  <span className="font-mono text-accentPurpleLight">{eye.brightness}%</span>
                </span>
                <input
                  type="range" min="50" max="100" step="1"
                  value={eye.brightness}
                  onChange={(e) => changeEye({ brightness: Number(e.target.value) })}
                  className="w-full accent-[#7c3aed]"
                />
              </label>

              <div>
                <p className="mb-1.5 text-sm">{tr("Cor das letras")}</p>
                <div className="flex flex-wrap gap-2">
                  {Object.entries(PRESETS).map(([id, p]) => (
                    <button
                      key={id}
                      onClick={() => changeEye({ preset: id })}
                      aria-pressed={eye.preset === id}
                      className={`flex items-center gap-2 rounded-full px-3 py-1.5 text-sm ${eye.preset === id ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}
                    >
                      <span className="h-3.5 w-3.5 rounded-full border border-white/20" style={{ background: p.text }} />
                      {tr(p.label)}
                    </button>
                  ))}
                  <ColorField value={eye.custom} active={eye.preset === 'custom'} onChange={custom => changeEye({ preset: 'custom', custom })} />
                </div>
              </div>

              <label className="block">
                <span className="flex items-center justify-between">
                  <span>{tr("Filtro de luz azul")}</span>
                  <span className="font-mono text-accentPurpleLight">{eye.blue}%</span>
                </span>
                <input
                  type="range" min="0" max="60" step="1"
                  value={eye.blue}
                  onChange={(e) => changeEye({ blue: Number(e.target.value) })}
                  className="w-full accent-[#7c3aed]"
                />
              </label>

              <label className="block">
                <span className="flex items-center justify-between">
                  <span>{tr("Textura de papel")}</span>
                  <span className="font-mono text-accentPurpleLight">{eye.paper}%</span>
                </span>
                <input
                  type="range" min="0" max="100" step="1"
                  value={eye.paper}
                  onChange={(e) => changeEye({ paper: Number(e.target.value) })}
                  className="w-full accent-[#7c3aed]"
                />
              </label>

              <p className="mc-read rounded-xl bg-composer p-3" style={{ color: eyePreview.text }}>
                <em style={{ color: eyePreview.action }}>{tr('*Sobre a porta, as letras escuras parecem arder.*')}</em><br />
                <span style={{ color: eyePreview.name, fontWeight: 600 }}>Dante:</span> Lasciate ogne speranza, voi ch’intrate.
              </p>

              <button
                onClick={() => changeEye({ preset: 'neutro', custom: '#e7e7ea', brightness: 100, blue: 0, paper: 0 })}
                className="rounded-full bg-composer px-4 py-2 text-sm"
              >
                {tr("Restaurar padrão")}
              </button>
            </div>
          </SubPage>
        )}

        {page === 'lang' && (
          <SubPage title={tr("Idioma")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-3 rounded-[26px] p-4 text-sm">
              <ChoiceField
                variant="dropdown"
                label={tr('Idioma da interface')}
                value={language}
                onChange={setLanguage}
                options={[{ value: 'pt-BR', label: 'Português (Brasil)', lang: 'pt-BR' }, { value: 'en', label: 'English', lang: 'en' }]}
              />
              <p className="text-xs text-textMuted">{tr('A escolha fica salva neste aparelho ou navegador. Não muda o idioma das conversas nem das respostas do modelo.')}</p>
            </div>
          </SubPage>
        )}

        {page === 'icon' && (
          <SubPage title={tr("Ícone do aplicativo")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] rounded-[26px] p-4">
              <div className="grid grid-cols-2 gap-3 pt-1">
                {APP_ICONS.map((ic) => {
                  const active = appIcon === ic.id;
                  return (
                    <button
                      key={ic.id}
                      type="button"
                      onClick={() => changeAppIcon(ic.id)}
                      aria-pressed={active}
                      className="mc-bubble flex flex-col items-center gap-2.5 rounded-[26px] px-3 pb-3 pt-4 text-center"
                    >
                      <img src={ic.tile} alt="" className="h-[76px] w-[76px] drop-shadow-[0_8px_14px_rgba(4,3,20,.65)]" />
                      <span className="min-w-0 w-full">
                        <span className="block truncate text-[13px] font-semibold">{ic.name}</span>
                        <span className="block truncate text-[11px] text-textMuted">{active ? tr("Em uso") : tr(ic.subtitle)}</span>
                      </span>
                    </button>
                  );
                })}
              </div>
              {iconMsg && <p className="mt-3 text-xs text-textMuted">{tr(iconMsg)}</p>}
            </div>
          </SubPage>
        )}

        {page === 'pairing' && (
          <SubPage title={tr("Pareamento")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-3 rounded-[26px] p-4 text-sm">
              {isNative() ? (
                <>
                  <p className="text-sm text-textMuted">{tr("Servidor ativo: ")}{config.kind === 'local' ? tr('Aparelho Android') : config.server || '—'}</p>
                  <p className="text-sm text-textMuted">{tr('Rede local:')} {config.connection.local || tr('não configurada')}</p>
                  <p className="text-sm text-textMuted">{tr('Internet:')} {config.connection.remote || tr('não configurada')}</p>
                  <button
                    onClick={() => {
                      close();
                      onResetServer();
                    }}
                    className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"
                  >
                    <Link2Off size={16} />{tr("Conectar PC / trocar pareamento")}
                  </button>
                </>
              ) : (
                <>
                  <button
                    onClick={() => runAction(async () => setCode(await api.startPairing()), tr("Código gerado (vale 5 min)."))}
                    className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"
                  >
                    <KeyRound size={16} />{tr(" Gerar código de pareamento")}
                  </button>
                  {code && <p className="rounded-xl bg-composer p-3 text-center font-mono text-[28px] tracking-[0.3em]">{code.code}</p>}
                </>
              )}

              <PairedDevices />
              {msg && <p className="text-xs text-accentPurpleLight">{msg}</p>}
            </div>
          </SubPage>
        )}

        {page === 'backup' && (
          <SubPage title={tr("Backup e exportação")} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-3 rounded-[26px] p-4 text-sm">
              <div className="flex flex-wrap gap-2">
                <button onClick={() => runAction(api.backup, (r) => tr("Cópia do banco salva no servidor ativo: {0}", [r.file]))} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Database size={16} />{tr("Backup no servidor ativo")}</button>
                <button onClick={() => runAction(exportAll, tr("Exportação baixada."))} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Download size={16} />{tr(" Exportar tudo")}</button>
                <button onClick={() => fileRef.current?.click()} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Upload size={16} />{tr(" Importar")}</button>
                <input
                  ref={fileRef} type="file" accept="application/json,.json" className="hidden"
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    e.target.value = '';
                    if (f) runAction(async () => api.importData(JSON.parse(await f.text())), (r) => tr("{0} conversa(s) importada(s){1}.", [r.imported.length, r.errors.length ? tr(", {0} com erro", [r.errors.length]) : '']));
                  }}
                />
              </div>
              {msg && <p className="text-xs text-accentPurpleLight">{msg}</p>}
            </div>
          </SubPage>
        )}

        {page === 'trash' && (
          <SubPage title={tr("Lixeira ({0} dias)", [trash?.days || 30])} onBack={() => setPage(null)}>
            <div className="mc-bubble mx-[10px] space-y-3 rounded-[26px] p-4 text-sm">
              {trashError && <p className="text-sm text-red-300">{trashError}</p>}
              {trash?.items?.length === 0 && <p className="text-textMuted">{tr("A lixeira está vazia.")}</p>}
              {trash?.items?.map((item) => (
                <div key={item.id} className="flex items-center justify-between rounded-xl bg-composer px-3 py-2 text-sm">
                  <span className="truncate pr-2">{item.name || item.id}</span>
                  <button
                    onClick={() => runAction(() => api.restoreTrash(item), tr("Item restaurado.")).then(loadTrash)}
                    className="shrink-0 text-accentPurpleLight font-semibold"
                  >
                    {tr("Restaurar")}
                  </button>
                </div>
              ))}
            </div>
          </SubPage>
        )}

        {personasOpen && (
          <PersonaManager
            onClose={() => setPersonasOpen(false)}
            onChosen={() => setPersonasOpen(false)}
          />
        )}

        {localHostOpen && (
          <Suspense fallback={null}>
            <LocalHostSettings onClose={() => setLocalHostOpen(false)} />
          </Suspense>
        )}
      </div>
    </div>
  );
}
