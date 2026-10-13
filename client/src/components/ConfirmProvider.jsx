import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { tr, useLanguage } from '../i18n.js';
import { Sheet } from './Sheet.jsx';

const Confirmation = createContext(null);

// Async confirmation preserves cancellation without an OS dialog.
export function ConfirmProvider({ children }) {
  useLanguage();
  const [pending, setPending] = useState(null);
  const resolve = useRef(null);
  const ask = useCallback(message => {
    if (resolve.current) return Promise.resolve(false);
    return new Promise(done => { resolve.current = done; setPending({ message }); });
  }, []);
  const finish = useCallback(accepted => { const done = resolve.current; resolve.current = null; setPending(null); done?.(accepted); }, []);
  useEffect(() => () => { resolve.current?.(false); resolve.current = null; }, []);
  return <Confirmation.Provider value={ask}>
    {children}
    {pending && <Sheet title={tr('Confirmar ação')} onClose={() => finish(false)} className="mc-control-sheet">
      <p className="whitespace-pre-wrap break-words px-3 py-4 text-[16px]">{pending.message}</p>
      <div className="flex gap-3 px-2 pb-2">
        <button type="button" data-autofocus onClick={() => finish(false)} className="flex-1 rounded-full bg-composer py-3">{tr('Cancelar')}</button>
        <button type="button" onClick={() => finish(true)} className="flex-1 rounded-full bg-red-500/20 py-3 font-semibold text-red-300">{tr('Confirmar')}</button>
      </div>
    </Sheet>}
  </Confirmation.Provider>;
}

export function useConfirm() {
  const ask = useContext(Confirmation);
  if (!ask) throw new Error('Confirmation provider missing');
  return ask;
}
