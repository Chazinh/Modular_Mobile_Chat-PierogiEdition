// Text color diagnostics (Eye care): opens the client build in headless Edge, with a fictitious HTTP server
// and a disposable profile, walks every screen per preset/brightness and lists the text nodes whose color does NOT derive
// from the --mc-* palette (semantic colors are ignored: error red, warning yellow, success green, accent purple
// and text on a filled colored surface). Also checks the input caret and, at the end, the blue-light
// filter (palette tint, no blend layer) and the paper texture (static layer, opacity only).
// Usage: MC_UI_DIST=<build folder> MC_TC_OUT=<temp folder> node scripts/verify-text-colors.mjs
// Optional: MC_EDGE (msedge.exe), MC_TC_PRESETS (e.g. neutro,ambar,custom), MC_TC_BRIGHTNESS (e.g. 100,60),
// MC_TC_CUSTOM (e.g. #ffb36b), MC_TC_SHOTS=1 (one screenshot per screen).
// Output: console report and full JSON in <MC_TC_OUT>/text-colors.json. Exit code 1 for nodes outside the palette or page errors.
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.resolve(process.env.MC_TC_OUT || (await fs.mkdtemp(path.join(os.tmpdir(), 'mc-text-colors-'))));
await fs.mkdir(out, { recursive: true });
const profile = path.join(out, 'edge-profile');
await fs.mkdir(profile, { recursive: true });
const dist = path.resolve(process.env.MC_UI_DIST || path.join(root, 'client/dist'));
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
const CUSTOM = (process.env.MC_TC_CUSTOM || '#ffb36b').toLowerCase();
// Preset labels from client/src/eyecare.js (kept here so the script does not import client code).
const PRESET_LABEL = { neutro: 'Neutro', suave: 'Branco suave', creme: 'Creme', ambar: 'Âmbar', sepia: 'Sépia', verde: 'Verde-água' };
const PRESET_IDS = (process.env.MC_TC_PRESETS || 'neutro,suave,creme,ambar,sepia,verde,custom').split(',').map((s) => s.trim()).filter(Boolean);
const BRIGHTNESS = (process.env.MC_TC_BRIGHTNESS || '100,60').split(',').map((s) => Number(s.trim()));
const SHOTS = process.env.MC_TC_SHOTS === '1';
const DRAWER_ROWS = ['Modelo e esforço', 'Persona', 'Nome do chat', 'Personalização (.txt)', 'Contexto', 'Estilo e limites (global)', 'Histórico', 'Elenco'];
const MEMORY = {
  local: { chat: { ok: true }, embed: { ok: true } },
  sync: { synced: true, lag: 0 },
  scene: { location: 'Biblioteca', time: 'Noite', mood: 'Tensa', present: ['Mira'], objects: ['carta'], characters: [] },
  timeline: { days: [] },
  memories: [{ id: 'm1', kind: 'EVENT', subject: 'Mira', content: 'Encontrou uma carta.', visibility: 'private', pinned: false, importance: 0.6, confidence: 0.8, sourceInteractionId: null, knowledge: [{ name: 'Mira', state: 'knows', via: 'witnessed' }] }],
  relationships: [], threads: [], episodes: [],
  jobs: { pending: 0, running: 0, failed: 0, done: 3 },
  cast: [],
};
const CAST = { cast: [
  { id: 'c1', name: 'Mira', kind: 'npc', aliases: [], description: 'Bibliotecária', confirmed: false },
  { id: 'c2', name: 'Alex', kind: 'user', aliases: [], description: 'Perfil do usuário', confirmed: true },
] };
const chat = () => ({
  id: 'fixture', name: 'Cena fictícia', persona: 'Alex', presetId: 'mock', routeId: 'mock/route', revision: 1, characterRev: 1,
  memorySummary: {}, prompt: '', settings: {}, generation: null,
  history: [
    { id: 'u1', role: 'user', content: '*Entro na biblioteca.* O que você encontrou?', interactionId: 'i1' },
    { id: 'a1', role: 'assistant', content: 'Mira: Encontrei uma carta.\n*Ela abre o envelope com cuidado.*', status: 'complete', versionCount: 1 },
    { id: 'a2', role: 'assistant', content: 'Mira: Podemos ler juntas. A biblioteca guarda histórias de várias épocas.\n*Uma luz violeta ilumina a mesa.*', status: 'complete', versionCount: 1 },
  ],
});

const MIME = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.json': 'application/json', '.png': 'image/png', '.svg': 'image/svg+xml', '.webmanifest': 'application/manifest+json', '.ico': 'image/x-icon' };
const json = (res, obj) => { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(obj)); };
// Fictitious server: only the read routes used by the screens; any other /api/ answers 404.
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://fixture');
    const p = url.pathname;
    if (p === '/api/chats') return json(res, { chats: [{ ...chat(), history: undefined, snippet: 'Conversa fictícia para verificação' }] });
    if (p === '/api/chats/fixture') return json(res, chat());
    if (p === '/api/chats/fixture/cast') return json(res, CAST);
    if (p === '/api/chats/fixture/memory') return json(res, MEMORY);
    if (p === '/api/settings/global') { req.resume(); return json(res, { settings: { botChars: 8000, dialoguePct: 65, content: '' } }); }
    if (p === '/api/presets') return json(res, { presets: [{ id: 'mock', label: 'Fixture', routes: [] }] });
    if (p === '/api/status') return json(res, { router: { ok: true, models: 3 }, local: { chat: { ok: true }, embed: { ok: true } }, db: { jobs: { done: 3 } } });
    if (p === '/api/devices') return json(res, { devices: [{ id: 'd1', name: 'Aparelho fictício', last_seen_at: null }] });
    if (p === '/api/trash') return json(res, { days: 30, items: [] });
    if (p === '/api/events') { res.writeHead(200, { 'Content-Type': 'text/event-stream' }); return res.end('data: {"type":"hello"}\n\n'); }
    if (p.startsWith('/api/')) { res.writeHead(404, { 'Content-Type': 'application/json' }); return res.end('{"error":"fixture route absent"}'); }
    const file = path.resolve(dist, p === '/' ? 'index.html' : '.' + decodeURIComponent(p));
    if (!file.startsWith(dist + path.sep)) { res.writeHead(403); return res.end(); }
    const data = await fs.readFile(file);
    res.writeHead(200, { 'Content-Type': MIME[path.extname(file)] || 'application/octet-stream' });
    res.end(data);
  } catch {
    if (!res.headersSent) res.writeHead(404);
    res.end();
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const edge = spawn(process.env.MC_EDGE || 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', [
  '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
], { windowsHide: true, stdio: 'ignore' });

let ws, seq = 0;
const pending = new Map();
const errors = [];
const results = [];

// Runs inside the page. Classifies each text node (and field, placeholder and currentColor icon) by computed color.
function pageScan() {
  const SEMANTIC = new Set([
    '239,68,68', '248,113,113', '252,165,165', '254,202,202', // vermelho (erro)
    '250,204,21', '253,224,71', '254,240,138', // amarelo (aviso)
    '52,211,153', '110,231,183', '167,243,208', '16,185,129', '34,197,94', '74,222,128', // green (success, connection)
    '124,58,237', '167,139,250', '196,181,253', '139,92,246', '216,180,254', // roxo (destaque)
  ]);
  const rootVar = (v, d) => { const n = parseFloat(getComputedStyle(document.documentElement).getPropertyValue(v)); return Number.isFinite(n) ? n : d; };
  const TINT = [rootVar('--mc-tr', 1), rootVar('--mc-tg', 1), rootVar('--mc-tb', 1)];
  const isSemantic = (k) => { const c = k.split(',').map(Number); for (const s of SEMANTIC) { const t = s.split(',').map((v, i) => Number(v) * TINT[i]); if (t.every((v, i) => Math.abs(v - c[i]) <= 1.5)) return true; } return false; };
  const BODY = [27 * TINT[0], 27 * TINT[1], 29 * TINT[2]];
  const hex = (h) => { const m = /^#([0-9a-f]{6})$/i.exec(h); if (!m) return null; const n = parseInt(m[1], 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; };
  const rootCs = getComputedStyle(document.documentElement);
  const VARS = ['--mc-text', '--mc-name', '--mc-action', '--mc-tone', '--mc-white', '--mc-muted'];
  const onAccentRgb = hex(getComputedStyle(document.documentElement).getPropertyValue('--mc-onaccent').trim());
  const onAccentKey = onAccentRgb ? onAccentRgb.join(',') : null;
  const palette = VARS.map((v) => ({ v, hex: rootCs.getPropertyValue(v).trim() }));
  const palMap = new Map();
  for (const p of palette) { const rgb = hex(p.hex); if (rgb) palMap.set(rgb.join(','), p.v); }
  const parse = (s) => { const m = (s || '').match(/[\d.]+/g); if (!m || m.length < 3) return null; return { rgb: m.slice(0, 3).map(Number), a: m[3] === undefined ? 1 : Number(m[3]) }; };
  const lum = (c) => { const f = (v) => { const x = v / 255; return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const surfaceOf = (el) => { for (let a = el, d = 0; a && d < 6; a = a.parentElement, d++) { const c = parse(getComputedStyle(a).backgroundColor); if (c && c.a > 0.05) return c; } return null; };
  const colored = (c) => !!c && c.a >= 0.5 && Math.max(...c.rgb) - Math.min(...c.rgb) >= 40;
  const COLOR_CLS = /^(placeholder:)?text-|^mc-(action|name|tone)$/;
  const SIZE_CLS = /^(placeholder:)?text-((xs|sm|base|lg|xl|[2-9]xl|left|center|right|justify|ellipsis|clip|wrap|nowrap|balance|pretty|start|end)$|\[\d)/;
  const colorClasses = (el) => [...el.classList].filter((c) => COLOR_CLS.test(c) && !SIZE_CLS.test(c));
  // index.css rules that swap a class color for a --mc-* variable (the mapping the palette uses).
  const mapped = new Set();
  const walk = (rules) => { for (const r of rules) { if (r.cssRules && !r.selectorText) walk(r.cssRules); else if (r.selectorText && r.style && /--mc-/.test(r.style.color || '')) mapped.add(r.selectorText); } };
  for (const sh of document.styleSheets) { try { walk(sh.cssRules); } catch { /* stylesheet from an external origin */ } }
  const isMapped = (cls) => { const sel = `.${CSS.escape(cls)}`; for (const s of mapped) if (s.includes(sel)) return true; return false; };
  const mappedNote = (classes) => (classes.some(isMapped) ? 'mapeada para --mc-* (tailwind.config/index.css)' : 'sem mapeamento para --mc-*');
  const INLINE = /(^|;)\s*color\s*:/;
  const originOf = (el, isPh) => {
    if (isPh) { const pc = colorClasses(el).filter((c) => c.startsWith('placeholder:')); return pc.length ? `classe ${pc.join(' ')} (${mappedNote(pc)}; pseudo ::placeholder)` : 'sem classe de placeholder (preflight do Tailwind: ::placeholder fixo)'; }
    if (INLINE.test(el.getAttribute('style') || '')) return 'estilo inline';
    const own = colorClasses(el);
    if (own.length) return `classe ${own.join(' ')} (${mappedNote(own)})`;
    for (let a = el.parentElement; a; a = a.parentElement) {
      const tag = a.tagName.toLowerCase();
      if (INLINE.test(a.getAttribute('style') || '')) return `herdada de <${tag}> estilo inline`;
      const c = colorClasses(a);
      if (c.length) return `herdada de <${tag}> classe ${c.join(' ')} (${mappedNote(c)})`;
      if (a === document.body) break;
    }
    return 'herdada sem classe de cor (html/body: var(--mc-text))';
  };
  const rows = [];
  const push = (el, text, colorStr, isPh) => {
    const c = parse(colorStr);
    if (!c) return;
    const k = c.rgb.join(',');
    const surf = surfaceOf(el);
    const bg = surf ? surf.rgb.map((v, i) => surf.a * v + (1 - surf.a) * BODY[i]) : BODY;
    const eff = c.rgb.map((v, i) => c.a * v + (1 - c.a) * bg[i]);
    const cr = ratio(eff, bg);
    let cat, why = '', risk = '';
    if (c.a >= 0.999 && k === onAccentKey && colored(surf)) cat = 'SUPERFICIE'; // text on a filled button (--mc-onaccent)
    else if (palMap.has(k) && c.a >= 0.999) {
      cat = 'PALETA';
      if (colored(surf) && palMap.get(k) === '--mc-white') risk = 'texto da paleta (--mc-white) sobre superfície colorida';
    } else if (c.a >= 0.999 && isSemantic(k)) cat = 'SEMANTICA';
    else if (c.a >= 0.999 && colored(surf)) cat = 'SUPERFICIE';
    else { cat = 'FORA'; why = palMap.has(k) ? 'opacidade <1 sobre cor da paleta' : 'cor fixa fora da paleta'; }
    if (cat === 'PALETA' && cr < 4.5) risk = risk || 'contraste <4.5';
    rows.push({
      cat, why, risk,
      origin: cat === 'FORA' || risk ? originOf(el, isPh) : '',
      hex: `#${c.rgb.map((v) => v.toString(16).padStart(2, '0')).join('')}`, a: +c.a.toFixed(2),
      tag: el.tagName.toLowerCase(), text: text.replace(/\s+/g, ' ').trim().slice(0, 40), cr: +cr.toFixed(2),
    });
  };
  for (const el of document.body.querySelectorAll('*')) {
    const tag = el.tagName.toLowerCase();
    if (el instanceof SVGElement && !(el instanceof SVGSVGElement)) continue;
    if (['script', 'style', 'option', 'noscript', 'template'].includes(tag)) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    const rc = el.getBoundingClientRect();
    if (rc.width < 1 || rc.height < 1) continue;
    if (el instanceof SVGSVGElement) {
      if (el.getAttribute('stroke') === 'currentColor' || el.getAttribute('fill') === 'currentColor') push(el, '(ícone)', cs.color, false);
      continue;
    }
    if (tag === 'input' || tag === 'textarea' || tag === 'select') {
      if (tag === 'input' && !['text', 'search', 'email', 'url', 'tel', 'password', 'number'].includes(el.type)) continue;
      if (tag === 'select') { push(el, el.options[el.selectedIndex]?.text || '(seleção)', cs.color, false); continue; }
      push(el, el.value || '(campo)', cs.color, false);
      if (tag !== 'select') push(el, '(cursor)', cs.caretColor, false);
      if (el.placeholder && !el.value) push(el, el.placeholder, getComputedStyle(el, '::placeholder').color, true);
      continue;
    }
    let own = '';
    for (const n of el.childNodes) if (n.nodeType === 3 && n.textContent.trim()) own += `${n.textContent.trim()} `;
    if (own) push(el, own, cs.color, false);
  }
  const counts = { total: rows.length, PALETA: 0, SEMANTICA: 0, SUPERFICIE: 0, FORA: 0, RISCO: 0, CONTRASTE: 0 };
  const groups = new Map();
  for (const r of rows) {
    counts[r.cat]++;
    if (r.risk.startsWith('contraste')) counts.CONTRASTE++;
    else if (r.risk) counts.RISCO++;
    if (r.cat !== 'FORA' && !r.risk) continue;
    const key = [r.cat, r.risk, r.why, r.origin, r.hex, r.a, r.tag].join('|');
    const g = groups.get(key) || { n: 0, cat: r.cat, risk: r.risk, why: r.why, origin: r.origin, hex: r.hex, a: r.a, tag: r.tag, cr: r.cr, sample: [] };
    g.n++;
    g.cr = Math.min(g.cr, r.cr);
    if (g.sample.length < 2 && !g.sample.includes(r.text)) g.sample.push(r.text);
    groups.set(key, g);
  }
  return { palette: palette.map((p) => `${p.v}=${p.hex}`), counts, groups: [...groups.values()].sort((a, b) => b.n - a.n) };
}

// Closes the top layer: sheet > screen (Back) > drawer (scrim) > chat (Back). Highest z-index wins; on a tie, the last in the DOM.
const CLOSE_JS = `(()=>{
  const z = (e) => { let m = 0; for (let a = e; a; a = a.parentElement) { const s = getComputedStyle(a); if (s.position === 'fixed') { const n = parseInt(s.zIndex, 10); if (!isNaN(n)) m = Math.max(m, n); } } return m; };
  const vis = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  let best = null, bz = -1;
  for (const e of document.querySelectorAll('.mc-sheet-scrim, [aria-label="Voltar"], .mc-drawer .mc-scrim')) {
    if (!vis(e)) continue;
    const zz = z(e);
    if (zz >= bz) { bz = zz; best = e; }
  }
  if (!best) return null;
  const kind = best.classList.contains('mc-sheet-scrim') ? 'folha' : best.classList.contains('mc-scrim') ? 'gaveta' : 'voltar';
  best.click();
  return kind;
})()`;
const LIST_JS = `(()=>{
  const h = Array.from(document.querySelectorAll('h1')).find((x) => x.textContent.trim() === 'Conversas');
  return !!h && h.getBoundingClientRect().width > 0 && !document.querySelector('.mc-chat, .mc-drawer, .mc-sheet-overlay, .fixed.inset-0.z-50');
})()`;

try {
  let port;
  for (let n = 0; n < 120; n++) { try { port = (await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; break; } catch { await pause(100); } }
  if (!port) throw new Error('Edge não iniciou o depurador em 12 s');
  const target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === 'page');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve, { once: true }); ws.addEventListener('error', reject, { once: true }); });
  ws.addEventListener('message', ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.id) { const p = pending.get(msg.id); if (!p) return; pending.delete(msg.id); clearTimeout(p.timer); msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result); }
    if (msg.method === 'Runtime.exceptionThrown') errors.push(`${msg.params.exceptionDetails.text}: ${msg.params.exceptionDetails.exception?.description || ''}`.slice(0, 300));
  });
  const cdp = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++seq;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const r = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  const waitFor = async (expression, what) => {
    for (let n = 0; n < 80; n++) { if (await evaluate(`!!(${expression})`)) return; await pause(75); }
    throw new Error(`Tempo esgotado aguardando: ${what}`);
  };
  const screenshot = async (name) => {
    const { data } = await cdp('Page.captureScreenshot', { format: 'png' });
    await fs.writeFile(path.join(out, `${name.replace(/[^\w.@-]+/g, '_')}.png`), Buffer.from(data, 'base64'));
  };
  const clickAria = async (label) => {
    const ok = await evaluate(`(()=>{const el=Array.from(document.querySelectorAll(${JSON.stringify(`[aria-label="${label}"]`)})).filter((e)=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0;}).at(-1);if(!el)return false;el.click();return true;})()`);
    if (!ok) throw new Error(`Controle não encontrado: aria-label "${label}"`);
  };
  const clickText = async (text, { exact = false, startsWith = false, scope = 'body' } = {}) => {
    const ok = await evaluate(`(()=>{const t=${JSON.stringify(text)};const sc=document.querySelector(${JSON.stringify(scope)});if(!sc)return false;const el=Array.from(sc.querySelectorAll('button')).filter((b)=>{const r=b.getBoundingClientRect();return r.width>0&&r.height>0;}).find((b)=>{const s=b.textContent.trim();return ${exact} ? s===t : ${startsWith} ? s.startsWith(t) : s.includes(t);});if(!el)return false;el.click();return true;})()`);
    if (!ok) throw new Error(`Botão não encontrado: "${text}" em ${scope}`);
  };
  // Waits for the closed layer to leave the DOM (the sheet and drawer animate when closing) instead of a fixed delay.
  const closeTop = async () => {
    const kind = await evaluate(CLOSE_JS);
    if (!kind) return null;
    const sel = { folha: '.mc-sheet-overlay', gaveta: '.mc-drawer' }[kind];
    for (let n = 0; sel && n < 30; n++) { if (!(await evaluate(`!!document.querySelector(${JSON.stringify(sel)})`))) break; await pause(60); }
    await pause(120);
    return kind;
  };
  const goList = async () => {
    for (let i = 0; i < 8; i++) {
      if (await evaluate(LIST_JS)) return;
      if (!(await closeTop())) break;
    }
    await waitFor(LIST_JS, 'lista de conversas');
  };
  const setRange = (label, value) => evaluate(`(()=>{const t=document.querySelector(${JSON.stringify(`input[aria-label="${label}"]`)});Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(t,${JSON.stringify(String(value))});t.dispatchEvent(new Event('input',{bubbles:true}));return t.value;})()`);
  const setCustom = (hex) => evaluate(`(()=>{const t=document.querySelector('input[aria-label="Escolher outra cor"]');if(!t)return false;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(t,${JSON.stringify(hex)});t.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
  const openMenu = async (id) => {
    await evaluate(`(()=>{const p=document.querySelector('[data-message-id="${id}"] p');p.dispatchEvent(new TouchEvent('touchstart',{bubbles:true,touches:[]}));p.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true}));p.dispatchEvent(new TouchEvent('touchend',{bubbles:true,touches:[]}));})()`);
    for (let n = 0; n < 15; n++) { if (await evaluate(`!!document.querySelector('.mc-sheet')`)) return; await pause(100); }
    const info = await evaluate(`({p:!!document.querySelector('[data-message-id="${id}"] p'),mensagens:document.querySelectorAll('[data-message-id]').length,overlay:!!document.querySelector('.mc-sheet-overlay'),chat:!!document.querySelector('.mc-chat'),editor:!!document.querySelector('.mc-editor'),texto:(document.querySelector('.mc-scroll')?.textContent||'').slice(0,160)})`);
    throw new Error(`Folha de menu não abriu para ${id}: ${JSON.stringify(info)}`);
  };

  // Each sweep records the palette state (--mc-* variables) and the nodes outside it.
  const scan = async (combo, screen) => {
    const r = await evaluate(`(${pageScan.toString()})()`);
    results.push({ combo, screen, ...r });
    if (SHOTS) await screenshot(`${combo}-${screen}`);
    const c = r.counts;
    console.log(`[${combo}] ${screen}: nós=${c.total} paleta=${c.PALETA} semântica=${c.SEMANTICA} superfície=${c.SUPERFICIE} FORA=${c.FORA} risco=${c.RISCO} contraste<4.5=${c.CONTRASTE}`);
    for (const g of r.groups.filter((x) => x.cat === 'FORA' || x.risk).slice(0, 8)) {
      console.log(`    ${g.n}× ${g.cat}${g.risk ? ` [${g.risk}]` : ''}${g.why ? ` (${g.why})` : ''} | ${g.hex}${g.a < 1 ? ` a=${g.a}` : ''} | <${g.tag}> | ${g.origin} | cr=${g.cr} | ex.: ${JSON.stringify(g.sample[0] || '')}`);
    }
  };

  await cdp('Page.enable'); await cdp('Runtime.enable');
  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
  await cdp('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  await cdp('Page.navigate', { url: base });
  await waitFor(`Array.from(document.querySelectorAll('button')).some((b) => b.textContent.includes('Cena fictícia'))`, 'lista de conversas (fixture)');

  for (const id of PRESET_IDS) {
    for (const b of BRIGHTNESS) {
      const tag = `${id}@${b}`;
      await goList();
      await clickAria('Configurações');
      await waitFor(`Array.from(document.querySelectorAll('h2')).some((h) => h.textContent.trim() === 'Configurações')`, 'tela Configurações');
      if (id === 'custom') { if (!(await setCustom(CUSTOM))) throw new Error('Campo "Escolher outra cor" ausente'); }
      else await clickText(PRESET_LABEL[id], { exact: true, scope: '.fixed.inset-0.z-50' });
      await setRange('Brilho das letras', b);
      await pause(150);
      await scan(tag, 'Configurações');
      await goList();
      await scan(tag, 'Lista de conversas');
      await clickText('Cena fictícia');
      await waitFor(`!!document.querySelector('[aria-label="Continuar a cena"]')`, 'chat aberto');
      await pause(150);
      await scan(tag, 'Chat (balões)');
      await openMenu('a1');
      await waitFor(`!!document.querySelector('.mc-sheet')`, 'folha de menu da mensagem');
      await pause(200);
      await scan(tag, 'Folha de menu da mensagem');
      await clickText('Editar', { exact: true });
      await waitFor(`!!document.querySelector('.mc-editor')`, 'editor de mensagem');
      await pause(200);
      await scan(tag, 'Edição da mensagem');
      await clickText('Cancelar', { exact: true, scope: '.mc-editor' });
      await waitFor(`!document.querySelector('.mc-editor')`, 'fechar editor');
      await pause(200);
      await clickAria('Opções');
      await waitFor(`!!document.querySelector('.mc-drawer')`, 'gaveta lateral');
      await pause(280);
      await scan(tag, 'Gaveta');
      for (const row of DRAWER_ROWS) {
        if (!(await evaluate(`!!document.querySelector('.mc-drawer')`))) { await clickAria('Opções'); await waitFor(`!!document.querySelector('.mc-drawer')`, 'gaveta lateral'); await pause(280); }
        await clickText(row, { startsWith: true, scope: '.mc-drawer' });
        await pause(320);
        await scan(tag, `Gaveta › ${row}`);
        await closeTop();
      }
      if (await evaluate(`!!document.querySelector('.mc-drawer')`)) await closeTop();
      await pause(260);
      await clickAria('Memória');
      await waitFor(`Array.from(document.querySelectorAll('h2')).some((h) => h.textContent.trim() === 'Memória do PC')`, 'Memória do PC');
      await pause(250);
      await scan(tag, 'Memória › Cena e dias');
      for (const t of ['Memórias', 'Relações e threads', 'Processamento']) {
        await clickText(t, { exact: true });
        await pause(250);
        await scan(tag, `Memória › ${t}`);
      }
      if (await evaluate(`!!document.querySelector('[aria-label="Adicionar memória"]')`)) {
        await clickAria('Adicionar memória');
        await pause(300);
        await scan(tag, 'Memória › Adicionar memória (folha)');
        await closeTop();
      }
      await closeTop();
      await pause(260);
      await goList();
      console.log(`  paleta ativa ${tag}: ${(results.at(-1).palette || []).join(' ')}`);
    }
  }

  // Blue-light filter and paper: the palette is tinted (no overlay), the paper is a static opacity-only layer.
  const BLUE = 60;
  await goList();
  await clickAria('Configurações');
  await waitFor(`Array.from(document.querySelectorAll('h2')).some((h) => h.textContent.trim() === 'Configurações')`, 'tela Configurações');
  await clickText('Neutro', { exact: true, scope: '.fixed.inset-0.z-50' });
  await setRange('Brilho das letras', 100);
  await setRange('Filtro de luz azul', BLUE);
  await setRange('Textura de papel', 100);
  await pause(200);
  const fx = await evaluate(`(()=>{
    const cs = (e, p) => getComputedStyle(e, p);
    const blend = Array.from(document.querySelectorAll('body *')).filter((e) => cs(e).mixBlendMode !== 'normal').map((e) => e.id || e.className.toString().slice(0, 40));
    const paper = document.getElementById('mc-paper');
    const root = cs(document.documentElement);
    return {
      pelicula: !!document.getElementById('mc-bluelight'),
      blendEmTela: blend,
      corpo: cs(document.body).backgroundColor,
      fatores: ['--mc-tr', '--mc-tg', '--mc-tb'].map((v) => root.getPropertyValue(v).trim()),
      texto: root.getPropertyValue('--mc-text').trim(),
      paper: paper ? { display: cs(paper).display, opacity: cs(paper).opacity, blend: cs(paper).mixBlendMode, willChange: cs(paper).willChange, evento: cs(paper).pointerEvents, pos: cs(paper).position } : null,
    };
  })()`);
  console.log('\n=== FILTRO DE LUZ AZUL + PAPEL (azul=' + BLUE + '%, papel=100%) ===');
  console.log(JSON.stringify(fx));
  const cb = (fx.corpo.match(/[\d.]+/g) || []).map(Number);
  const fxErrors = [];
  if (fx.pelicula) fxErrors.push('película #mc-bluelight ainda existe');
  if (fx.blendEmTela.length) fxErrors.push('mix-blend-mode em: ' + fx.blendEmTela.join(', '));
  if (!(cb[2] < 20 && cb[0] > 25)) fxErrors.push('fundo do corpo não foi tingido: ' + fx.corpo);
  if (!fx.paper || fx.paper.display !== 'block' || Math.abs(Number(fx.paper.opacity) - 0.24) > 0.001) fxErrors.push('papel fora do esperado: ' + JSON.stringify(fx.paper));
  if (fx.paper && (fx.paper.blend !== 'normal' || fx.paper.willChange !== 'auto' || fx.paper.evento !== 'none')) fxErrors.push('papel com blend/will-change/eventos: ' + JSON.stringify(fx.paper));
  await scan(`neutro@100+azul${BLUE}`, 'Configurações');
  if (SHOTS) await screenshot(`neutro@100+azul${BLUE}-papel`);
  await goList();
  await scan(`neutro@100+azul${BLUE}`, 'Lista de conversas');
  await clickText('Cena fictícia');
  await waitFor(`!!document.querySelector('[aria-label="Continuar a cena"]')`, 'chat aberto');
  await pause(150);
  await scan(`neutro@100+azul${BLUE}`, 'Chat (balões)');
  if (SHOTS) await screenshot(`neutro@100+azul${BLUE}-chat`);
  await goList();
  await clickAria('Configurações');
  await waitFor(`Array.from(document.querySelectorAll('h2')).some((h) => h.textContent.trim() === 'Configurações')`, 'tela Configurações');
  await setRange('Filtro de luz azul', 0);
  await setRange('Textura de papel', 0);
  for (const e of fxErrors) console.log('  FALHA: ' + e);
  if (fxErrors.length) errors.push(...fxErrors.map((e) => 'filtro/papel: ' + e));

  // Root causes: groups the nodes outside the palette (and at risk) by origin and color, with the affected screens and combinations.
  const agg = new Map();
  for (const e of results) for (const g of e.groups) {
    if (g.cat !== 'FORA' && !g.risk) continue;
    const key = `${g.cat}${g.risk ? ` [${g.risk}]` : ''}${g.why ? ` (${g.why})` : ''} | ${g.origin} | ${g.hex}${g.a < 1 ? ` a=${g.a}` : ''}`;
    const x = agg.get(key) || { key, n: 0, screens: new Set(), combos: new Set(), sample: g.sample[0] || '' };
    x.n += g.n; x.screens.add(e.screen); x.combos.add(e.combo);
    agg.set(key, x);
  }
  const causes = [...agg.values()].sort((a, b) => b.n - a.n).map((x) => ({ n: x.n, telas: x.screens.size, combinacoes: x.combos.size, origem: x.key, exemplo: x.sample }));
  const totals = results.reduce((t, e) => ({ nos: t.nos + e.counts.total, paleta: t.paleta + e.counts.PALETA, semantica: t.semantica + e.counts.SEMANTICA, superficie: t.superficie + e.counts.SUPERFICIE, fora: t.fora + e.counts.FORA, risco: t.risco + e.counts.RISCO, contraste: t.contraste + e.counts.CONTRASTE }), { nos: 0, paleta: 0, semantica: 0, superficie: 0, fora: 0, risco: 0, contraste: 0 });
  console.log('\n=== CAUSAS RAIZ (nós FORA da paleta ou com risco, somados em todas as telas e combinações) ===');
  for (const c of causes.slice(0, 40)) console.log(`${String(c.n).padStart(6)}× telas=${c.telas} combinações=${c.combinacoes} | ${c.origem} | ex.: ${JSON.stringify(c.exemplo)}`);
  console.log(`\nTotais: nós=${totals.nos} paleta=${totals.paleta} semântica=${totals.semantica} superfície=${totals.superficie} FORA=${totals.fora} risco=${totals.risco} contraste<4.5=${totals.contraste}`);
  await fs.writeFile(path.join(out, 'text-colors.json'), JSON.stringify({ generatedAt: new Date().toISOString(), dist, presets: PRESET_IDS, brightness: BRIGHTNESS, totals, causes, errors, results }, null, 2));
  console.log(`JSON completo: ${path.join(out, 'text-colors.json')}`);
  if (errors.length) console.log(`Erros de página: ${errors.length} (primeiro: ${errors[0]})`);
  process.exitCode = totals.fora > 0 || errors.length ? 1 : 0;
} finally {
  if (errors.length) console.error(`Erros de página (${errors.length}; primeiro: ${errors[0]})`);
  ws?.close();
  for (const p of pending.values()) clearTimeout(p.timer);
  edge.kill();
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
  await fs.rm(profile, { recursive: true, force: true }).catch(() => {});
}
