import React, { useEffect, useRef, useState } from 'react';
import { api, config, isNative } from '../api.js';
import { synchronizePairedPc } from '../pc-pairing.js';
import { tr, useLanguage } from '../i18n.js';

// Return to the paired PC without generating a new code or replaying a message.
export default function PcConnectionButton() {
  useLanguage();
  const [connecting, setConnecting] = useState(false);
  const [message, setMessage] = useState('');
  const action = useRef(null);
  useEffect(() => () => action.current?.abort(), []);
  if (!isNative() || config.kind === 'pc' || !(config.connection.local || config.connection.remote)) return null;
  async function connect() {
    if (action.current) return;
    const controller = action.current = new AbortController();
    setConnecting(true); setMessage('');
    try {
      await synchronizePairedPc({ connect: signal => api.switchToPc(signal), signal: controller.signal,
        onWaiting: () => setMessage(tr('Pareamento salvo. Aguardando trabalho ativo para conectar ao PC…')) });
    } catch (error) { if (!controller.signal.aborted) setMessage(error.message); }
    finally { if (action.current === controller) { action.current = null; if (!controller.signal.aborted) setConnecting(false); } }
  }
  return <div className="mt-2">
    <button type="button" onClick={connect} disabled={connecting} className="min-h-11 rounded-full bg-accentPurple px-4 py-2 text-xs font-semibold text-white disabled:opacity-60">{tr(connecting ? 'Conectando ao PC…' : 'Conectar ao PC')}</button>
    {connecting && <button type="button" className="ml-3 min-h-11 text-xs underline" onClick={() => { action.current?.abort(); action.current = null; setConnecting(false); setMessage(''); }}>{tr('Cancelar')}</button>}
    {message && <p role="status" className="mt-1 break-words text-xs">{tr(message)}</p>}
  </div>;
}
