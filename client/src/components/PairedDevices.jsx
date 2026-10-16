import React, { useEffect, useRef, useState } from 'react';
import { Loader2, Trash2 } from 'lucide-react';
import { api } from '../api.js';
import { tr, useLanguage } from '../i18n.js';

const deviceKey = (origin, id) => JSON.stringify([origin?.kind, origin?.server, id]);

// Both settings entry points must use the same mutation, origin and live-list lifecycle.
export default function PairedDevices() {
  useLanguage();
  const [devices, setDevices] = useState(null);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [removing, setRemoving] = useState('');
  const read = useRef(0), originRef = useRef(null), removal = useRef('');
  const removed = useRef(new Set());

  async function load() {
    const revision = ++read.current;
    let origin;
    try {
      const result = await api.devices({ onTarget: target => { origin = { server: target.server, kind: target.kind }; } });
      if (revision !== read.current) return;
      originRef.current = origin;
      setDevices(result.devices.filter(device => !removed.current.has(deviceKey(origin, device.id))));
      setError('');
    } catch (failure) {
      if (revision === read.current) setError(failure.message);
    }
  }

  async function remove(device) {
    const origin = originRef.current;
    if (!origin || removal.current) return;
    removal.current = device.id;
    read.current++; // An older GET cannot put the removed item back.
    setRemoving(device.id);
    setMessage(''); setError('');
    const sameOrigin = () => originRef.current?.server === origin.server && originRef.current?.kind === origin.kind;
    try {
      const result = await api.revokeDevice(device.id, { origin });
      removed.current.add(deviceKey(origin, device.id));
      if (sameOrigin()) {
        setDevices(current => (result.devices || current)?.filter(item => !removed.current.has(deviceKey(origin, item.id))));
        setMessage(tr('Pareamento removido: {0}.', [device.name]));
      }
      if (originRef.current) await load();
    } catch (failure) {
      if (sameOrigin()) setError(failure.message);
    } finally { removal.current = ''; setRemoving(''); }
  }

  useEffect(() => {
    load();
    const changed = () => {
      read.current++; originRef.current = null;
      setDevices(null); setMessage(''); setError('');
      load();
    };
    const updated = event => { if (event.detail?.type === 'devices' && !removal.current) load(); };
    const resumed = () => { if (document.visibilityState !== 'hidden') load(); };
    window.addEventListener('mc:connection-change', changed);
    window.addEventListener('mc:event', updated);
    window.addEventListener('mc:resume', resumed);
    document.addEventListener('visibilitychange', resumed);
    return () => {
      read.current++; originRef.current = null;
      window.removeEventListener('mc:connection-change', changed);
      window.removeEventListener('mc:event', updated);
      window.removeEventListener('mc:resume', resumed);
      document.removeEventListener('visibilitychange', resumed);
    };
  }, []);

  const active = devices?.filter(device => !device.revoked_at && !removed.current.has(deviceKey(originRef.current, device.id))) || [];
  return <>
    {message && <p role="status" className="rounded-xl bg-green-500/10 px-3 py-2 text-xs text-green-300">{message}</p>}
    {error && <p role="alert" className="rounded-xl bg-red-500/10 px-3 py-2 text-xs text-red-300">{tr(error)}</p>}
    {devices && <p aria-live="polite" className="text-xs text-textMuted">{tr('Pareamentos ativos: {0}', [active.length])}</p>}
    {active.map(device => <div key={device.id} className="flex items-center justify-between rounded-xl bg-composer px-3 py-2 text-sm">
      <span className="min-w-0 break-words">{device.name}<span className="block text-[11px] text-textMuted">{tr('visto ')}{device.last_seen_at ? new Date(device.last_seen_at).toLocaleString() : tr('nunca')}{' · '}{tr('ID {0}', [device.id.slice(-6)])}</span></span>
      <button type="button" onClick={() => remove(device)} disabled={!!removing}
        aria-label={tr('Remover pareamento de {0}', [device.name])} aria-busy={removing === device.id}
        className="ml-2 flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-red-300 active:bg-red-500/15 disabled:opacity-40">
        {removing === device.id ? <Loader2 size={18} className="animate-spin" /> : <Trash2 size={18} />}
      </button>
    </div>)}
  </>;
}
