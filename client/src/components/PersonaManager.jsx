import { useConfirm } from './ConfirmProvider.jsx';
import { tr, useLanguage } from '../i18n.js';
import React, { useEffect, useRef, useState } from 'react';
import { Plus, Check, Pencil, Trash2, Camera, Star } from 'lucide-react';
import { api } from '../api.js';
import { Avatar, fileToAvatar } from '../ui.jsx';
import { Screen, Sheet } from './Sheet.jsx';

// Persona profiles (global). With `chat`, lets you pick which profile this conversation uses.
export default function PersonaManager({ chat, onClose, onChosen }) {
  useLanguage();
  const [list, setList] = useState(null);
  const [edit, setEdit] = useState(null);
  const [error, setError] = useState('');

  async function reload() {
    try {
      setList((await api.personas()).personas);
    } catch (e) {
      setError(e.message);
    }
  }
  useEffect(() => {
    reload();
  }, []);

  const activeId = chat?.settings?.personaId;

  async function choose(p) {
    if (!chat) return;
    try {
      const updated = await api.updateChat(chat.id, { personaId: p.id });
      onChosen?.(updated);
    } catch (e) {
      setError(e.message);
    }
  }

  return (
    <Screen
      title={chat ? tr("Persona desta conversa") : tr("Perfis de persona")}
      onClose={onClose}
      action={
        <button
          onClick={() => setEdit({ name: '', description: '', avatar: null, isDefault: false })}
          className="mc-glass-pill mc-glass-accent pointer-events-auto flex h-11 w-11 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
          aria-label={tr("Novo perfil")}
        >
          <Plus size={20} />
        </button>
      }
    >
      <div className="space-y-3 px-4 py-4">
        <p className="text-sm leading-relaxed text-textMuted">{tr("\n          Cada perfil é quem você interpreta: nome, descrição (enviada ao modelo como sua ficha) e avatar. ")}{chat ? tr("Toque em um perfil para usá-lo nesta conversa; vale para as próximas respostas.") : tr("O perfil com estrela é o padrão dos chats novos.")}
        </p>
        {error && <p className="text-sm text-red-300">{tr(error)}</p>}
        {list?.map((p) => (
          <div key={p.id} className={`mc-bubble flex items-start gap-3 rounded-[28px] p-3 ${activeId === p.id ? 'mc-selected' : ''}`}>
            <button onClick={() => choose(p)} className="flex min-w-0 flex-1 items-start gap-3 text-left" disabled={!chat}>
              <Avatar name={p.name} avatar={p.avatar} size={48} round />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate font-semibold">{p.name}</span>
                  {p.isDefault && <Star size={13} className="shrink-0 fill-yellow-300 text-yellow-300" />}
                  {activeId === p.id && <Check size={15} className="shrink-0 text-emerald-300" />}
                </div>
                <p className="line-clamp-3 whitespace-pre-line text-sm text-textMuted">{p.description || tr("Sem descrição.")}</p>
              </div>
            </button>
            <button onClick={() => setEdit(p)} className="p-1 text-textMuted" aria-label={tr("Editar perfil")}>
              <Pencil size={16} />
            </button>
          </div>
        ))}
      </div>
      {edit && (
        <EditPersona
          p={edit}
          onClose={() => setEdit(null)}
          onSaved={async () => {
            setEdit(null);
            await reload();
          }}
          onError={setError}
        />
      )}
    </Screen>
  );
}

function EditPersona({ p, onClose, onSaved, onError }) {
  const confirm = useConfirm();
  useLanguage();
  const [name, setName] = useState(p.name);
  const [description, setDescription] = useState(p.description || '');
  const [avatar, setAvatar] = useState(p.avatar || null);
  const [isDefault, setIsDefault] = useState(!!p.isDefault);
  const fileRef = useRef(null);

  async function save() {
    try {
      const body = { name, description, avatar, isDefault };
      if (p.id) await api.updatePersona(p.id, body);
      else await api.createPersona(body);
      onSaved();
    } catch (e) {
      onError(e.message);
    }
  }
  async function remove() {
    if (!await confirm(tr("Apagar o perfil \"{0}\"? Conversas que o usam mantêm o nome e a descrição atuais.", [p.name]))) return;
    try {
      await api.deletePersona(p.id);
      onSaved();
    } catch (e) {
      onError(e.message);
    }
  }

  return (
    <Sheet onClose={onClose} title={p.id ? tr("Editar perfil") : tr("Novo perfil")} full>
      <div className="space-y-3 px-2 pb-4">
        <div className="flex items-center gap-3">
          <button onClick={() => fileRef.current?.click()} className="relative" aria-label={tr("Avatar")}>
            <Avatar name={name || '?'} avatar={avatar} size={64} round />
            <span className="absolute -bottom-1 -right-1 rounded-full bg-accentPurple p-1"><Camera size={12} /></span>
          </button>
          <input ref={fileRef} type="file" accept="image/*" className="hidden" onChange={async (e) => e.target.files?.[0] && setAvatar(await fileToAvatar(e.target.files[0]))} />
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder={tr('Nome (ex.: Alex)')} className="flex-1 rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        </div>
        <label className="block text-sm">
          <span className="text-textMuted">{tr("Descrição (idade, aparência, origem, jeito...) — vai para o modelo como sua ficha")}</span>
          <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={9} className="mt-1 w-full resize-none rounded-xl bg-composer px-4 py-3 text-[15px] leading-relaxed outline-none" />
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={isDefault} onChange={(e) => setIsDefault(e.target.checked)} />{tr(" Padrão para chats novos\n        ")}</label>
        <button onClick={save} disabled={!name.trim()} className="mc-bubble mc-glass-accent w-full rounded-full py-3.5 font-semibold text-white disabled:opacity-40">{tr("\n          Salvar\n        ")}</button>
        {p.id && (
          <button onClick={remove} className="flex w-full items-center justify-center gap-2 rounded-full bg-red-500/15 py-3 text-sm text-red-300">
            <Trash2 size={16} />{tr(" Apagar perfil\n          ")}</button>
        )}
      </div>
    </Sheet>
  );
}
