import { tr, useLanguage, setLanguage } from '../i18n.js';
import React, { useEffect, useRef, useState } from 'react';
import { Download, Upload, Database, KeyRound, Trash2, RotateCcw, Smartphone, Link2Off, Type, Eye, Sparkles, Languages } from 'lucide-react';
import { PRESETS, applyEyeCare, loadEyeCare, saveEyeCare, colorsFor } from '../eyecare.js';
import { api, config, isNative } from '../api.js';
import { APP_ICONS, getSelectedAppIcon, applyAppIcon, initAppIcon } from '../app-icon.js';
import PersonaManager from './PersonaManager.jsx';
import { Screen } from './Sheet.jsx';
import ChoiceField from './ChoiceField.jsx';
import ColorField from './ColorField.jsx';
import { useConfirm } from './ConfirmProvider.jsx';

// App settings: reading, pairing, backup/import, trash and global cleanup.
export default function GlobalSettings({ fontSize, onFontSize, onClose, onChanged, onResetServer }) {
  const language = useLanguage();
  const confirm = useConfirm();
  const [code, setCode] = useState(null);
  const [devices, setDevices] = useState(null);
  const [trash, setTrash] = useState(null); // null = loading (or failed: see trashError)
  const [trashError, setTrashError] = useState('');
  const [eye, setEye] = useState(loadEyeCare);
  const [status, setStatus] = useState(null);
  const [msg, setMsg] = useState('');
  const [personas, setPersonas] = useState(false);
  const [appIcon, setAppIcon] = useState(getSelectedAppIcon);
  const [iconMsg, setIconMsg] = useState('');
  const fileRef = useRef(null);

  useEffect(() => {
    initAppIcon().then(setAppIcon).catch(() => {});
  }, []);

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

  async function load() {
    api.devices().then((r) => setDevices(r.devices)).catch(() => setDevices(null));
    loadTrash();
    api.status().then(setStatus).catch(() => setStatus(null));
  }
  // A network/server error must NEVER look like an "empty trash": show the error and allow retrying.
  async function loadTrash() {
    try {
      setTrash(await api.trash());
      setTrashError('');
    } catch (e) {
      setTrashError(e.message || 'Não foi possível carregar a lixeira.');
    }
  }
  useEffect(() => {
    load();
  }, []);

  function changeEye(patch) {
    const next = { ...eye, ...patch };
    setEye(next);
    saveEyeCare(next);
    applyEyeCare(next);
  }

  async function run(fn, ok) {
    setMsg('');
    try {
      const r = await fn();
      if (ok) setMsg(typeof ok === 'function' ? ok(r) : ok);
      load();
      onChanged();
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

  const itemLabel = (t) => (t.type === 'conversation' ? tr('Conversa “{0}”{1}', [t.name, t.count ? tr(' · {0} mensagem(ns)', [t.count]) : '']) : t.type === 'interactions' ? tr('{0} mensagem(ns) apagada(s) em “{1}”', [t.count, t.name]) : tr('{0} resposta(s) apagada(s) em “{1}”', [t.count, t.name]));
  const daysLeft = (t) => Math.max(0, Math.ceil(((trash?.days || 30) * 86_400_000 - (Date.now() - new Date(t.deletedAt).getTime())) / 86_400_000));
  const preview = colorsFor(eye);

  return (
    <Screen title={tr("Configurações")} onClose={onClose}>
      <div className="space-y-5 px-4 py-4 text-[15px]">
        <Section title={tr('Idioma')} icon={<Languages size={16} />}>
          <p className="text-sm text-textMuted">{tr('Idioma da interface')}</p>
          <ChoiceField label={tr('Idioma da interface')} value={language} onChange={setLanguage}
            options={[{ value: 'pt-BR', label: 'Português (Brasil)', lang: 'pt-BR' }, { value: 'en', label: 'English', lang: 'en' }]} />
          <p className="text-xs text-textMuted">{tr('A escolha fica salva neste aparelho ou navegador. Não muda o idioma das conversas nem das respostas do modelo.')}</p>
        </Section>
        <Section title={tr("Leitura")} icon={<Type size={16} />}>
          <div className="flex items-center justify-between">
            <span>{tr("Tamanho da letra")}</span>
            <span className="font-mono text-accentPurpleLight">{fontSize}{tr("px")}</span>
          </div>
          <input type="range" min="14" max="22" step="1" value={fontSize} onChange={(e) => onFontSize(Number(e.target.value))} className="w-full accent-[#7c3aed]" />
          <p className="mc-read rounded-xl bg-composer p-3"><em className="mc-action">{tr('*Sobre a porta, as letras escuras parecem arder.*')}</em><br /><span className="mc-name">Dante:</span> Lasciate ogne speranza, voi ch’intrate.</p>
        </Section>

        <Section title={tr("Proteção ocular")} icon={<Eye size={16} />}>
          <label className="block">
            <span className="flex items-center justify-between">
              <span>{tr("Brilho das letras")}</span>
              <span className="font-mono text-accentPurpleLight">{eye.brightness}%</span>
            </span>
            <input type="range" min="50" max="100" step="1" value={eye.brightness} onChange={(e) => changeEye({ brightness: Number(e.target.value) })} className="w-full accent-[#7c3aed]" aria-label={tr("Brilho das letras")} />
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
            <input type="range" min="0" max="60" step="1" value={eye.blue} onChange={(e) => changeEye({ blue: Number(e.target.value) })} className="w-full accent-[#7c3aed]" aria-label={tr("Filtro de luz azul")} />
            <span className="mt-0.5 block text-xs text-textMuted">{tr("Esquenta as cores do app (fundos, letras e destaques), reduzindo o azul.")}</span>
          </label>
          <label className="block">
            <span className="flex items-center justify-between">
              <span>{tr("Textura de papel")}</span>
              <span className="font-mono text-accentPurpleLight">{eye.paper}%</span>
            </span>
            <input type="range" min="0" max="100" step="1" value={eye.paper} onChange={(e) => changeEye({ paper: Number(e.target.value) })} className="w-full accent-[#7c3aed]" aria-label={tr("Textura de papel")} />
            <span className="mt-0.5 block text-xs text-textMuted">{tr("Granulação sutil por cima da tela, que lembra a aspereza do papel e tira o brilho liso do vidro.")}</span>
          </label>
          <p className="mc-read rounded-xl bg-composer p-3" style={{ color: preview.text }}>
            <em style={{ color: preview.action }}>{tr('*Sobre a porta, as letras escuras parecem arder.*')}</em><br /><span style={{ color: preview.name, fontWeight: 600 }}>Dante:</span> Lasciate ogne speranza, voi ch’intrate.
          </p>
          <button onClick={() => changeEye({ preset: 'neutro', custom: '#e7e7ea', brightness: 100, blue: 0, paper: 0 })} className="rounded-full bg-composer px-4 py-2 text-sm">{tr("Restaurar padrão")}</button>
        </Section>

        <Section title={tr("Ícone do aplicativo")} icon={<Sparkles size={16} />}>
          <p className="text-sm text-textMuted">{tr("Escolha o ícone do aplicativo:")}</p>
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
          {iconMsg && <p className="text-xs text-textMuted">{tr(iconMsg)}</p>}
        </Section>

        <Section title={tr("Perfis de persona")} icon={<Type size={16} />}>
          <p className="text-sm text-textMuted">{tr("Crie perfis para quem você interpreta e troque entre eles em cada conversa (⋯ → Persona).")}</p>
          <button onClick={() => setPersonas(true)} className="rounded-full bg-composer px-4 py-2.5 text-sm">{tr("Gerenciar perfis")}</button>
        </Section>

        <Section title={tr("Pareamento")} icon={<Smartphone size={16} />}>
          {isNative() ? (
            <>
              <p className="text-sm text-textMuted">{tr("Servidor: ")}{config.server || '—'}</p>
              <p className="text-sm text-textMuted">{tr('Rede local:')} {config.connection.local || tr('não configurada')}</p>
              <p className="text-sm text-textMuted">{tr('Internet:')} {config.connection.remote || tr('não configurada')}</p>
              <p className="text-xs text-textMuted">{tr('O app prefere a rede local e reavalia a conexão ao voltar ou quando a rede muda.')}</p>
              <button onClick={onResetServer} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Link2Off size={16} />{tr(" Trocar PC / refazer pareamento")}</button>
            </>
          ) : (
            <>
              <button onClick={() => run(async () => setCode(await api.startPairing()), tr("Código gerado (vale 5 min)."))} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><KeyRound size={16} />{tr(" Gerar código de pareamento")}</button>
              {code && <p className="rounded-xl bg-composer p-3 text-center font-mono text-[28px] tracking-[0.3em]">{code.code}</p>}
            </>
          )}
          {devices?.filter((d) => !d.revoked_at).map((d) => (
            <div key={d.id} className="flex items-center justify-between rounded-xl bg-composer px-3 py-2 text-sm">
              <span>{d.name}<span className="block text-[11px] text-textMuted">{tr("visto ")}{d.last_seen_at ? new Date(d.last_seen_at).toLocaleString() : tr("nunca")}</span></span>
              <button onClick={() => run(() => api.revokeDevice(d.id), tr("Aparelho removido."))} aria-label={tr("Remover aparelho")}><Trash2 size={16} className="text-red-300" /></button>
            </div>
          ))}
          <p className="text-xs text-textMuted">{tr('As chaves dos provedores ficam só no PC; o aparelho guarda apenas um token do ModularChat.')}</p>
        </Section>

        <Section title={tr("Backup")} icon={<Database size={16} />}>
          <div className="flex flex-wrap gap-2">
            <button onClick={() => run(api.backup, (r) => tr("Cópia do banco salva no PC: {0}", [r.file]))} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Database size={16} />{tr(" Backup no PC")}</button>
            <button onClick={() => run(exportAll, tr("Exportação baixada."))} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Download size={16} />{tr(" Exportar tudo")}</button>
            <button onClick={() => fileRef.current?.click()} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Upload size={16} />{tr(" Importar")}</button>
            <input
              ref={fileRef} type="file" accept="application/json,.json" className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = '';
                if (f) run(async () => api.importData(JSON.parse(await f.text())), (r) => tr("{0} conversa(s) importada(s){1}.", [r.imported.length, r.errors.length ? tr(", {0} com erro", [r.errors.length]) : '']));
              }}
            />
          </div>
        </Section>

        <Section title={tr("Lixeira ({0} dias)", [trash?.days || 30])} icon={<RotateCcw size={16} />}>
          {trashError && (
            <div role="alert" className="flex items-center justify-between gap-3 rounded-xl bg-red-500/10 px-3 py-2 text-sm text-red-300">
              <span className="min-w-0 flex-1">{tr("Não foi possível carregar a lixeira (")}{trashError}{tr("). Seus itens continuam guardados no PC.")}</span>
              <button onClick={loadTrash} className="shrink-0 rounded-full bg-cardDark px-3 py-1.5 text-xs text-white">{tr("Tentar de novo")}</button>
            </div>
          )}
          {!trash && !trashError && <p className="text-sm text-textMuted">{tr("Carregando…")}</p>}
          {trash && trash.items.length === 0 && !trashError && <p className="text-sm text-textMuted">{tr("Vazia.")}</p>}
          {trash?.items.map((t) => (
            <div key={`${t.type}:${t.id || t.convId}:${t.batch || ''}`} className="space-y-2 rounded-xl bg-composer px-3 py-2.5 text-sm">
              <div className="min-w-0">
                <p className="break-words">{itemLabel(t)}</p>
                {t.preview && <p className="mt-0.5 line-clamp-2 break-words text-[13px] italic text-textMuted">“{t.preview}”</p>}
                <p className="mt-0.5 text-[11px] text-textMuted">{tr("apagado em ")}{new Date(t.deletedAt).toLocaleString()}{tr(" · restam ")}{daysLeft(t)}{tr(" dia(s)")}</p>
              </div>
              <div className="flex gap-2">
                <button onClick={() => run(() => api.restoreTrash(t), tr("Restaurado."))} className="rounded-full bg-cardDark px-3 py-1.5 text-xs">{tr("Restaurar")}</button>
                <button
                  onClick={async () => await confirm(tr("Apagar DE VEZ? Esta ação não pode ser desfeita.")) && run(() => api.purgeTrash(t), tr("Apagado definitivamente."))}
                  className="rounded-full bg-red-500/15 px-3 py-1.5 text-xs text-red-300"
                >{tr("\n                  Apagar de vez\n                ")}</button>
              </div>
            </div>
          ))}
          {trash?.items.length > 0 && (
            <button
              onClick={async () => await confirm(tr("Esvaziar a lixeira? Tudo o que está nela será apagado de vez e não poderá ser restaurado.")) && run(api.purgeAllTrash, tr("Lixeira esvaziada."))}
              className="rounded-full bg-red-500/15 px-4 py-2.5 text-sm text-red-300"
            >{tr("\n              Esvaziar lixeira\n            ")}</button>
          )}
        </Section>

        <Section title={tr("Servidor")} icon={<Database size={16} />}>
          {status ? (
            <div className="space-y-1 text-sm text-textMuted">
              <p>{tr('Provedores:')} {status.router.ok ? tr("conectado ({0} modelos)", [status.router.models]) : tr("indisponível — {0}", [status.router.error || ''])}</p>
              <p>{tr("Modelo local de memória: ")}{status.local.chat.ok ? tr("disponível") : tr("indisponível")}{tr(" · embeddings: ")}{status.local.embed.ok ? tr("disponível") : tr("indisponível")}</p>
              <p>{tr("Trabalhos: ")}{Object.entries(status.db.jobs).map(([k, v]) => `${k} ${v}`).join(' · ') || tr("nenhum")}</p>
            </div>
          ) : (
            <p className="text-sm text-textMuted">{tr("Sem resposta do servidor.")}</p>
          )}
        </Section>

        <Section title={tr("Zona de risco")} icon={<Trash2 size={16} />}>
          <button
            onClick={async () => await confirm(tr("Apagar o histórico de TODAS as conversas? Vai para a lixeira por 30 dias e é recusado se houver resposta em andamento.")) && run(api.clearAllHistory, tr("Histórico movido para a lixeira."))}
            className="rounded-full bg-red-500/15 px-4 py-2.5 text-sm text-red-300"
          >{tr("\n            Apagar todo o histórico\n          ")}</button>
        </Section>

        {msg && <p className="break-all rounded-xl bg-cardDark px-3 py-2 text-sm">{tr(msg)}</p>}
        {personas && <PersonaManager onClose={() => setPersonas(false)} />}
      </div>
    </Screen>
  );
}

function Section({ title, icon, children }) {
  useLanguage();
  return (
    <section className="mc-bubble space-y-2.5 rounded-[28px] p-4">
      <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-textMuted">{icon} {title}</h3>
      {children}
    </section>
  );
}
