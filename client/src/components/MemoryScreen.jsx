import React, { useEffect, useState } from 'react';
import { Trash2, Lock, Globe, Plus, RefreshCw, Cpu, Pin, FileText } from 'lucide-react';
import { api } from '../api.js';
import { Screen, Sheet } from './Sheet.jsx';

const TABS = [['scene', 'Cena e dias'], ['memories', 'Memórias'], ['story', 'Relações e threads'], ['state', 'Processamento']];
const KIND = { CHARACTER_FACT: 'personagem', USER_FACT: 'usuário', RELATIONSHIP: 'relação', EVENT: 'evento', SECRET: 'segredo', PROMISE: 'promessa', PREFERENCE: 'preferência', WORLD_FACT: 'mundo', OBJECT: 'objeto', LOCATION: 'local', OPEN_THREAD: 'thread' };
const VIA = { witnessed: 'presenciou', heard: 'ouviu', told: 'foi contado', discovered: 'descobriu', received: 'recebeu', inferred: 'deduziu' };

// Review and fix what the PC remembers: scene, memories with sources, relationships and the processing queue.
export default function MemoryScreen({ chatId, onClose }) {
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
      title="Memória do PC"
      onClose={onClose}
      action={
        tab === 'memories' ? (
          <button onClick={() => setAdding(true)} className="rounded-full bg-accentPurple p-2 text-white" aria-label="Adicionar memória">
            <Plus size={18} />
          </button>
        ) : null
      }
    >
      <div className="flex gap-2 overflow-x-auto px-4 pt-3">
        {TABS.map(([k, label]) => (
          <button key={k} onClick={() => setTab(k)} className={`shrink-0 rounded-full px-4 py-2 text-sm ${tab === k ? 'bg-accentPurple text-white' : 'bg-cardDark'}`}>{label}</button>
        ))}
      </div>
      {error && <p className="px-4 pt-3 text-sm text-red-300">{error}</p>}
      {data && (
        <div className="space-y-3 px-4 py-4">
          {degraded && (
            <div className="flex gap-2 rounded-xl bg-yellow-500/10 px-3 py-2 text-xs text-yellow-200">
              <Cpu size={14} className="mt-0.5 shrink-0" />
              Modelo local indisponível ({local.chat.reason || local.chat.lastError}). Funcionando com histórico recente e busca textual; a extração fica pendente. Nada é enviado a serviços externos para compensar.
            </div>
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
  const [source, setSource] = useState(null);
  const s = data.scene;
  return (
    <>
      <div className={`rounded-xl px-3 py-2 text-xs ${data.sync.synced ? 'bg-emerald-500/10 text-emerald-200' : 'bg-yellow-500/10 text-yellow-200'}`}>
        {data.sync.synced ? 'Estado sincronizado com a conversa.' : `${data.sync.lag} turno(s) ainda não processado(s): o histórico recente vale mais que este estado.`}
      </div>
      {!s && <p className="py-6 text-center text-sm text-textMuted">Ainda não há estado de cena extraído.</p>}
      {s && (
        <div className="space-y-2 rounded-2xl bg-cardDark p-4 text-sm">
          <Line k="Local" v={s.location} />
          <Line k="Momento" v={s.time} />
          <Line k="Clima" v={s.mood} />
          <Line k="Presentes" v={s.present?.join(', ')} />
          <Line k="Objetos" v={s.objects?.join(', ')} />
          {s.characters?.map((c) => (
            <Line key={c.key} k={c.name} v={[c.position, c.location, c.holding?.length ? `segura ${c.holding.join(', ')}` : ''].filter(Boolean).join(' · ')} />
          ))}
        </div>
      )}
      {data.timeline?.days?.map(day => (
        <div key={day.key} className="space-y-2 rounded-2xl bg-cardDark p-4 text-sm">
          <b>{day.date || `${day.months ? `${day.months} mês(es) + ` : ''}Dia ${day.days} desde o início`}</b>
          {(day.blocks || []).map(block => (
            <div key={`${block.sourceId}:${block.start}`} className="border-t border-borderDark pt-2">
              <span className="text-xs text-textMuted">Turno {block.seq} · {block.role === 'user' ? 'mensagem enviada' : 'resposta'}</span>
              <button className="ml-3 text-xs text-accentLight" onClick={async () => {
                const original = await api.source(chatId, block.interactionId);
                const text = original.sources?.find(s => s.id === block.sourceId)?.text;
                setSource({ ...original, text: text == null ? 'A fonte mudou; atualize a memória para consultar o trecho atual.' : text.slice(block.start, block.end), block });
              }}>Ver trecho original</button>
            </div>
          ))}
        </div>
      ))}
      {source && <Sheet title="Fonte do dia" onClose={() => setSource(null)}><p className="whitespace-pre-wrap px-5 py-4 text-sm">{source.text}</p></Sheet>}
    </>
  );
}

function Memories({ chatId, data, onChanged }) {
  const [sel, setSel] = useState(null);
  async function del(m) {
    if (!confirm('Apagar esta memória?')) return;
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
      <p className="text-xs text-textMuted">Fixe (📌) o que nunca pode ser esquecido: memórias fixadas entram em todas as respostas, além das 8 recuperadas por relevância.</p>
      {data.memories.length === 0 && <p className="py-6 text-center text-sm text-textMuted">Nenhuma memória ainda. Elas surgem após cada resposta (quando o modelo local está disponível).</p>}
      {data.memories.map((m) => (
        <div key={m.id} className="rounded-2xl bg-cardDark p-3">
          <div className="mb-1 flex items-center gap-2 text-[11px] text-textMuted">
            <span className="rounded-full bg-composer px-2 py-0.5">{KIND[m.kind] || m.kind}</span>
            {m.subject && <span>{m.subject}</span>}
            <button onClick={() => toggleVis(m)} className="ml-auto flex items-center gap-1" title="Alternar entre restrita e pública">
              {m.visibility === 'private' ? <><Lock size={11} /> restrita</> : <><Globe size={11} /> pública</>}
            </button>
            <button onClick={() => togglePin(m)} aria-label={m.pinned ? 'Desafixar' : 'Fixar'} className={m.pinned ? 'text-accentPurpleLight' : ''}><Pin size={13} /></button>
            {m.sourceInteractionId && <button onClick={() => showSource(m)} aria-label="Ver origem"><FileText size={13} /></button>}
            <button onClick={() => del(m)} aria-label="Apagar"><Trash2 size={13} /></button>
          </div>
          <p className="text-sm" onClick={() => setSel(sel === m.id ? null : m.id)}>{m.content}</p>
          {m.knowledge.length > 0 && (
            <p className="mt-1 text-[11px] text-textMuted">
              {m.knowledge.map((k) => `${k.name}: ${k.state === 'knows' ? 'sabe' : k.state === 'believes' ? 'acredita' : 'desconhece'}${k.via ? ` (${VIA[k.via] || k.via})` : ''}`).join(' · ')}
            </p>
          )}
          {sel === m.id && <p className="mt-1 text-[11px] text-textMuted">importância {Math.round(m.importance * 100)}% · confiança {Math.round(m.confidence * 100)}%{m.sourceInteractionId ? ' · com fonte na conversa' : ' · manual'}</p>}
        </div>
      ))}
      {src && (
        <Sheet onClose={() => setSrc(null)} title={src.error ? 'Origem' : `Origem: turno ${src.seq}${src.deleted ? ' (apagado)' : ''}`} full>
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
  return (
    <>
      <h3 className="text-xs font-semibold uppercase tracking-wide text-textMuted">Relações</h3>
      {data.relationships.length === 0 && <p className="text-sm text-textMuted">Nenhuma.</p>}
      {data.relationships.map((r) => (
        <p key={r.id} className="rounded-2xl bg-cardDark p-3 text-sm"><b>{r.a}</b> ↔ <b>{r.b}</b>: {r.description}</p>
      ))}
      <h3 className="pt-2 text-xs font-semibold uppercase tracking-wide text-textMuted">Threads abertas</h3>
      {data.threads.length === 0 && <p className="text-sm text-textMuted">Nenhuma.</p>}
      {data.threads.map((t) => <p key={t.id} className="rounded-2xl bg-cardDark p-3 text-sm">{t.text}</p>)}
      <h3 className="pt-2 text-xs font-semibold uppercase tracking-wide text-textMuted">Episódios resumidos</h3>
      {data.episodes.length === 0 && <p className="text-sm text-textMuted">Os resumos surgem quando os turnos saem da janela recente; os originais continuam guardados.</p>}
      {data.episodes.map((e) => <p key={e.id} className="rounded-2xl bg-cardDark p-3 text-sm"><span className="text-xs text-textMuted">turnos {e.from_seq}–{e.to_seq}</span><br />{e.summary}</p>)}
    </>
  );
}

function State({ chatId, data, onChanged }) {
  const j = data.jobs || {};
  const l = data.local;
  return (
    <>
      <div className="space-y-1.5 rounded-2xl bg-cardDark p-4 text-sm">
        <Line k="Pendentes" v={String((j.pending || 0) + (j.running || 0))} />
        <Line k="Falharam" v={String(j.failed || 0)} />
        <Line k="Concluídos" v={String(j.done || 0)} />
        <Line k="Modelo de memória (GPU sob demanda)" v={l.chat.ok ? 'Qwen3-4B Q4 disponível' : 'indisponível'} />
        <Line k="Embeddings (CPU)" v={l.embed.ok ? 'BGE-M3 disponível' : 'indisponível'} />
      </div>
      {(j.failed || 0) > 0 && (
        <button onClick={async () => { await api.retryJobs(chatId); onChanged(); }} className="flex w-full items-center justify-center gap-2 rounded-full bg-composer py-3 text-sm">
          <RefreshCw size={16} /> Tentar de novo os que falharam
        </button>
      )}
    </>
  );
}

function AddMemory({ chatId, cast, onClose, onSaved }) {
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
    <Sheet onClose={onClose} title="Nova memória manual" full>
      <div className="space-y-3 px-2 pb-4">
        <select value={kind} onChange={(e) => setKind(e.target.value)} className="w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none">
          {Object.entries(KIND).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
        </select>
        <input value={subject} onChange={(e) => setSubject(e.target.value)} placeholder="Assunto (personagem ou coisa)" className="w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        <textarea value={content} onChange={(e) => setContent(e.target.value)} rows={4} placeholder="Fato atômico, ex.: Lia sabotou o carro." className="w-full resize-none rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={priv} onChange={(e) => setPriv(e.target.checked)} /> Restrita (só alguns personagens sabem)</label>
        {priv && (
          <div className="flex flex-wrap gap-2">
            {cast.filter((c) => c.kind !== 'user' || true).map((c) => (
              <button key={c.id} onClick={() => setKnowers((k) => (k.includes(c.name) ? k.filter((x) => x !== c.name) : [...k, c.name]))} className={`rounded-full px-3 py-1.5 text-sm ${knowers.includes(c.name) ? 'bg-accentPurple text-white' : 'bg-composer'}`}>{c.name}</button>
            ))}
          </div>
        )}
        <button onClick={save} disabled={content.trim().length < 6} className="w-full rounded-full bg-accentPurple py-3.5 font-semibold text-white disabled:opacity-40">Salvar</button>
      </div>
    </Sheet>
  );
}

const Line = ({ k, v }) => (v ? (
  <div className="flex justify-between gap-4"><span className="text-textMuted">{k}</span><span className="text-right">{v}</span></div>
) : null);
