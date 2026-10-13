import React, { useRef, useState } from 'react';
import { Check, ChevronDown } from 'lucide-react';
import { tr, useLanguage } from '../i18n.js';
import { Sheet } from './Sheet.jsx';

// HTML selects open an OS-styled dialog on Android; keep choices inside the app.
export default function ChoiceField({ label, value, options, onChange }) {
  useLanguage();
  const [open, setOpen] = useState(false);
  const trigger = useRef(null);
  const selected = options.find(option => option.value === value);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  function move(event) {
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const buttons = [...event.currentTarget.querySelectorAll('[role="radio"]')];
    const index = buttons.indexOf(document.activeElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowUp' ? -1 : 1) + buttons.length) % buttons.length;
    buttons[next]?.focus();
  }
  return <>
    <button ref={trigger} type="button" aria-label={label} aria-haspopup="dialog" aria-expanded={open}
      onClick={() => setOpen(true)} className="mc-choice-trigger flex w-full items-center gap-3 rounded-2xl bg-composer px-4 py-3 text-left text-[16px]">
      <span className="min-w-0 flex-1 truncate">{selected?.label || value}</span><ChevronDown size={18} className="text-accentPurpleLight" />
    </button>
    {open && <Sheet title={label} onClose={close} className="mc-control-sheet">
      <div role="radiogroup" aria-label={label} onKeyDown={move} className="space-y-2 py-2">
        {options.map(option => <button key={option.value} type="button" role="radio" lang={option.lang}
          aria-checked={option.value === value} data-autofocus={option.value === value || undefined}
          onClick={() => { onChange(option.value); close(); }}
          className={`mc-choice-option flex min-h-14 w-full items-center gap-3 rounded-2xl px-4 py-3 text-left ${option.value === value ? 'bg-accentPurple/20 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
          <span className="min-w-0 flex-1"><span className="block">{option.label}</span>{option.detail && <span className="block text-xs text-textMuted">{option.detail}</span>}</span>
          {option.value === value && <Check size={20} className="text-accentPurpleLight" aria-hidden="true" />}
        </button>)}
      </div>
      <button type="button" onClick={close} className="w-full rounded-full bg-composer py-3">{tr('Cancelar')}</button>
    </Sheet>}
  </>;
}
