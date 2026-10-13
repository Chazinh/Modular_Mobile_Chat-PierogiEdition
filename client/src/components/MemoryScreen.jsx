import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage } from '../i18n.js';
import ChoiceField from './ChoiceField.jsx';
import React, { useEffect, useState } from 'react';
import { Trash2, Lock, Globe, Plus, RefreshCw, Cpu, Pin, FileText } from 'lucide-react';
import { api } from '../api.js';
import { Screen, Sheet } from './Sheet.jsx';

const TABS = [['scene', 'Cena e dias'], ['memories', 'Memórias'], ['story', 'Relações e threads'], ['state', 'Processamento']];
const KIND = { CHARACTER_FACT: 'personagem', USER_FACT: 'usuário', RELATIONSHIP: 'relação', EVENT: 'evento', SECRET: 'segredo', PROMISE: 'promessa', PREFERENCE: 'preferência', WORLD_FACT: 'mundo', OBJECT: 'objeto', LOCATION: 'local', OPEN_THREAD: 'thread' };
const VIA = { witnessed: 'presenciou', heard: 'ouviu', told: 'foi contado', discovered: 'descobriu', received: 'recebeu', inferred: 'deduziu' };

// Review and fix what the PC remembers: scene, memories with sources, relationships and the processing queue.
export default function MemoryScreen({ chatId, onClose }) {
  useLanguage();
  const [tab, setTab] = useState('scene');
  const [data, setData] = useState(null);
  const [error, setError] = useState('');
  const [adding, setAdding] = useState(false);

  async function reload() {
    try {
      setData(await api.memory(chatId));
      setError('');
    } catch (e) {
      setError(e.message);
    }
  }
  useEffect(() => {
    reload();
    const on = (e) => e.detail.convId === chatId && ['memory', 'flags', 'cast'].includes(e.detail.type) && reload();
    window.addEventListener('mc:event', on);
    return () => window.removeEventListener('mc:event', on);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId]);

  const local = data?.local;
  const degraded = local && !local.chat.ok;

  return (
    <Screen
      title={tr("Memória do PC")}
      onClose={onClose}
      action={
        tab === 'memories' ? (
          <button onClick={() => setAdding(true)} className="rounded-full bg-accentPurple p-2 text-white" aria-label={tr("Adicionar memória")}>
            <Plus size={18} />
          </button>
        ) : null
      }
    >
      <div className="flex gap-2 overflow-x-auto px-4 pt-3">
        {TABS.map(([k, label]) => (
          <button key={k} onClick={() => setTab(k)} className={`shrink-0 rounded-full px-4 py-2 text-sm ${tab === k ? 'bg-accentPurple text-white' : 'bg-cardDark'}`}>{tr(label)}</button>
        ))}
      </div>
      {error && <p className="px-4 pt-3 text-sm text-red-300">{tr(error)}</p>}
      {data && (
        <div className="space-y-3 px-4 py-4">
          {degraded && (
            <div className="flex gap-2 rounded-xl bg-yellow-500/10 px-3 py-2 text-xs text-yellow-200">
              <Cpu size={14} className="mt-0.5 shrink-0" />{tr("\n              Modelo local indisponível (")}{local.chat.reason || local.chat.lastError}{tr("). Funcionando com histórico recente e busca textual; a extração fica pendente. Nada é enviado a serviços externos para compensar.\n            ")}</div>
          )}
          {tab === 'scene' && <Scene data={data} chatId={chatId} />}
          {tab === 'memories' && <Memories chatId={chatId} data={data} onChanged={reload} />}
          {tab === 'story' && <Story data={data} />}
          {tab === 'state' && <State chatId={chatId} data={data} onChanged={reload} />}
        </div>
      )}
      {adding && <AddMemory chatId={chatId} cast={data?.cast || []} onClose={() => setAdding(false)} onSaved={() => { setAdding(false); reload(); }} />}
    </Screen>
  );
}

function Scene({ data, chatId }) {
  useLanguage();
  const [source, setSource] = useState(null);
  const s = data.scene;
  return (
    <>
      <div className={`rounded-xl px-3 py-2 text-xs ${data.sync.synced ? 'bg-emerald-500/10 text-emerald-200' : 'bg-yellow-500/10 text-yellow-200'}`}>
        {data.sync.synced ? tr("Estado sincronizado com a conversa.") : tr("{0} turno(s) ainda não processado(s): o histórico recente vale mais que este estado.", [data.sync.lag])}
      </div>
      {!s && <p className="py-6 text-center text-sm text-textMuted">{tr("Ainda não há estado de cena extraído.")}</p>}
      {s && (
        <div className="space-y-2 rounded-2xl bg-cardDark p-4 text-sm">
          <Line k={tr("Local")} v={s.location} />
          <Line k={tr("Momento")} v={s.time} />
          <Line k={tr("Clima")} v={s.mood} />
          <Line k={tr("Presentes")} v={s.present?.join(', ')} />
          <Line k={tr("Objetos")} v={s.objects?.join(', ')} />
          {s.characters?.map((c) => (
            <Line key={c.key} k={c.name} v={[c.position, c.location, c.holding?.length ? `segura ${c.holding.join(', ')}` : ''].filter(Boolean).join(' · ')} />
          ))}
        </div>
      )}
      {data.timeline?.days?.map(day => (
        <div key={day.key} className="space-y-2 rounded-2xl bg-cardDark p-4 text-sm">
          <b>{day.date || tr("{0}Dia {1} desde o início", [day.months ? tr("{0} mês(es) + ", [day.months]) : '', day.days])}</b>
          {(day.blocks || []).map(block => (
            <div key={`${block.sourceId}:${block.start}`} className="border-t border-borderDark pt-2">
              <span className="text-xs text-textMuted">{tr("Turno ")}{block.seq} · {block.role === 'user' ? tr("mensagem enviada") : tr("resposta")}</span>
              <button className="ml-3 text-xs text-accentLight" onClick={async () => {
                const original = await api.source(chatId, block.interactionId);
                const text = original.sources?.find(s => s.id === block.sourceId)?.text;
                setSource({ ...original, text: text == null ? 'A fonte mudou; atualize a memória para consultar o trecho atual.' : text.slice(block.start, block.end), block });
              }}>{tr("Ver trecho original")}</button>
            </div>
          ))}
        </div>
      ))}
      {source && <Sheet title={tr("Fonte do dia")} onClose={() => setSource(null)}><p className="whitespace-pre-wrap px-5 py-4 text-sm">{source.text}</p></Sheet>}
    </>
  );
}

function Memories({ chatId, data, onChanged }) {
  const confirm = useConfirm();
  useLanguage();
  const [sel, setSel] = useState(null);
  async function del(m) {
    if (!await confirm(tr("Apagar esta memória?"))) return;
    await api.deleteMemory(chatId, m.id);
    onChanged();
  }
  const [src, setSrc] = useState(null);
  async function togglePin(m) {
    await api.updateMemory(chatId, m.id, { pinned: !m.pinned });
    onChanged();
  }
  async function showSource(m) {
    try {
      setSrc(await api.source(chatId, m.sourceInteractionId));
    } catch (e) {
      setSrc({ error: e.message });
    }
  }
  async function toggleVis(m) {
    await api.updateMemory(chatId, m.id, { visibility: m.visibility === 'private' ? 'public' : 'private' });
    onChanged();
  }
  return (
    <>
      <p className="text-xs text-textMuted">{tr("Fixe (📌) o que nunca pode ser esquecido: memórias fixadas entram em todas as respostas, além das 8 recuperadas por relevância.")}</p>
      {data.memories.length === 0 && <p className="py-6 text-center text-sm text-textMuted">{tr("Nenhuma memória ainda. Elas surgem após cada resposta (quando o modelo local está disponível).")}</p>}
      {data.memories.map((m) => (
        <div key={m.id} className="rounded-2xl bg-cardDark p-3">
          <div className="mb-1 flex items-center gap-2 text-[11px] text-textMuted">
            <span className="rounded-full bg-composer px-2 py-0.5">{tr(KIND[m.kind] || m.kind)}</span>
            {m.subject && <span>{m.subject}</span>}
            <button onClick={() => toggleVis(m)} className="ml-auto flex items-center gap-1" title={tr("Alternar entre restrita e pública")}>
              {m.visibility === 'private' ? <><Lock size={11} />{tr(" restrita")}</> : <><Globe size={11} />{tr(" pública")}</>}
            </button>
            <button onClick={() => togglePin(m)} aria-label={m.pinned ? tr("Desafixar") : tr("Fixar")} className={m.pinned ? 'text-accentPurpleLight' : ''}><Pin size={13} /></button>
            {m.sourceInteractionId && <button onClick={() => showSource(m)} aria-label={tr("Ver origem")}><FileText size={13} /></button>}
            <button onClick={() => del(m)} aria-label={tr("Apagar")}><Trash2 size={13} /></button>
          </div>
          <p className="text-sm" onClick={() => setSel(sel === m.id ? null : m.id)}>{m.content}</p>
          {m.knowledge.length > 0 && (
            <p className="mt-1 text-[11px] text-textMuted">
              {m.knowledge.map((k) => `${k.name}: ${k.state === 'knows' ? tr('sabe') : k.state === 'believes' ? tr('acredita') : tr('desconhece')}${k.via ? ` (${tr(VIA[k.via] || k.via)})` : ''}`).join(' · ')}
            </p>
          )}
          {sel === m.id && <p className="mt-1 text-[11px] text-textMuted">{tr("importância ")}{Math.round(m.importance * 100)}{tr("% · confiança ")}{Math.round(m.confidence * 100)}%{m.sourceInteractionId ? tr(" · com fonte na conversa") : tr(" · manual")}</p>}
        </div>
      ))}
      {src && (
        <Sheet onClose={() => setSrc(null)} title={src.error ? tr("Origem") : tr("Origem: turno {0}{1}", [src.seq, src.deleted ? tr(" (apagado)") : ''])} full>
          {src.error ? (
            <p className="px-3 text-sm text-red-300">{src.error}</p>
          ) : (
            <div className="space-y-3 px-2 pb-4 text-sm">
              {src.user && <p className="rounded-2xl bg-accentPurple/20 p-3">{src.user}</p>}
              <p className="whitespace-pre-wrap rounded-2xl bg-cardDark p-3">{src.assistant}</p>
            </div>
          )}
        </Sheet>
      )}
    </>
  );
}

function Story({ data }) {
  useLanguage();
  return (
    <>
      <h3 className="text-xs font-semibold uppercase tracking-wide text-textMuted">{tr("Relações")}</h3>
      {data.relationships.length === 0 && <p className="text-sm text-textMuted">{tr("Nenhuma.")}</p>}
      {data.relationships.map((r) => (
        <p key={r.id} className="rounded-2xl bg-cardDark p-3 text-sm"><b>{r.a}</b> ↔ <b>{r.b}</b>: {r.description}</p>
      ))}
      <h3 className="pt-2 text-xs font-semibold uppercase tracking-wide text-textMuted">{tr("Threads abertas")}</h3>
      {data.threads.length === 0 && <p className="text-sm text-textMuted">{tr("Nenhuma.")}</p>}
      {data.threads.map((t) => <p key={t.id} className="rounded-2xl bg-cardDark p-3 text-sm">{t.text}</p>)}
      <h3 className="pt-2 text-xs font-semibold uppercase tracking-wide text-textMuted">{tr("Episódios resumidos")}</h3>
      {data.episodes.length === 0 && <p className="text-sm text-textMuted">{tr("Os resumos surgem quando os turnos saem da janela recente; os originais continuam guardados.")}</p>}
      {data.episodes.map((e) => <p key={e.id} className="rounded-2xl bg-cardDark p-3 text-sm"><span className="text-xs text-textMuted">{tr("turnos ")}{e.from_seq}–{e.to_seq}</span><br />{e.summary}</p>)}
    </>
  );
}

function State({ chatId, data, onChanged }) {
  useLanguage();
  const j = data.jobs || {};
  const l = data.local;
  return (
    <>
      <div className="space-y-1.5 rounded-2xl bg-cardDark p-4 text-sm">
        <Line k={tr("Pendentes")} v={String((j.pending || 0) + (j.running || 0))} />
        <Line k={tr("Falharam")} v={String(j.failed || 0)} />
        <Line k={tr("Concluídos")} v={String(j.done || 0)} />
        <Line k={tr("Modelo de memória (GPU sob demanda)")} v={l.chat.ok ? tr("Qwen3-4B Q4 disponível") : tr("indisponível")} />
        <Line k={tr("Embeddings (CPU)")} v={l.embed.ok ? tr("BGE-M3 disponível") : tr("indisponível")} />
      </div>
      {(j.failed || 0) > 0 && (
        <button onClick={async () => { await api.retryJobs(chatId); onChanged(); }} className="flex w-full items-center justify-center gap-2 rounded-full bg-composer py-3 text-sm">
          <RefreshCw size={16} />{tr(" Tentar de novo os que falharam\n        ")}</button>
      )}
    </>
  );
}

function AddMemory({ chatId, cast, onClose, onSaved }) {
  useLanguage();
  const [content, setContent] = useState('');
  const [subject, setSubject] = useState('');
  const [kind, setKind] = useState('WORLD_FACT');
  const [knowers, setKnowers] = useState([]);
  const [priv, setPriv] = useState(false);
  async function save() {
    await api.addMemory(chatId, { content, subject, kind, visibility: priv ? 'private' : 'public', knowers: priv ? knowers : [], importance: 0.8 });
    onSaved();
  }
  return (
    <Sheet onClose={onClose} title={tr("Nova memória manual")} full>
      <div className="space-y-3 px-2 pb-4">
        <ChoiceField label={tr('Tipo de memória')} value={kind} onChange={setKind}
          options={Object.entries(KIND).map(([value, label]) => ({ value, label: tr(label) }))} />
        <input value={subject} onChange={(e) => setSubject(e.target.value)} placeholder={tr("Assunto (personagem ou coisa)")} className="w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        <textarea value={content} onChange={(e) => setContent(e.target.value)} rows={4} placeholder={tr("Fato atômico, ex.: Lia sabotou o carro.")} className="w-full resize-none rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={priv} onChange={(e) => setPriv(e.target.checked)} />{tr(" Restrita (só alguns personagens sabem)")}</label>
        {priv && (
          <div className="flex flex-wrap gap-2">
            {cast.filter((c) => c.kind !== 'user' || true).map((c) => (
              <button key={c.id} onClick={() => setKnowers((k) => (k.includes(c.name) ? k.filter((x) => x !== c.name) : [...k, c.name]))} className={`rounded-full px-3 py-1.5 text-sm ${knowers.includes(c.name) ? 'bg-accentPurple text-white' : 'bg-composer'}`}>{c.name}</button>
            ))}
          </div>
        )}
        <button onClick={save} disabled={content.trim().length < 6} className="w-full rounded-full bg-accentPurple py-3.5 font-semibold text-white disabled:opacity-40">{tr("Salvar")}</button>
      </div>
    </Sheet>
  );
}

const Line = ({ k, v }) => (v ? (
  <div className="flex justify-between gap-4"><span className="text-textMuted">{k}</span><span className="text-right">{v}</span></div>
) : null);
