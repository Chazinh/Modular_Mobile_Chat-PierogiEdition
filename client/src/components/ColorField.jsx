import React, { useState } from 'react';
import { Palette } from 'lucide-react';
import { tr, useLanguage } from '../i18n.js';
import { validHex, hexToHsl, hslToHex } from '../color.js';
import { Sheet } from './Sheet.jsx';

export default function ColorField({ value, active, onChange }) {
  useLanguage();
  const [open, setOpen] = useState(false);
  const [hex, setHex] = useState(value);
  const [hsl, setHsl] = useState(() => hexToHsl(value));
  const valid = validHex(hex);
  function begin() { setHex(value); setHsl(hexToHsl(value)); setOpen(true); }
  function enter(value) { setHex(value); if (validHex(value)) setHsl(hexToHsl(value)); }
  function adjust(key, value) { const next = { ...hsl, [key]: Number(value) }; setHsl(next); setHex(hslToHex(next)); }
  return <>
    <button type="button" onClick={begin} aria-label={tr('Escolher outra cor')} aria-haspopup="dialog" aria-expanded={open} aria-pressed={active}
      className={`flex items-center gap-2 rounded-full px-3 py-1.5 text-sm ${active ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
      <span className="h-5 w-5 rounded-full border border-white/20" style={{ background: value }} /><Palette size={14} />{tr('Outra cor')}
    </button>
    {open && <Sheet title={tr('Cor personalizada')} onClose={() => setOpen(false)} className="mc-control-sheet">
      <div className="space-y-4 px-2 py-3">
        <p className="mc-read rounded-2xl bg-composer p-4" style={{ color: valid ? hex : value }}>{tr('Prévia da cor das letras')}</p>
        <label className="block space-y-2"><span className="text-sm text-textMuted">{tr('Código da cor')}</span>
          <input data-autofocus value={hex} onChange={event => enter(event.target.value)} maxLength={7} spellCheck={false} autoCapitalize="none" autoCorrect="off"
            aria-label={tr('Código da cor')} aria-invalid={!valid} aria-describedby={!valid ? 'mc-color-error' : undefined}
            className="w-full rounded-2xl bg-composer px-4 py-3 font-mono text-[16px] outline-none focus:ring-1 focus:ring-accentPurpleLight" />
        </label>
        {!valid && <p id="mc-color-error" role="alert" className="text-sm text-red-300">{tr('Use uma cor no formato #RRGGBB.')}</p>}
        {[['h', 'Matiz', 360, 'linear-gradient(to right, #ff0000, #ffff00, #00ff00, #00ffff, #0000ff, #ff00ff, #ff0000)'],
          ['s', 'Saturação', 100, `linear-gradient(to right, hsl(${hsl.h} 0% ${hsl.l}%), hsl(${hsl.h} 100% ${hsl.l}%))`],
          ['l', 'Luminosidade', 100, `linear-gradient(to right, #000, hsl(${hsl.h} ${hsl.s}% 50%), #fff)`]].map(([key, label, max, background]) =>
          <label key={key} className="block space-y-2"><span className="flex justify-between text-sm"><span>{tr(label)}</span><span className="font-mono text-textMuted">{Math.round(hsl[key])}{key === 'h' ? '°' : '%'}</span></span>
            <input type="range" min="0" max={max} step="1" value={hsl[key]} onChange={event => adjust(key, event.target.value)} aria-label={tr(label)}
              className="mc-color-slider w-full" style={{ background }} />
          </label>)}
        <div className="flex gap-3 pt-2">
          <button type="button" onClick={() => setOpen(false)} className="flex-1 rounded-full bg-composer py-3">{tr('Cancelar')}</button>
          <button type="button" disabled={!valid} onClick={() => { onChange(hex.toLowerCase()); setOpen(false); }} className="flex-1 rounded-full bg-accentPurple py-3 font-semibold text-white disabled:opacity-40">{tr('Aplicar cor')}</button>
        </div>
      </div>
    </Sheet>}
  </>;
}
