import React, { useState } from 'react';
import { useLayer } from '../nav.js';
import '../drawer.css';

export default function RenameModal({ title = 'Renomear conversa', initial = '', onClose, onSave }) {
  const [name, setName] = useState(initial);
  const [saving, setSaving] = useState(false);

  useLayer(true, onClose);

  async function submit(e) {
    e?.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || saving) return;
    setSaving(true);
    try {
      await onSave(trimmed);
      onClose();
    } catch {
      setSaving(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-md mc-modal-backdrop"
      onClick={onClose}
    >
      <div
        className="mc-glass-pill w-full max-w-sm rounded-[28px] p-5 shadow-2xl mc-modal-card"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="mb-3 text-lg font-bold text-white">{title}</h3>
        <form onSubmit={submit}>
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Nome da conversa"
            spellCheck={true}
            lang="pt-BR, en"
            autoCorrect="on"
            autoCapitalize="sentences"
            className="w-full rounded-2xl bg-white/10 px-4 py-3 text-[16px] text-white outline-none border border-white/15 focus:border-accentPurpleLight transition-colors"
          />
          <div className="mt-4 flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-full px-4 py-2 text-sm text-textMuted active:bg-white/10 transition-colors"
            >
              Cancelar
            </button>
            <button
              type="submit"
              disabled={!name.trim() || saving}
              className="rounded-full bg-accentPurple px-5 py-2 text-sm font-semibold text-white shadow-md active:scale-95 disabled:opacity-40 transition-transform"
            >
              {saving ? 'Salvando…' : 'Salvar'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
