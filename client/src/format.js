// Parser for the scene format:
//   *actions and narration in italics*      (third person, between asterisks)
//   Name: speech (optional tone)            (speech in plain text, with the character name)
// Tolerates incomplete markers (streaming): an unmatched "*" opens an italic run to the end of the
// line, so no raw asterisk shows up and the style does not change abruptly when the pair arrives.
// Nothing is interpreted as HTML: the result is plain data (React escapes everything).

const NAME = String.raw`\p{Lu}[\p{L}\p{M}'’.\-]*(?:\s+(?:(?:de|da|do|das|dos|di|du|van|von|la|le|el|al)\s+)?\p{Lu}[\p{L}\p{M}'’.\-]*){0,2}`;
const PLAIN = new RegExp(`^(${NAME})\\s*:\\s*(.*)$`, 'u');
const WRAPPED = new RegExp(`^\\*{1,2}(${NAME})\\*{1,2}\\s*:\\s*\\*{0,2}\\s*(.*)$`, 'u');
const WRAPPED_INSIDE = new RegExp(`^\\*{1,2}(${NAME}):\\*{1,2}\\s*(.*)$`, 'u');

// Splits a text into segments {i: italic?, t: text}. Runs of "*" count as a single marker.
export function inlineSegments(text) {
  const parts = String(text).replace(/\*{2,}/g, '*').split('*');
  const segs = [];
  parts.forEach((p, idx) => {
    if (p === '') return;
    segs.push({ i: idx % 2 === 1, t: p });
  });
  return segs;
}

function extractTone(text) {
  const m = text.match(/\s*\(([^()]{1,60})\)\s*$/);
  if (!m) return { text, tone: null };
  // only treated as tone when it is at the end and is not the whole speech
  const rest = text.slice(0, m.index);
  if (!rest.trim()) return { text, tone: null };
  return { text: rest, tone: m[1].trim() };
}

export function parseLine(line) {
  const raw = line.replace(/\s+$/, '');
  if (!raw.trim()) return null;
  const trimmed = raw.trimStart();
  let m = trimmed.match(WRAPPED) || trimmed.match(WRAPPED_INSIDE) || trimmed.match(PLAIN);
  if (m) {
    const name = m[1].trim();
    const body = m[2];
    const { text, tone } = extractTone(body);
    return { kind: 'speech', name, tone, segments: inlineSegments(text.trim()) };
  }
  const segments = inlineSegments(trimmed);
  const allItalic = segments.length > 0 && segments.every((s) => s.i);
  return { kind: allItalic ? 'action' : 'narration', segments };
}

export function parseScene(text) {
  const lines = String(text || '').split(/\r?\n/);
  const out = [];
  for (const l of lines) {
    const p = parseLine(l);
    if (p) out.push(p);
  }
  return out;
}

// Plain text (no markup) of a reply, for copy/search/preview.
export function plainText(text) {
  return String(text || '').replace(/\*{1,3}/g, '').replace(/\s+\n/g, '\n').trim();
}

// Speaker names in order of appearance (for side avatars).
export function speakersOf(text) {
  const seen = [];
  for (const b of parseScene(text)) if (b.kind === 'speech' && !seen.includes(b.name)) seen.push(b.name);
  return seen;
}
