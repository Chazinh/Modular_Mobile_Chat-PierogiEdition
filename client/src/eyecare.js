// Eye care (per device, stored in the browser/APK itself):
//  - light text brightness (50–100%): dims white text without touching the background;
//  - text color: neutral, soft, cream, amber, sepia, aqua or a custom color;
//  - blue-light filter (0–60%): tints the PALETTE (backgrounds, surfaces, borders, accents and text) with three
//    per-channel factors (--mc-tr/--mc-tg/--mc-tb), reducing blue. Same result as a multiplied amber overlay,
//    but without a full-screen blend layer (cheap for the WebView GPU);
//  - paper texture (0–100%): fine, subtle grain over the screen that mimics the roughness of paper and
//    breaks the smooth shine of the glass (SVG-generated noise tile, no image file). Static layer:
//    opacity only, no blending and no will-change.
// Everything becomes CSS variables (--mc-*) used by index.css and tailwind.config.mjs; no JavaScript in the text rendering path.
const KEY = 'mc.eyecare';

// Base colors (100% brightness) of each preset.
export const PRESETS = {
  neutro: { label: 'Neutro', text: '#e7e7ea', name: '#f1f1f4', action: '#a1a1aa', tone: '#8d8d96', white: '#ffffff', muted: '#9a9aa3' },
  suave: { label: 'Branco suave', text: '#d6d4cf', name: '#e2e0da', action: '#a09e98', tone: '#8a8882', white: '#e8e6e0', muted: '#96948e' },
  creme: { label: 'Creme', text: '#e6dcc6', name: '#efe6d2', action: '#b3a78e', tone: '#978c75', white: '#f1e9d6', muted: '#a39a86' },
  ambar: { label: 'Âmbar', text: '#efc98f', name: '#f6d6a3', action: '#b99562', tone: '#9c7b4f', white: '#f7dcae', muted: '#a98a5f' },
  sepia: { label: 'Sépia', text: '#d9c3a0', name: '#e5d0ae', action: '#a8946f', tone: '#8c7a5a', white: '#e9d6b6', muted: '#9c8a68' },
  verde: { label: 'Verde-água', text: '#c4e4d2', name: '#d3eedf', action: '#8fb3a0', tone: '#7a9a89', white: '#d9f0e5', muted: '#86a595' },
};
export const DEFAULTS = { preset: 'neutro', custom: '#e7e7ea', brightness: 100, blue: 0, paper: 0 };

const clamp = (v, a, b, d) => {
  const n = Number(v);
  return Number.isFinite(n) ? Math.min(b, Math.max(a, n)) : d;
};
const HEX = /^#[0-9a-f]{6}$/i;

export function normalize(raw = {}) {
  const preset = raw.preset === 'custom' || PRESETS[raw.preset] ? raw.preset : DEFAULTS.preset;
  return {
    preset,
    custom: HEX.test(raw.custom) ? raw.custom.toLowerCase() : DEFAULTS.custom,
    brightness: Math.round(clamp(raw.brightness, 50, 100, 100)),
    blue: Math.round(clamp(raw.blue, 0, 60, 0)),
    paper: Math.round(clamp(raw.paper, 0, 100, 0)),
  };
}

export function loadEyeCare() {
  try {
    return normalize(JSON.parse(localStorage.getItem(KEY) || '{}'));
  } catch {
    return { ...DEFAULTS };
  }
}

export function saveEyeCare(v) {
  try {
    localStorage.setItem(KEY, JSON.stringify(normalize(v)));
  } catch {
    /* no storage: applies to this session only */
  }
}

const toRgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
const toHex = (rgb) => `#${rgb.map((c) => Math.round(Math.min(255, Math.max(0, c))).toString(16).padStart(2, '0')).join('')}`;
export const scaleColor = (hex, f) => toHex(toRgb(hex).map((c) => c * f));
const mix = (hex, f, towards = 255) => toHex(toRgb(hex).map((c) => c + (towards - c) * f));

// From a single chosen color, derives the others while keeping contrast between name, speech and action.
function fromCustom(hex) {
  return { text: hex, name: mix(hex, 0.12), action: scaleColor(hex, 0.7), tone: scaleColor(hex, 0.6), white: mix(hex, 0.2), muted: scaleColor(hex, 0.72) };
}

// Blue-light filter tint. The old overlay was a multiplied rgba(255, g, b, A): each channel c became
// c * (1 - A + A * k / 255), with k = 255 (red), g (green) and b (blue). Here the factor comes straight from that formula.
export function blueTint(blue) {
  const a = clamp(blue, 0, 60, 0) / 100;
  if (a <= 0) return { r: 1, g: 1, b: 1 };
  const alpha = Math.min(0.75, a * 1.15);
  const f = (k) => Number((1 - alpha + (alpha * k) / 255).toFixed(4));
  return { r: 1, g: f(170 - 40 * a), b: f(80 - 40 * a) };
}
export const tintColor = (hex, t) => {
  if (!t || (t.r === 1 && t.g === 1 && t.b === 1)) return hex;
  const [r, g, b] = toRgb(hex);
  return toHex([r * t.r, g * t.g, b * t.b]);
};

export function colorsFor(value) {
  const v = normalize(value);
  const base = v.preset === 'custom' ? fromCustom(v.custom) : PRESETS[v.preset];
  const f = v.brightness / 100;
  const t = blueTint(v.blue);
  const out = {};
  for (const k of ['text', 'name', 'action', 'tone', 'white', 'muted']) out[k] = tintColor(scaleColor(base[k], f), t);
  // text on a button filled with the accent color: ignores brightness (only the blue-light tint)
  out.onAccent = tintColor('#ffffff', t);
  return out;
}

// Fine noise (fractalNoise) in a paper tone; the layer opacity sets the intensity.
const PAPER_SVG = "<svg xmlns='http://www.w3.org/2000/svg' width='220' height='220'><filter id='p' x='0' y='0' width='100%' height='100%'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='3' seed='11' stitchTiles='stitch'/><feColorMatrix type='matrix' values='0 0 0 0 0.93  0 0 0 0 0.9  0 0 0 0 0.84  0 0 0 1.6 -0.42'/></filter><rect width='100%' height='100%' filter='url(#p)'/></svg>";
export const PAPER_IMAGE = `url("data:image/svg+xml,${encodeURIComponent(PAPER_SVG)}")`;
export const PAPER_MAX_OPACITY = 0.24;

// Paper layer: created only when needed; static (opacity only), no mix-blend-mode or will-change.
function paperLayer() {
  if (typeof document === 'undefined') return null;
  let el = document.getElementById('mc-paper');
  if (!el) {
    el = document.createElement('div');
    el.id = 'mc-paper';
    el.setAttribute('aria-hidden', 'true');
    Object.assign(el.style, { position: 'fixed', inset: '0', pointerEvents: 'none', display: 'none', zIndex: '2147483647', backgroundSize: '220px 220px', backgroundRepeat: 'repeat', backgroundImage: PAPER_IMAGE });
    document.body.appendChild(el);
  }
  return el;
}

export function applyEyeCare(value = loadEyeCare()) {
  if (typeof document === 'undefined') return;
  const v = normalize(value);
  const c = colorsFor(v);
  const t = blueTint(v.blue);
  const root = document.documentElement.style;
  root.setProperty('--mc-text', c.text);
  root.setProperty('--mc-name', c.name);
  root.setProperty('--mc-action', c.action);
  root.setProperty('--mc-tone', c.tone);
  root.setProperty('--mc-white', c.white);
  root.setProperty('--mc-muted', c.muted);
  root.setProperty('--mc-onaccent', c.onAccent);
  // per-channel tint factors and the tinted pure white (for rgba(var(--mc-white-rgb), .1) in glass and borders)
  root.setProperty('--mc-tr', String(t.r));
  root.setProperty('--mc-tg', String(t.g));
  root.setProperty('--mc-tb', String(t.b));
  root.setProperty('--mc-white-rgb', toRgb(tintColor('#ffffff', t)).join(', '));
  // full-screen overlay (older versions): removed, the filter now lives in the colors
  const old = document.getElementById('mc-bluelight');
  if (old && old.remove) old.remove();
  if (v.paper > 0 || document.getElementById('mc-paper')) {
    const paper = paperLayer();
    if (paper) {
      // the higher the intensity, the more visible the grain (at most 0.24 opacity: text stays readable)
      paper.style.display = v.paper > 0 ? 'block' : 'none';
      paper.style.opacity = (Math.min(1, v.paper / 100) * PAPER_MAX_OPACITY).toFixed(3);
    }
  }
}

if (typeof window !== 'undefined' && typeof document !== 'undefined') {
  const run = () => applyEyeCare();
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', run, { once: true });
  else run();
}
