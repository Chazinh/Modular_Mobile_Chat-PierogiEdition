import React, { useLayoutEffect, useRef } from 'react';
import { ChevronLeft } from 'lucide-react';
import { useLayer } from '../nav.js';
import { usePanelDrag } from '../gestures.js';
import { PanelMotion } from '../drawer-motion.js';
import '../drawer.css';

// Glass bottom sheet. Springs up and is dragged down by its handle (rubber band when pulled up).
// The Back button closes the sheet before anything else.
export function Sheet({ onClose, title, children, full = false }) {
  const panelRef = useRef(null);
  const scrimRef = useRef(null);
  const grabRef = useRef(null);
  const motionRef = useRef(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useLayoutEffect(() => {
    const panel = panelRef.current;
    const motion = new PanelMotion({ panel, scrim: scrimRef.current, axis: 'y', closed: panel.offsetHeight + 64, onClosed: () => onCloseRef.current() });
    motionRef.current = motion;
    motion.open();
    return () => {
      motion.destroy();
      motionRef.current = null;
    };
  }, []);
  const close = () => {
    const motion = motionRef.current;
    if (!motion) return onClose();
    motion.closed = panelRef.current.offsetHeight + 64; // a altura pode ter mudado desde a abertura
    motion.close();
  };
  usePanelDrag(grabRef, motionRef, { axis: 'y' });
  useLayer(true, onClose);
  return (
    <div className="mc-sheet-overlay">
      <div ref={scrimRef} className="mc-sheet-scrim" onClick={close} />
      <div ref={panelRef} className={`mc-sheet mc-glass ${full ? 'mc-sheet-full' : ''}`}>
        <div ref={grabRef} className="mc-sheet-grab">
          <span className="mc-grabber" />
          {title && <p className="mx-2 mb-1 text-sm font-semibold text-textMuted">{title}</p>}
        </div>
        <div className="mc-sheet-body">{children}</div>
      </div>
    </div>
  );
}

export function SheetButton({ icon, label, onClick, danger, detail, disabled }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`flex w-full items-center gap-4 rounded-2xl px-4 py-3.5 text-left text-[16px] active:bg-cardDarkHover disabled:opacity-40 ${danger ? 'text-red-400' : ''}`}
    >
      {icon}
      <span className="flex-1">{label}</span>
      {detail && <span className="text-sm text-textMuted">{detail}</span>}
    </button>
  );
}

// Stacked full screen (editing, cast, memory...). Back closes only this screen.
export function Screen({ title, onClose, action, children }) {
  useLayer(true, onClose);
  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-bgDark">
      {/* Detached floating header with liquid-glass bubbles */}
      <header className="pointer-events-none fixed inset-x-0 top-0 z-30 px-3" style={{ paddingTop: 'calc(var(--sat) + 8px)' }}>
        <div className="mc-col flex items-center gap-2">
          <button
            onClick={onClose}
            className="mc-glass-pill pointer-events-auto flex h-11 w-11 shrink-0 items-center justify-center rounded-full shadow-lg active:scale-90 transition-transform"
            aria-label="Voltar"
          >
            <ChevronLeft size={24} />
          </button>
          <div className="mc-glass-pill pointer-events-auto flex items-center rounded-full px-4 py-2.5 shadow-lg min-w-0 flex-1">
            <h2 className="truncate text-[16px] font-semibold">{title}</h2>
          </div>
          {action && (
            <div className="mc-glass-pill pointer-events-auto flex items-center rounded-full px-3 py-1.5 shadow-lg shrink-0">
              {action}
            </div>
          )}
        </div>
      </header>
      <div
        className="flex-1 overflow-y-auto"
        style={{
          paddingTop: 'calc(var(--sat) + 68px)',
          paddingBottom: 'calc(var(--sab) + 16px)',
        }}
      >
        <div className="mc-col">{children}</div>
      </div>
    </div>
  );
}
