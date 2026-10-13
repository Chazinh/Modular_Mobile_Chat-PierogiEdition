import React, { useCallback, useEffect, useMemo, useRef } from 'react';
import { parseScene } from './format.js';

// Stable callback that always calls the latest version of the function: lets children be memoized without new closures.
export function useEvent(fn) {
  const ref = useRef(fn);
  ref.current = fn;
  return useCallback((...args) => ref.current(...args), []);
}

// Long press (iOS via timer, Android/desktop via contextmenu); fires once and suppresses the following click.
export function useLongPress(onLongPress, ms = 500) {
  const timer = useRef(null);
  const fired = useRef(false);
  const trigger = () => {
    if (fired.current) return;
    fired.current = true;
    if (navigator.vibrate) navigator.vibrate(15);
    onLongPress();
  };
  const cancel = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  return {
    onTouchStart: () => {
      fired.current = false;
      cancel();
      timer.current = setTimeout(trigger, ms);
    },
    onTouchEnd: cancel,
    onTouchCancel: cancel,
    onTouchMove: cancel,
    onContextMenu: (e) => {
      e.preventDefault();
      cancel();
      trigger();
      setTimeout(() => (fired.current = false), 400);
    },
    onClickCapture: (e) => {
      if (fired.current) {
        e.stopPropagation();
        e.preventDefault();
        fired.current = false;
      }
    },
  };
}

// navigator.clipboard only exists in a secure context; outside it the legacy method is used.
export async function copyText(text) {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      /* falls back */
    }
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.cssText = 'position:fixed;top:0;left:0;opacity:0;';
  document.body.appendChild(ta);
  ta.select();
  ta.setSelectionRange(0, text.length);
  let ok = false;
  try {
    ok = document.execCommand('copy');
  } catch {
    ok = false;
  }
  document.body.removeChild(ta);
  return ok;
}

export function timeAgo(iso) {
  if (!iso) return '';
  const diff = (Date.now() - new Date(iso).getTime()) / 1000;
  if (diff < 60) return 'agora';
  const min = Math.floor(diff / 60);
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d} d`;
  const mo = Math.floor(d / 30);
  if (mo < 12) return `${mo} ${mo > 1 ? 'meses' : 'mês'}`;
  return `${Math.floor(mo / 12)} ano(s)`;
}

export const fold = (s) => String(s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim();

export function findCast(cast, name) {
  const k = fold(name);
  if (!k || !cast) return null;
  return (
    cast.find((c) => fold(c.name) === k || (c.aliases || []).some((a) => fold(a) === k)) ||
    cast.find((c) => fold(c.name).startsWith(`${k} `) || k.startsWith(`${fold(c.name)} `)) ||
    null
  );
}

const COLORS = ['#7c3aed', '#db2777', '#0891b2', '#ea580c', '#16a34a', '#4f46e5', '#ca8a04'];

export function Avatar({ name, avatar, size = 56, round = false }) {
  const radius = round ? 'rounded-full' : 'rounded-2xl';
  if (avatar && (avatar.startsWith('http') || avatar.startsWith('data:image/'))) {
    return <img src={avatar} alt={name || ''} className={`shrink-0 ${radius} object-cover border border-white/10`} style={{ width: size, height: size }} />;
  }
  const label = (name || '?').trim();
  let hash = 0;
  for (const ch of label) hash = (hash * 31 + ch.codePointAt(0)) >>> 0;
  const initial = [...label][0]?.toUpperCase() || '?';
  return (
    <div
      className={`shrink-0 ${radius} flex items-center justify-center font-bold text-white shadow-sm`}
      style={{ width: size, height: size, background: COLORS[hash % COLORS.length], fontSize: size * 0.42 }}
    >
      {initial}
    </div>
  );
}

// Shrinks the chosen image into a small local avatar (JPEG data URL).
export function fileToAvatar(file, size = 256) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(file);
    img.onload = () => {
      const s = Math.min(img.width, img.height);
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = size;
      canvas.getContext('2d').drawImage(img, (img.width - s) / 2, (img.height - s) / 2, s, s, 0, 0, size, size);
      URL.revokeObjectURL(url);
      resolve(canvas.toDataURL('image/jpeg', 0.85));
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error('Imagem inválida.'));
    };
    img.src = url;
  });
}

function Segs({ segments }) {
  return segments.map((s, i) =>
    s.i ? (
      <em key={i} className="mc-action">
        {s.t}
      </em>
    ) : (
      <React.Fragment key={i}>{s.t}</React.Fragment>
    ),
  );
}

// Renders a scene reply: actions in gray italics, "Name: text" speech without a side avatar.
// Safe by construction: React escapes all text and nothing external is loaded.
export function SceneText({ text, cast, streaming }) {
  const lines = useMemo(() => parseScene(text), [text]);
  const rows = [];
  lines.forEach((l, i) => {
    if (l.kind === 'speech') {
      rows.push(
        <p key={i} className="break-words leading-relaxed">
          <span className="mc-name">{l.name}:</span> <Segs segments={l.segments} />
          {l.tone ? <span className="mc-tone"> ({l.tone})</span> : null}
        </p>,
      );
    } else {
      rows.push(
        <p key={i} className={`break-words leading-relaxed ${l.kind === 'action' ? 'mc-action' : ''}`}>
          <Segs segments={l.segments} />
        </p>,
      );
    }
  });
  return (
    <div className="space-y-2.5">
      {rows}
      {streaming ? <span className="streaming-cursor" /> : null}
    </div>
  );
}

// User text: same grammar (italics between asterisks), no avatars.
export function UserText({ text }) {
  const lines = useMemo(() => parseScene(text), [text]);
  return (
    <div className="space-y-1.5">
      {lines.map((l, i) => (
        <p key={i} className="break-words leading-relaxed">
          {l.kind === 'speech' ? <span className="mc-name">{l.name}: </span> : null}
          <Segs segments={l.segments} />
          {l.tone ? <span className="mc-tone"> ({l.tone})</span> : null}
        </p>
      ))}
    </div>
  );
}
