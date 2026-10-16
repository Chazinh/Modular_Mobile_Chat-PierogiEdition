import React, { useCallback, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown } from 'lucide-react';
import { tr, useLanguage } from '../i18n.js';
import { useLayer } from '../nav.js';
import { Sheet } from './Sheet.jsx';

// HTML selects open an OS-styled dialog on Android; keep choices inside the app.
export default function ChoiceField({ variant = 'sheet', ...props }) {
  return variant === 'dropdown' ? <AnchoredChoiceField {...props} /> : <SheetChoiceField {...props} />;
}

function AnchoredChoiceField({ label, value, options, onChange }) {
  useLanguage();
  const id = useId();
  const trigger = useRef(null);
  const list = useRef(null);
  const initialFocus = useRef(0);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const selectedIndex = options.findIndex(option => option.value === value);
  const close = useCallback((restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) trigger.current?.focus({ preventScroll: true });
  }, []);
  useLayer(open, close);

  function show(index = Math.max(0, selectedIndex)) {
    initialFocus.current = index;
    setActiveIndex(index);
    setOpen(true);
  }

  useLayoutEffect(() => {
    if (!open) return;
    const button = trigger.current, panel = list.current;
    const viewport = window.visualViewport;
    // Settings cards sit inside a scrolling screen. A portal avoids ancestor clipping.
    const clippers = [], ancestors = [];
    for (let parent = button.parentElement; parent; parent = parent.parentElement) {
      ancestors.push(parent);
      const style = getComputedStyle(parent);
      if (/(auto|scroll|hidden|clip)/.test(style.overflowX + style.overflowY)) clippers.push(parent);
    }
    let frame = 0;
    const position = () => {
      frame = 0;
      const rect = button.getBoundingClientRect();
      const style = getComputedStyle(panel);
      const left = (viewport?.offsetLeft || 0) + parseFloat(style.scrollPaddingLeft);
      const right = (viewport?.offsetLeft || 0) + (viewport?.width || innerWidth) - parseFloat(style.scrollPaddingRight);
      const top = (viewport?.offsetTop || 0) + parseFloat(style.scrollPaddingTop);
      const bottom = (viewport?.offsetTop || 0) + (viewport?.height || innerHeight) - parseFloat(style.scrollPaddingBottom);
      let visibleTop = top, visibleBottom = bottom, visibleLeft = left, visibleRight = right;
      for (const parent of clippers) {
        const clip = parent.getBoundingClientRect();
        visibleTop = Math.max(visibleTop, clip.top); visibleBottom = Math.min(visibleBottom, clip.bottom);
        visibleLeft = Math.max(visibleLeft, clip.left); visibleRight = Math.min(visibleRight, clip.right);
      }
      if (rect.bottom <= visibleTop || rect.top >= visibleBottom || rect.right <= visibleLeft || rect.left >= visibleRight) {
        close(); return;
      }
      const width = Math.min(rect.width, right - left);
      panel.style.width = `${width}px`;
      const below = bottom - rect.bottom - 6, above = rect.top - top - 6;
      const upwards = below < Math.min(280, panel.scrollHeight) && above > below;
      const height = Math.min(280, panel.scrollHeight, Math.max(0, upwards ? above : below));
      if (height < 32 || width <= 0) { close(); return; }
      panel.style.left = `${Math.max(left, Math.min(rect.left, right - width))}px`;
      panel.style.top = `${upwards ? rect.top - 6 - height : rect.bottom + 6}px`;
      panel.style.maxHeight = `${height}px`;
      panel.style.visibility = 'visible';
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(position); };
    const outside = event => {
      if (!button.contains(event.target) && !panel.contains(event.target)) close(event.type !== 'focusin');
    };
    const escape = event => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopPropagation(); close();
    };
    position();
    const option = panel.querySelectorAll('[role="option"]')[initialFocus.current];
    option?.focus({ preventScroll: true });
    if (option) panel.scrollTop = Math.max(0, option.offsetTop + option.offsetHeight - panel.clientHeight + 8);
    // Click runs after the pointer's default focus change, so a tap on plain text
    // cannot undo restoration. Focusing another control dismisses without stealing focus.
    document.addEventListener('click', outside, true);
    document.addEventListener('focusin', outside);
    document.addEventListener('keydown', escape, true);
    window.addEventListener('scroll', schedule, true);
    window.addEventListener('resize', schedule);
    viewport?.addEventListener('scroll', schedule);
    viewport?.addEventListener('resize', schedule);
    const observer = new ResizeObserver(schedule);
    observer.observe(button);
    // Drawer springs move ancestors with transforms without resizing the trigger.
    const motionObserver = new MutationObserver(schedule);
    for (const parent of ancestors) motionObserver.observe(parent, { attributes: true, attributeFilter: ['style', 'class'] });
    return () => {
      observer.disconnect(); motionObserver.disconnect(); cancelAnimationFrame(frame);
      document.removeEventListener('click', outside, true);
      document.removeEventListener('focusin', outside);
      document.removeEventListener('keydown', escape, true);
      window.removeEventListener('scroll', schedule, true);
      window.removeEventListener('resize', schedule);
      viewport?.removeEventListener('scroll', schedule);
      viewport?.removeEventListener('resize', schedule);
      if (panel.contains(document.activeElement) && button.isConnected) button.focus({ preventScroll: true });
    };
  }, [open, close]);

  function move(event) {
    if (event.key === 'Tab') { close(); return; }
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const buttons = [...list.current.querySelectorAll('[role="option"]')];
    const index = buttons.indexOf(document.activeElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowUp' ? -1 : 1) + buttons.length) % buttons.length;
    const target = buttons[next];
    target?.focus({ preventScroll: true });
    if (target) {
      const panel = list.current;
      if (target.offsetTop < panel.scrollTop + 8) panel.scrollTop = Math.max(0, target.offsetTop - 8);
      else if (target.offsetTop + target.offsetHeight > panel.scrollTop + panel.clientHeight - 8) panel.scrollTop = target.offsetTop + target.offsetHeight - panel.clientHeight + 8;
    }
  }

  return <>
    <button ref={trigger} id={`${id}-trigger`} type="button" aria-label={label} aria-haspopup="listbox" aria-expanded={open}
      aria-controls={open ? `${id}-listbox` : undefined} disabled={!options.length}
      onKeyDown={event => {
        if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        show(event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1 : Math.max(0, selectedIndex));
      }}
      onClick={() => open ? close() : show()} className="mc-choice-trigger flex w-full items-center gap-3 rounded-2xl bg-composer px-4 py-3 text-left text-[16px]">
      <span className="min-w-0 flex-1 truncate">{options[selectedIndex]?.label || value}</span><ChevronDown size={18} className="text-accentPurpleLight" aria-hidden="true" />
    </button>
    {open && createPortal(<div ref={list} id={`${id}-listbox`} role="listbox" aria-labelledby={`${id}-trigger`}
      onKeyDown={move} className="mc-choice-listbox mc-glass space-y-1">
      {options.map((option, index) => <button key={option.value} type="button" role="option" lang={option.lang}
        aria-selected={option.value === value} tabIndex={index === activeIndex ? 0 : -1} onFocus={() => setActiveIndex(index)}
        onClick={() => { onChange(option.value); close(); }}
        className={`mc-choice-option flex min-h-14 w-full items-center gap-3 rounded-2xl px-4 py-3 text-left ${option.value === value ? 'bg-accentPurple/20 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
        <span className="min-w-0 flex-1"><span className="block">{option.label}</span>{option.detail && <span className="block text-xs text-textMuted">{option.detail}</span>}</span>
        {option.value === value && <Check size={20} className="text-accentPurpleLight" aria-hidden="true" />}
      </button>)}
    </div>, document.body)}
  </>;
}

function SheetChoiceField({ label, value, options, onChange }) {
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
