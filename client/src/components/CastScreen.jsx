import React, { useEffect, useRef, useState } from 'react';
import { Plus, Check, Trash2, Merge, Camera, RefreshCw } from 'lucide-react';
import { api } from '../api.js';
import { Avatar, fileToAvatar } from '../ui.jsx';
import { Screen, Sheet, SheetButton } from './Sheet.jsx';

const KIND = { main: 'principal', npc: 'coadjuvante', user: 'você' };
const SOURCE = { card: 'da ficha .txt', narrative: 'da narrativa', manual: 'manual' };

// Simple screen to review and fix the cast identified from the character sheet and the story.
export default function CastScreen({ chatId, persona, onClose, onChanged }) {
  const [cast, setCast] = useState(null);
  const [edit, setEdit] = useState(null);
  const [error, setError] = useState('');

  async function reload() {
    try {
      setCast((await api.cast(chatId)).cast);
    } catch (e) {
      setError(e.message);
    }
  }
  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId]);

  const pending = cast?.filter((c) => !c.confirmed && c.kind !== 'user').length || 0;

  async function after(fn) {
    try {
      await fn();
      await reload();
      onChanged?.();
      setEdit(null);
    } catch (e) {
      setError(e.message);
    }
  }

  return (
    <Screen
      title="Elenco"
      onClose={onClose}
      action={
        <button onClick={() => setEdit({ isNew: true, name: '', aliases: [], description: '', kind: 'npc' })} className="rounded-full bg-accentPurple p-2 text-white" aria-label="Adicionar personagem">
          <Plus size={18} />
        </button>
      }
    >
      <div className="space-y-3 px-4 py-4">
        <p className="text-sm leading-relaxed text-textMuted">
          O elenco vem da sua ficha (.txt) e da narrativa. Confira os nomes: eles definem <b className="text-white">quem sabe o quê</b> e quais avatares aparecem nas falas.
          {pending > 0 && <span className="text-yellow-300"> {pending} aguardando confirmação.</span>}
        </p>
        <button
          onClick={() => after(async () => {
            const r = await api.reanalyzeCast(chatId);
            setError(`Ficha relida: ${r.total} personagem(ns)${r.removed ? `, ${r.removed} registro(s) errado(s) removido(s)` : ''}.`);
          })}
          className="flex items-center gap-2 rounded-full bg-composer px-4 py-2.5 text-sm"
        >
          <RefreshCw size={16} /> Reanalisar ficha
        </button>
        {error && <p className="text-sm text-yellow-200">{error}</p>}
        {cast?.length === 0 && <p className="py-8 text-center text-sm text-textMuted">Nenhum personagem ainda. A ficha será analisada ao salvar.</p>}
        {cast?.map((c) => (
          <button key={c.id} onClick={() => setEdit(c)} className="flex w-full items-center gap-3 rounded-2xl bg-cardDark p-3 text-left active:bg-cardDarkHover">
            <Avatar name={c.name} avatar={c.avatar} size={48} round />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate font-semibold">{c.name}</span>
                <span className="rounded-full bg-composer px-2 py-0.5 text-[11px] text-textMuted">{KIND[c.kind] || c.kind}</span>
                {c.confirmed ? <Check size={14} className="text-emerald-300" /> : <span className="text-[11px] text-yellow-300">confirmar</span>}
              </div>
              {c.description && <p className="line-clamp-2 text-sm text-textMuted">{c.description}</p>}
              {c.kind === 'user' && <p className="text-[11px] text-accentPurpleLight">Edite em ⋯ → Persona (perfil)</p>}
              <p className="text-[11px] text-textMuted">
                {SOURCE[c.source] || c.source}
                {c.aliases?.length ? ` · também: ${c.aliases.join(', ')}` : ''}
                {!c.active ? ' · inativo' : ''}
              </p>
            </div>
          </button>
        ))}
      </div>

      {edit && (
        <EditCast
          c={edit}
          others={(cast || []).filter((x) => x.id !== edit.id)}
          persona={persona}
          onClose={() => setEdit(null)}
          onSave={(patch) => after(() => (edit.isNew ? api.addCast(chatId, patch) : api.updateCast(chatId, edit.id, patch)))}
          onDelete={() => confirm(`Remover ${edit.name} do elenco?`) && after(() => api.deleteCast(chatId, edit.id))}
          onMerge={(intoId) => after(() => api.mergeCast(chatId, edit.id, intoId))}
        />
      )}
    </Screen>
  );
}

function EditCast({ c, others, onClose, onSave, onDelete, onMerge }) {
  const [name, setName] = useState(c.name);
  const [aliases, setAliases] = useState((c.aliases || []).join(', '));
  const [description, setDescription] = useState(c.description || '');
  const [kind, setKind] = useState(c.kind);
  const [active, setActive] = useState(c.active !== false);
  const [avatar, setAvatar] = useState(c.avatar || null);
  const [merging, setMerging] = useState(false);
  const fileRef = useRef(null);

  const patch = () => ({
    name: name.trim(),
    aliases: aliases.split(',').map((a) => a.trim()).filter(Boolean),
    description,
    kind,
    active,
    avatar,
    confirmed: true,
  });

  if (merging) {
    return (
      <Sheet onClose={() => setMerging(false)} title={`Mesclar ${c.name} em…`}>
        {others.filter((o) => o.kind !== 'user').map((o) => (
          <SheetButton key={o.id} icon={<Merge size={18} />} label={o.name} onClick={() => onMerge(o.id)} />
        ))}
      </Sheet>
    );
  }

  return (
    <Sheet onClose={onClose} title={c.isNew ? 'Novo personagem' : c.name} full>
      <div className="space-y-3 px-2 pb-4">
        <div className="flex items-center gap-3">
          <button onClick={() => fileRef.current?.click()} className="relative" aria-label="Avatar">
            <Avatar name={name} avatar={avatar} size={64} round />
            <span className="absolute -bottom-1 -right-1 rounded-full bg-accentPurple p-1"><Camera size={12} /></span>
          </button>
          <input ref={fileRef} type="file" accept="image/*" className="hidden" onChange={async (e) => e.target.files?.[0] && setAvatar(await fileToAvatar(e.target.files[0]))} />
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Nome" disabled={c.kind === 'user'} className="flex-1 rounded-xl bg-composer px-4 py-3 text-[16px] outline-none disabled:opacity-60" />
        </div>
        <label className="block text-sm">
          <span className="text-textMuted">Apelidos (separados por vírgula)</span>
          <input value={aliases} onChange={(e) => setAliases(e.target.value)} className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        </label>
        <label className="block text-sm">
          <span className="text-textMuted">Descrição curta</span>
          <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={3} className="mt-1 w-full resize-none rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        </label>
        {c.kind !== 'user' && (
          <div className="flex gap-2">
            {['main', 'npc'].map((k) => (
              <button key={k} onClick={() => setKind(k)} className={`rounded-full px-4 py-2 text-sm ${kind === k ? 'bg-accentPurple text-white' : 'bg-composer'}`}>{KIND[k]}</button>
            ))}
            <button onClick={() => setActive((a) => !a)} className={`ml-auto rounded-full px-4 py-2 text-sm ${active ? 'bg-composer' : 'bg-red-500/20 text-red-300'}`}>{active ? 'ativo' : 'inativo'}</button>
          </div>
        )}
        <button onClick={() => onSave(patch())} disabled={!name.trim()} className="w-full rounded-full bg-accentPurple py-3.5 font-semibold text-white disabled:opacity-40">
          {c.isNew ? 'Adicionar' : 'Salvar e confirmar'}
        </button>
        {!c.isNew && c.kind !== 'user' && (
          <div className="flex gap-2">
            <button onClick={() => setMerging(true)} className="flex flex-1 items-center justify-center gap-2 rounded-full bg-composer py-3 text-sm"><Merge size={16} /> Mesclar</button>
            <button onClick={onDelete} className="flex flex-1 items-center justify-center gap-2 rounded-full bg-red-500/15 py-3 text-sm text-red-300"><Trash2 size={16} /> Remover</button>
          </div>
        )}
      </div>
    </Sheet>
  );
}
