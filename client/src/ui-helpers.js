import { tr } from './i18n.js';

// navigator.clipboard only exists in a secure context; outside it the legacy method is used.
export async function copyText(text) {
  if (typeof navigator !== 'undefined' && navigator.clipboard && typeof window !== 'undefined' && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      /* falls back */
    }
  }
  if (typeof document === 'undefined') return false;
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
  if (diff < 60) return tr('agora');
  const min = Math.floor(diff / 60);
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d} d`;
  const mo = Math.floor(d / 30);
  if (mo < 12) return tr('{0} {1}', [mo, mo > 1 ? tr('meses') : tr('mês')]);
  return tr('{0} ano(s)', [Math.floor(mo / 12)]);
}

export const fold = (s) => String(s || '').normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().trim();

export function findCast(cast, name) {
  const k = fold(name);
  if (!k || !cast) return null;
  return (
    cast.find((c) => fold(c.name) === k || (c.aliases || []).some((a) => fold(a) === k)) ||
    cast.find((c) => fold(c.name).startsWith(`${k} `) || k.startsWith(`${fold(c.name)} `)) ||
    null
  );
}
