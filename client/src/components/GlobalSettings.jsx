import React, { useEffect, useRef, useState } from 'react';
import { Download, Upload, Database, KeyRound, Trash2, RotateCcw, Smartphone, Link2Off, Type, Eye, Sparkles } from 'lucide-react';
import { PRESETS, applyEyeCare, loadEyeCare, saveEyeCare, colorsFor } from '../eyecare.js';
import { api, config, isNative } from '../api.js';
import { APP_ICONS, getSelectedAppIcon, applyAppIcon, initAppIcon } from '../app-icon.js';
import PersonaManager from './PersonaManager.jsx';
import { Screen } from './Sheet.jsx';

// App settings: reading, pairing, backup/import, trash and global cleanup.
export default function GlobalSettings({ fontSize, onFontSize, onClose, onChanged, onResetServer }) {
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
      if (r.pending) setIconMsg('Pronto. O ícone da tela inicial muda assim que você sair do app (pode levar alguns segundos no lançador).');
    } catch (e) {
      setIconMsg(`Não foi possível trocar o ícone: ${e.message || e}`);
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

  const itemLabel = (t) => (t.type === 'conversation' ? `Conversa “${t.name}”${t.count ? ` · ${t.count} mensagem(ns)` : ''}` : t.type === 'interactions' ? `${t.count} mensagem(ns) apagada(s) em “${t.name}”` : `${t.count} resposta(s) apagada(s) em “${t.name}”`);
  const daysLeft = (t) => Math.max(0, Math.ceil(((trash?.days || 30) * 86_400_000 - (Date.now() - new Date(t.deletedAt).getTime())) / 86_400_000));
  const preview = colorsFor(eye);

  return (
    <Screen title="Configurações" onClose={onClose}>
      <div className="space-y-5 px-4 py-4 text-[15px]">
        <Section title="Leitura" icon={<Type size={16} />}>
          <div className="flex items-center justify-between">
            <span>Tamanho da letra</span>
            <span className="font-mono text-accentPurpleLight">{fontSize}px</span>
          </div>
          <input type="range" min="14" max="22" step="1" value={fontSize} onChange={(e) => onFontSize(Number(e.target.value))} className="w-full accent-[#7c3aed]" />
          <p className="mc-read rounded-xl bg-composer p-3"><em className="mc-action">*Sobre a porta, as letras escuras parecem arder.*</em><br /><span className="mc-name">Dante:</span> Lasciate ogne speranza, voi ch’intrate.</p>
        </Section>

        <Section title="Proteção ocular" icon={<Eye size={16} />}>
          <label className="block">
            <span className="flex items-center justify-between">
              <span>Brilho das letras</span>
              <span className="font-mono text-accentPurpleLight">{eye.brightness}%</span>
            </span>
            <input type="range" min="50" max="100" step="1" value={eye.brightness} onChange={(e) => changeEye({ brightness: Number(e.target.value) })} className="w-full accent-[#7c3aed]" aria-label="Brilho das letras" />
          </label>
          <div>
            <p className="mb-1.5 text-sm">Cor das letras</p>
            <div className="flex flex-wrap gap-2">
              {Object.entries(PRESETS).map(([id, p]) => (
                <button
                  key={id}
                  onClick={() => changeEye({ preset: id })}
                  aria-pressed={eye.preset === id}
                  className={`flex items-center gap-2 rounded-full px-3 py-1.5 text-sm ${eye.preset === id ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}
                >
                  <span className="h-3.5 w-3.5 rounded-full border border-white/20" style={{ background: p.text }} />
                  {p.label}
                </button>
              ))}
              <label className={`flex items-center gap-2 rounded-full px-3 py-1.5 text-sm ${eye.preset === 'custom' ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
                <input type="color" value={eye.custom} onChange={(e) => changeEye({ preset: 'custom', custom: e.target.value })} className="h-5 w-5 cursor-pointer rounded-full border-0 bg-transparent p-0" aria-label="Escolher outra cor" />
                Outra cor
              </label>
            </div>
          </div>
          <label className="block">
            <span className="flex items-center justify-between">
              <span>Filtro de luz azul</span>
              <span className="font-mono text-accentPurpleLight">{eye.blue}%</span>
            </span>
            <input type="range" min="0" max="60" step="1" value={eye.blue} onChange={(e) => changeEye({ blue: Number(e.target.value) })} className="w-full accent-[#7c3aed]" aria-label="Filtro de luz azul" />
            <span className="mt-0.5 block text-xs text-textMuted">Esquenta as cores do app (fundos, letras e destaques), reduzindo o azul.</span>
          </label>
          <label className="block">
            <span className="flex items-center justify-between">
              <span>Textura de papel</span>
              <span className="font-mono text-accentPurpleLight">{eye.paper}%</span>
            </span>
            <input type="range" min="0" max="100" step="1" value={eye.paper} onChange={(e) => changeEye({ paper: Number(e.target.value) })} className="w-full accent-[#7c3aed]" aria-label="Textura de papel" />
            <span className="mt-0.5 block text-xs text-textMuted">Granulação sutil por cima da tela, que lembra a aspereza do papel e tira o brilho liso do vidro.</span>
          </label>
          <p className="mc-read rounded-xl bg-composer p-3" style={{ color: preview.text }}>
            <em style={{ color: preview.action }}>*Sobre a porta, as letras escuras parecem arder.*</em><br /><span style={{ color: preview.name, fontWeight: 600 }}>Dante:</span> Lasciate ogne speranza, voi ch’intrate.
          </p>
          <button onClick={() => changeEye({ preset: 'neutro', custom: '#e7e7ea', brightness: 100, blue: 0, paper: 0 })} className="rounded-full bg-composer px-4 py-2 text-sm">Restaurar padrão</button>
        </Section>

        <Section title="Ícone do aplicativo" icon={<Sparkles size={16} />}>
          <p className="text-sm text-textMuted">Escolha o ícone do aplicativo:</p>
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
                    <span className="block truncate text-[11px] text-textMuted">{active ? 'Em uso' : ic.subtitle}</span>
                  </span>
                </button>
              );
            })}
          </div>
          {iconMsg && <p className="text-xs text-textMuted">{iconMsg}</p>}
        </Section>

        <Section title="Perfis de persona" icon={<Type size={16} />}>
          <p className="text-sm text-textMuted">Crie perfis para quem você interpreta e troque entre eles em cada conversa (⋯ → Persona).</p>
          <button onClick={() => setPersonas(true)} className="rounded-full bg-composer px-4 py-2.5 text-sm">Gerenciar perfis</button>
        </Section>

        <Section title="Pareamento" icon={<Smartphone size={16} />}>
          {isNative() ? (
            <>
              <p className="text-sm text-textMuted">Servidor: {config.server || '—'}</p>
              <button onClick={onResetServer} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Link2Off size={16} /> Trocar PC / refazer pareamento</button>
            </>
          ) : (
            <>
              <button onClick={() => run(async () => setCode(await api.startPairing()), 'Código gerado (vale 5 min).')} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><KeyRound size={16} /> Gerar código de pareamento</button>
              {code && <p className="rounded-xl bg-composer p-3 text-center font-mono text-[28px] tracking-[0.3em]">{code.code}</p>}
            </>
          )}
          {devices?.filter((d) => !d.revoked_at).map((d) => (
            <div key={d.id} className="flex items-center justify-between rounded-xl bg-composer px-3 py-2 text-sm">
              <span>{d.name}<span className="block text-[11px] text-textMuted">visto {d.last_seen_at ? new Date(d.last_seen_at).toLocaleString() : 'nunca'}</span></span>
              <button onClick={() => run(() => api.revokeDevice(d.id), 'Aparelho removido.')} aria-label="Remover aparelho"><Trash2 size={16} className="text-red-300" /></button>
            </div>
          ))}
          <p className="text-xs text-textMuted">As chaves dos provedores ficam só no PC; o aparelho guarda apenas um token do ModularChat.</p>
        </Section>

        <Section title="Backup" icon={<Database size={16} />}>
          <div className="flex flex-wrap gap-2">
            <button onClick={() => run(api.backup, (r) => `Cópia do banco salva no PC: ${r.file}`)} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Database size={16} /> Backup no PC</button>
            <button onClick={() => run(exportAll, 'Exportação baixada.')} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Download size={16} /> Exportar tudo</button>
            <button onClick={() => fileRef.current?.click()} className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"><Upload size={16} /> Importar</button>
            <input
              ref={fileRef} type="file" accept="application/json,.json" className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = '';
                if (f) run(async () => api.importData(JSON.parse(await f.text())), (r) => `${r.imported.length} conversa(s) importada(s)${r.errors.length ? `, ${r.errors.length} com erro` : ''}.`);
              }}
            />
          </div>
        </Section>

        <Section title={`Lixeira (${trash?.days || 30} dias)`} icon={<RotateCcw size={16} />}>
          {trashError && (
            <div role="alert" className="flex items-center justify-between gap-3 rounded-xl bg-red-500/10 px-3 py-2 text-sm text-red-300">
              <span className="min-w-0 flex-1">Não foi possível carregar a lixeira ({trashError}). Seus itens continuam guardados no PC.</span>
              <button onClick={loadTrash} className="shrink-0 rounded-full bg-cardDark px-3 py-1.5 text-xs text-white">Tentar de novo</button>
            </div>
          )}
          {!trash && !trashError && <p className="text-sm text-textMuted">Carregando…</p>}
          {trash && trash.items.length === 0 && !trashError && <p className="text-sm text-textMuted">Vazia.</p>}
          {trash?.items.map((t) => (
            <div key={`${t.type}:${t.id || t.convId}:${t.batch || ''}`} className="space-y-2 rounded-xl bg-composer px-3 py-2.5 text-sm">
              <div className="min-w-0">
                <p className="break-words">{itemLabel(t)}</p>
                {t.preview && <p className="mt-0.5 line-clamp-2 break-words text-[13px] italic text-textMuted">“{t.preview}”</p>}
                <p className="mt-0.5 text-[11px] text-textMuted">apagado em {new Date(t.deletedAt).toLocaleString()} · restam {daysLeft(t)} dia(s)</p>
              </div>
              <div className="flex gap-2">
                <button onClick={() => run(() => api.restoreTrash(t), 'Restaurado.')} className="rounded-full bg-cardDark px-3 py-1.5 text-xs">Restaurar</button>
                <button
                  onClick={() => confirm('Apagar DE VEZ? Esta ação não pode ser desfeita.') && run(() => api.purgeTrash(t), 'Apagado definitivamente.')}
                  className="rounded-full bg-red-500/15 px-3 py-1.5 text-xs text-red-300"
                >
                  Apagar de vez
                </button>
              </div>
            </div>
          ))}
          {trash?.items.length > 0 && (
            <button
              onClick={() => confirm('Esvaziar a lixeira? Tudo o que está nela será apagado de vez e não poderá ser restaurado.') && run(api.purgeAllTrash, 'Lixeira esvaziada.')}
              className="rounded-full bg-red-500/15 px-4 py-2.5 text-sm text-red-300"
            >
              Esvaziar lixeira
            </button>
          )}
        </Section>

        <Section title="Servidor" icon={<Database size={16} />}>
          {status ? (
            <div className="space-y-1 text-sm text-textMuted">
              <p>Provedores: {status.router.ok ? `conectado (${status.router.models} modelos)` : `indisponível — ${status.router.error || ''}`}</p>
              <p>Modelo local de memória: {status.local.chat.ok ? 'disponível' : 'indisponível'} · embeddings: {status.local.embed.ok ? 'disponível' : 'indisponível'}</p>
              <p>Trabalhos: {Object.entries(status.db.jobs).map(([k, v]) => `${k} ${v}`).join(' · ') || 'nenhum'}</p>
            </div>
          ) : (
            <p className="text-sm text-textMuted">Sem resposta do servidor.</p>
          )}
        </Section>

        <Section title="Zona de risco" icon={<Trash2 size={16} />}>
          <button
            onClick={() => confirm('Apagar o histórico de TODAS as conversas? Vai para a lixeira por 30 dias e é recusado se houver resposta em andamento.') && run(api.clearAllHistory, 'Histórico movido para a lixeira.')}
            className="rounded-full bg-red-500/15 px-4 py-2.5 text-sm text-red-300"
          >
            Apagar todo o histórico
          </button>
        </Section>

        {msg && <p className="break-all rounded-xl bg-cardDark px-3 py-2 text-sm">{msg}</p>}
        {personas && <PersonaManager onClose={() => setPersonas(false)} />}
      </div>
    </Screen>
  );
}

function Section({ title, icon, children }) {
  return (
    <section className="mc-bubble space-y-2.5 rounded-[28px] p-4">
      <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-textMuted">{icon} {title}</h3>
      {children}
    </section>
  );
}
