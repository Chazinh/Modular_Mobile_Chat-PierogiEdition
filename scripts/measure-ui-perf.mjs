// Chat UI performance measurement: headless Edge, emulated phone (390x844), throttled CPU,
// fictitious HTTP server and a disposable profile. Uses neither the real server nor personal data.
// What the numbers mean: main-thread CPU time (script, layout, style) and, in the trace,
// software paint and raster time. Headless without a GPU measures neither compositing nor the device's 90–120 Hz.
// Usage: MC_UI_DIST=<build folder> [MC_PERF_OUT=<output folder>] [MC_PERF_CSS_FILE=<hypothesis css>] node scripts/measure-ui-perf.mjs
// Keep MC_PERF_OUT short: Edge's IndexedDB fails ("Internal error opening backing store") with long paths on Windows.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';

const env = process.env;
assert.ok(env.MC_UI_DIST, 'defina MC_UI_DIST com a pasta de saída do build do cliente');
const dist = path.resolve(env.MC_UI_DIST);
const out = path.resolve(env.MC_PERF_OUT || path.join(os.tmpdir(), `mc-perf-${Date.now()}`));
const MESSAGES = 2 * Math.ceil(Math.max(300, Number(env.MC_PERF_MESSAGES || 320)) / 2); // even: the last turn belongs to the scene
const CPU_RATE = Number(env.MC_PERF_CPU_RATE || 4);
const SCROLL_STEPS = Number(env.MC_PERF_SCROLL_STEPS || 60);
const PRIVACY = env.MC_PERF_PRIVACY !== '0'; // on the device the cache is always encrypted; here it is AES-GCM through WebCrypto
const EXTRA_CSS = env.MC_PERF_CSS_FILE ? await fs.readFile(path.resolve(env.MC_PERF_CSS_FILE), 'utf8') : '';
const EDGE = env.MC_EDGE || 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe';
const TITLE = 'Cena de desempenho';
const TYPED = 'Quero saber mais sobre a carta antiga, e onde ela estava?'.slice(0, 40);
const TRACE_CATS = 'devtools.timeline,disabled-by-default-devtools.timeline,disabled-by-default-devtools.timeline.frame,blink,cc,gpu,viz,toplevel';
const TRACE_NAMES = new Set(['RasterTask', 'Rasterize', 'ImageDecodeTask', 'Paint', 'PaintImage', 'CompositeLayers', 'UpdateLayerTree', 'DrawFrame', 'Layout', 'UpdateLayoutTree', 'RecalculateStyles', 'HitTest', 'PrePaint', 'Commit', 'EventDispatch']);
const METRICS = ['TaskDuration', 'ScriptDuration', 'LayoutDuration', 'RecalcStyleDuration', 'LayoutCount', 'RecalcStyleCount', 'Nodes', 'JSHeapUsedSize'];

const pause = (ms) => new Promise((r) => setTimeout(r, ms));
const r2 = (x) => Math.round(x * 100) / 100;
const toMs = (s) => r2(s * 1000);
const stat = (values) => {
  const s = [...values].sort((a, b) => a - b);
  if (!s.length) return null;
  const at = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))];
  return { median: r2(at(0.5)), p95: r2(at(0.95)), max: r2(s.at(-1)), mean: r2(s.reduce((a, b) => a + b, 0) / s.length) };
};
// Least squares: cost = fixed + slope × messages.
const fit = (points) => {
  const n = points.length;
  const sx = points.reduce((s, p) => s + p.x, 0), sy = points.reduce((s, p) => s + p.y, 0);
  const sxx = points.reduce((s, p) => s + p.x * p.x, 0), sxy = points.reduce((s, p) => s + p.x * p.y, 0);
  const slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
  return { fixedMsPerKey: r2((sy - slope * sx) / n), msPerMessagePerKey: Math.round(slope * 10000) / 10000 };
};

// Fictitious, reproducible history (fixed PRNG): alternating turns, replies with scene markup.
let seed = 20260612;
const rnd = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 4294967296;
const pick = (list) => list[Math.floor(rnd() * list.length)];
const ACTIONS = ['Ela ajusta o xale sobre os ombros.', 'Uma luz violeta ilumina a mesa.', 'O relógio da torre marca meia-noite.', 'Ela abre o envelope com cuidado.', 'A chuva bate nas janelas altas.'];
const LINES = ['Podemos ler juntas.', 'Não toque nessa carta ainda.', 'A biblioteca guarda histórias de várias épocas.', 'Eu achei uma passagem escondida.', 'Você confia em mim?'];
const WORDS = ['mistério', 'silêncio', 'segredo', 'ferrugem', 'lembrança'];
function replyFor() {
  const paragraphs = [];
  const n = 3 + Math.floor(rnd() * 4);
  for (let k = 0; k < n; k++) {
    const x = rnd();
    if (x < 0.35) paragraphs.push(`*${pick(ACTIONS)}*`);
    else if (x < 0.75) paragraphs.push(`Mira: ${pick(LINES)} **${pick(WORDS)}** ${pick(LINES)}${rnd() < 0.25 ? ' (baixinho)' : ''}`);
    else paragraphs.push(`*${pick(ACTIONS)}* ${pick(LINES)}`);
  }
  return paragraphs.join('\n');
}
const history = [];
for (let i = 0; i < MESSAGES; i++) {
  if (i % 2 === 0) history.push({ id: `u${i}`, role: 'user', content: `*Entro na cena ${i}.* ${pick(LINES)}`, interactionId: `i${i}` });
  else history.push({ id: `a${i}`, role: 'assistant', content: replyFor(), status: 'complete', versionCount: 1 });
}
const chat = { id: 'perf', name: TITLE, persona: 'Alex', presetId: 'mock', routeId: 'mock/route', revision: 1, characterRev: 1, memorySummary: {}, history };
const LAST_ID = history.at(-1).id;
const cacheKey = randomBytes(32).toString('base64');
const hits = { chat: 0 };
const unknownApi = new Set();

const MIME = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.webp': 'image/webp', '.woff2': 'font/woff2' };
const sendJson = (res, obj, status = 200) => { res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(obj)); };
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://fixture');
    const p = url.pathname;
    if (p.startsWith('/api/')) {
      if (p === '/api/privacy/cache-key' && PRIVACY) return sendJson(res, { key: cacheKey });
      if (p === '/api/chats') return sendJson(res, { chats: [{ ...chat, history: undefined, snippet: 'Conversa fictícia de desempenho' }] });
      if (p === '/api/chats/perf') { hits.chat++; return sendJson(res, { ...chat, generation: null }); }
      if (p === '/api/chats/perf/cast') return sendJson(res, { cast: [] });
      if (p === '/api/chats/perf/message' && req.method === 'POST') {
        // Confirms the send and keeps the stream open: the UI stays on "Thinking…" for the idle-frame probe.
        res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
        res.write(`data: ${JSON.stringify({ ack: { generationId: 'g-idle', mode: 'continue', revision: chat.revision } })}\n\n`);
        return;
      }
      if (p === '/api/presets') return sendJson(res, { presets: [{ id: 'mock', label: 'Fixture', routes: [] }] });
      if (p === '/api/settings/global') return sendJson(res, { settings: { botChars: 8000, dialoguePct: 65, content: '' } });
      if (p === '/api/events') { res.writeHead(200, { 'Content-Type': 'text/event-stream' }); return res.end('data: {"type":"hello"}\n\n'); }
      unknownApi.add(p);
      return sendJson(res, { error: 'fixture route absent' }, 404);
    }
    const file = path.resolve(dist, p === '/' ? 'index.html' : '.' + decodeURIComponent(p));
    if (!file.startsWith(dist + path.sep)) { res.writeHead(403); return res.end(); }
    const data = await fs.readFile(file);
    res.writeHead(200, { 'Content-Type': MIME[path.extname(file)] || 'application/octet-stream' });
    res.end(data);
  } catch {
    if (!res.headersSent) res.writeHead(500);
    res.end();
  }
});

// Runs in the page before the app: DevTools hook (production React calls it on every commit).
// It only walks the tree between start()/stop(); outside that the cost is near zero. Does not change client code.
function installPerfHook() {
  const state = { on: false, root: null, seen: new Map(), commits: 0, renders: { user: 0, assistant: 0 } };
  const visit = (fiber, count) => {
    const stack = [fiber];
    while (stack.length) {
      const f = stack.pop();
      const p = f.memoizedProps;
      // New props = the parent re-rendered and the message rendered again (UserBubble/AssistantCard receive `m`).
      if (p && p.m && p.m.id != null && f.key != null && state.seen.get(f.key) !== p) {
        state.seen.set(f.key, p);
        if (count) state.renders[p.m.role] = (state.renders[p.m.role] || 0) + 1;
      }
      if (f.sibling) stack.push(f.sibling);
      if (f.child) stack.push(f.child);
    }
  };
  window.__mcPerf = {
    start() {
      state.commits = 0;
      state.renders = { user: 0, assistant: 0 };
      if (state.root) visit(state.root.current, false);
      state.on = true;
    },
    stop() {
      state.on = false;
      return { commits: state.commits, renders: { ...state.renders } };
    },
  };
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    isDisabled: false,
    renderers: new Map(),
    inject: () => 1,
    checkDCE() {},
    onCommitFiberRoot(_id, root) {
      state.root = root;
      if (!state.on) return;
      state.commits++;
      visit(root.current, true);
    },
    onCommitFiberUnmount() {},
    onPostCommitFiberRoot() {},
  };
}

// Runs in the page: resolves when the last message and the "Continue the scene" button exist and have been painted.
function watchOpen(lastId) {
  return new Promise((resolve) => {
    const t0 = performance.now();
    const marks = {};
    const check = () => {
      const now = performance.now() - t0;
      if (marks.first == null && document.querySelector('[data-message-id]')) marks.first = now;
      if (marks.last == null && document.querySelector(`[data-message-id="${lastId}"]`)) marks.last = now;
      if (marks.action == null && document.querySelector('[aria-label="Continuar a cena"]')) marks.action = now;
      if (marks.last != null && marks.action != null) {
        mo.disconnect();
        requestAnimationFrame(() => requestAnimationFrame(() => resolve({ ...marks, paintedMs: performance.now() - t0 })));
      }
    };
    const mo = new MutationObserver(check);
    mo.observe(document.body, { childList: true, subtree: true });
    check();
    setTimeout(() => { mo.disconnect(); resolve({ timeout: true, ...marks }); }, 20000);
  });
}

// Runs in the page: programmatic scroll from top to bottom, one step per frame. Measures rAF intervals.
async function scrollThrough(steps) {
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const el = document.querySelector('.mc-scroll');
  const stamps = [];
  let running = true;
  const tick = (t) => { stamps.push(t); if (running) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  let scrollEvents = 0;
  const onScroll = () => { scrollEvents++; };
  el.addEventListener('scroll', onScroll, { passive: true });
  el.scrollTop = 0;
  await frame();
  const span = el.scrollHeight - el.clientHeight;
  const step = span / steps;
  const t0 = performance.now();
  for (let i = 1; i <= steps; i++) { el.scrollTop = Math.round(step * i); await frame(); }
  const wallMs = performance.now() - t0;
  running = false;
  el.removeEventListener('scroll', onScroll);
  const gaps = stamps.slice(1).map((t, i) => t - stamps[i]);
  return { steps, span: Math.round(span), scrollEvents, wallMs: Math.round(wallMs), gaps };
}

// Replica of the OLD rememberReading() implementation (linear scan over [data-message-id]), only to compare its cost;
// the current ChatView uses a binary search over the list's children, so this number is the "before" reference, not the current app.
// "clean": layout already computed (scrolling). "dirty": layout invalidated first, as when typing.
function benchRememberReading(samples) {
  const scroller = document.querySelector('.mc-scroll');
  const ta = document.querySelector('textarea');
  const total = scroller.scrollHeight - scroller.clientHeight;
  const remember = () => {
    const top = scroller.getBoundingClientRect().top;
    const anchor = [...scroller.querySelectorAll('[data-message-id]')].find((e) => e.getBoundingClientRect().bottom > top);
    return anchor ? anchor.dataset.messageId : null;
  };
  const clean = [];
  const dirty = [];
  for (let i = 0; i <= samples; i++) {
    scroller.scrollTop = Math.round((total * i) / samples);
    scroller.getBoundingClientRect();
    const t0 = performance.now();
    remember();
    clean.push(performance.now() - t0);
    ta.style.height = `${40 + (i % 2)}px`;
    const t1 = performance.now();
    remember();
    dirty.push(performance.now() - t1);
  }
  ta.style.height = '';
  return { clean, dirty };
}

// Sum by name and thread of trace events (nested events with the same name may count more than once).
function summarizeTrace(events) {
  const names = new Map();
  const open = new Map();
  const totals = new Map();
  const add = (name, key, dur) => {
    const k = `${names.get(key) || 'outra'} | ${name}`;
    const t = totals.get(k) || { ms: 0, n: 0 };
    t.ms += dur / 1000;
    t.n++;
    totals.set(k, t);
  };
  for (const e of events) if (e.ph === 'M' && e.name === 'thread_name') names.set(`${e.pid}:${e.tid}`, e.args?.name);
  for (const e of events) {
    const key = `${e.pid}:${e.tid}`;
    if (e.ph === 'X' && e.dur && TRACE_NAMES.has(e.name)) add(e.name, key, e.dur);
    else if (e.ph === 'B') { const st = open.get(key) || []; st.push(e); open.set(key, st); }
    else if (e.ph === 'E') { const b = open.get(key)?.pop(); if (b && TRACE_NAMES.has(b.name)) add(b.name, key, e.ts - b.ts); }
  }
  const top = [...totals].sort((a, b) => b[1].ms - a[1].ms).slice(0, 12).map(([k, t]) => [k, { ms: r2(t.ms), n: t.n }]);
  const rasterMs = r2([...totals].filter(([k]) => k.endsWith('| RasterTask')).reduce((s, [, t]) => s + t.ms, 0));
  return { rasterMs, top: Object.fromEntries(top) };
}

await fs.mkdir(out, { recursive: true });
const profile = await fs.mkdtemp(path.join(out, 'edge-'));
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const edge = spawn(EDGE, [
  '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
], { windowsHide: true, stdio: 'ignore' });

let ws;
let seq = 0;
const pending = new Map();
const runtimeErrors = [];
let traceEvents = null;
let traceDone = null;
const report = {
  config: { messages: MESSAGES, cpuRate: CPU_RATE, scrollSteps: SCROLL_STEPS, typedChars: TYPED.length, privacyProxy: PRIVACY, whatIfCss: env.MC_PERF_CSS_FILE ? path.basename(env.MC_PERF_CSS_FILE) : null, viewport: '390x844 mobile', browser: 'Edge headless --disable-gpu', dist },
  notes: [
    'scriptMs/layoutMs/styleMs: Performance.getMetrics, tempo de CPU da thread principal, já com throttle.',
    'messageRenders: mensagens cujo objeto de props mudou no intervalo (hook de DevTools, só nas fases de contagem).',
    'trace: soma por nome de evento e thread. Pintura e raster são de software (sem GPU); não é o custo na GPU do aparelho.',
    'frameGapMs: intervalos de requestAnimationFrame no headless; indicativo, não equivale a 90–120 Hz.',
    'idle: animações CSS em curso (document.getAnimations) e eventos de quadro do compositor em 2 s, com a tela parada e em "Pensando…". É a medida direta de "tela parada = zero quadros"; o headless sem GPU não mede o custo de raster por quadro.',
  ],
};

try {
  let port;
  for (let n = 0; n < 120; n++) {
    try { port = (await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; break; } catch { await pause(100); }
  }
  assert.ok(port, 'Edge iniciou o depurador em 12 segundos');
  const target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === 'page');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve, { once: true }); ws.addEventListener('error', reject, { once: true }); });
  ws.addEventListener('message', ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.id) {
      const p = pending.get(msg.id);
      if (!p) return;
      pending.delete(msg.id);
      clearTimeout(p.timer);
      if (msg.error) p.reject(new Error(msg.error.message)); else p.resolve(msg.result);
    } else if (msg.method === 'Runtime.exceptionThrown') {
      runtimeErrors.push(msg.params.exceptionDetails.text + ':' + (msg.params.exceptionDetails.exception?.description || ''));
    } else if (msg.method === 'Tracing.dataCollected' && traceEvents) {
      for (const e of msg.params.value) traceEvents.push(e);
    } else if (msg.method === 'Tracing.tracingComplete') {
      traceDone?.();
    }
  });
  const cdp = (method, params = {}, timeout = 60000) => new Promise((resolve, reject) => {
    const id = ++seq;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, timeout);
    pending.set(id, { resolve, reject, timer });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const r = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  const waitFor = async (expression) => {
    for (let n = 0; n < 200; n++) { if (await evaluate(`!!(${expression})`)) return; await pause(75); }
    const snapshot = await evaluate('location.href + " | " + document.body.innerText.slice(0, 160).replace(/\\s+/g, " ")').catch(() => '?');
    throw new Error(`DOM timeout: ${expression.slice(0, 90)} :: ${snapshot}`);
  };
  const readMetrics = async () => Object.fromEntries((await cdp('Performance.getMetrics')).metrics.filter((m) => METRICS.includes(m.name)).map((m) => [m.name, m.value]));
  const metricDelta = (a, b) => ({
    scriptMs: toMs(b.ScriptDuration - a.ScriptDuration),
    layoutMs: toMs(b.LayoutDuration - a.LayoutDuration),
    styleMs: toMs(b.RecalcStyleDuration - a.RecalcStyleDuration),
    taskMs: toMs(b.TaskDuration - a.TaskDuration),
    layouts: b.LayoutCount - a.LayoutCount,
    styleRecalcs: b.RecalcStyleCount - a.RecalcStyleCount,
    heapMB: r2(b.JSHeapUsedSize / 1048576),
  });
  const screenshot = async (name) => {
    const { data } = await cdp('Page.captureScreenshot', { format: 'png' });
    await fs.writeFile(path.join(out, name), Buffer.from(data, 'base64'));
  };
  const frameSettle = 'new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(r, 30))))';
  const setDraft = (value) => evaluate(`(() => { const t = document.querySelector('textarea'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(t, ${JSON.stringify(value)}); t.dispatchEvent(new Event('input', { bubbles: true })); })()`);
  const typeTimed = async () => {
    await setDraft('');
    await evaluate('document.querySelector("textarea").focus()');
    const keys = [];
    for (const ch of TYPED) {
      const a = await readMetrics();
      await cdp('Input.insertText', { text: ch });
      await evaluate(frameSettle);
      keys.push(metricDelta(a, await readMetrics()));
    }
    assert.equal(await evaluate('document.querySelector("textarea").value'), TYPED, 'texto digitado chegou ao compositor');
    return keys;
  };
  const typeCounted = async () => {
    await setDraft('');
    await evaluate('window.__mcPerf.start()');
    for (const ch of TYPED) { await cdp('Input.insertText', { text: ch }); await evaluate(frameSettle); }
    const counted = await evaluate('window.__mcPerf.stop()');
    await setDraft('');
    return counted;
  };
  const keyReport = (keys) => {
    const col = (f) => stat(keys.map((k) => k[f]));
    return { scriptMs: col('scriptMs'), layoutMs: col('layoutMs'), styleMs: col('styleMs'), layoutsPerKey: col('layouts'), styleRecalcsPerKey: col('styleRecalcs'), totalScriptMs: r2(keys.reduce((s, k) => s + k.scriptMs, 0)) };
  };
  const reloadRevision = (type = 'flags') => evaluate(`window.dispatchEvent(new CustomEvent('mc:event', { detail: { convId: 'perf', revision: ${chat.revision}, type: ${JSON.stringify(type)} } }))`);

  await cdp('Page.enable');
  await cdp('Runtime.enable');
  await cdp('Performance.enable');
  await cdp('Emulation.setCPUThrottlingRate', { rate: CPU_RATE });
  await cdp('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'no-preference' }] });
  await cdp('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  await cdp('Page.addScriptToEvaluateOnNewDocument', { source: `(${installPerfHook.toString()})()` });
  if (EXTRA_CSS) {
    // Before the HTML exists, documentElement is null: wait for the root element to appear.
    await cdp('Page.addScriptToEvaluateOnNewDocument', { source: `(function(){const add=()=>{const s=document.createElement('style');s.textContent=${JSON.stringify(EXTRA_CSS)};(document.head||document.documentElement).append(s)};if(document.documentElement)add();else new MutationObserver((_,o)=>{if(document.documentElement){o.disconnect();add();}}).observe(document,{childList:true,subtree:true});})()` });
  }

  // Opening: cold (no local cache) and then warm (IndexedDB cache from the same profile session).
  const openChat = async () => {
    await cdp('Page.navigate', { url: base });
    await waitFor(`Array.from(document.querySelectorAll('button')).some((b) => b.textContent.includes(${JSON.stringify(TITLE)}))`);
    const before = await readMetrics();
    await evaluate(`window.__openWatch = (${watchOpen.toString()})(${JSON.stringify(LAST_ID)}); Array.from(document.querySelectorAll('button')).find((b) => b.textContent.includes(${JSON.stringify(TITLE)})).click(); true`);
    const marks = await evaluate('window.__openWatch');
    const metrics = metricDelta(before, await readMetrics());
    const elements = await evaluate('document.querySelectorAll("*").length');
    return { marksMs: Object.fromEntries(Object.entries(marks).map(([k, v]) => [k, typeof v === 'number' ? r2(v) : v])), metrics: { ...metrics, elements } };
  };
  report.open = { cold: await openChat() };
  await screenshot('chat-390.png');
  report.open.warm = await openChat();

  // Geometry: the header with backdrop-filter sits above the scroll area without overlapping the content.
  report.headerGeometry = await evaluate(`(() => {
    const h = document.querySelector('.mc-chat-header').getBoundingClientRect();
    const s = document.querySelector('.mc-scroll').getBoundingClientRect();
    return { headerBottom: Math.round(h.bottom), scrollTop: Math.round(s.top), overlapPx: Math.max(0, Math.round(h.bottom - s.top)), backdrop: getComputedStyle(document.querySelector('.mc-chat-header')).backdropFilter };
  })()`);

  // (a) Programmatic scroll from top to bottom. Blur is removed by injected CSS only in the "no blur" phases.
  const setBlur = (enabled) => evaluate(enabled
    ? `document.getElementById('perf-noblur')?.remove()`
    : `(() => { const s = document.createElement('style'); s.id = 'perf-noblur'; s.textContent = '.bg-headerDark,.bg-sheet{backdrop-filter:none!important;-webkit-backdrop-filter:none!important}'; (document.head || document.documentElement).append(s); })()`);
  const scrollRun = async (trace) => {
    const m0 = await readMetrics();
    let done = null;
    if (trace) {
      traceEvents = [];
      done = new Promise((r) => { traceDone = r; });
      await cdp('Tracing.start', { categories: TRACE_CATS, transferMode: 'ReportEvents' });
    }
    const run = await evaluate(`(${scrollThrough.toString()})(${SCROLL_STEPS})`);
    let traceSummary = null;
    if (trace) {
      await cdp('Tracing.end');
      await Promise.race([done, pause(15000)]);
      traceSummary = summarizeTrace(traceEvents);
      traceEvents = null;
      traceDone = null;
    }
    const m1 = metricDelta(m0, await readMetrics());
    const gap = stat(run.gaps);
    return {
      ...m1,
      scrollEvents: run.scrollEvents,
      scriptMsPerScrollEvent: run.scrollEvents ? r2(m1.scriptMs / run.scrollEvents) : null,
      wallMs: run.wallMs,
      frameGapMs: gap && { ...gap, over25: run.gaps.filter((g) => g > 25).length, n: run.gaps.length },
      trace: traceSummary,
    };
  };
  await setBlur(true);
  report.scroll = { base: await scrollRun(false), blurRuns: [], noBlurRuns: [] };
  for (const withBlur of [true, false, true, false, true, false]) { // alternated to remove order/drift effects
    await setBlur(withBlur);
    (withBlur ? report.scroll.blurRuns : report.scroll.noBlurRuns).push(await scrollRun(true));
  }
  await setBlur(true);
  const med = (runs, f) => stat(runs.map(f))?.median;
  report.scroll.blurSummary = {
    withBlur: { taskMs: med(report.scroll.blurRuns, (r) => r.taskMs), scriptMsPerScrollEvent: med(report.scroll.blurRuns, (r) => r.scriptMsPerScrollEvent), rasterMs: med(report.scroll.blurRuns, (r) => r.trace.rasterMs), paintMs: med(report.scroll.blurRuns, (r) => r.trace.top['CrRendererMain | Paint']?.ms ?? 0) },
    withoutBlur: { taskMs: med(report.scroll.noBlurRuns, (r) => r.taskMs), scriptMsPerScrollEvent: med(report.scroll.noBlurRuns, (r) => r.scriptMsPerScrollEvent), rasterMs: med(report.scroll.noBlurRuns, (r) => r.trace.rasterMs), paintMs: med(report.scroll.noBlurRuns, (r) => r.trace.top['CrRendererMain | Paint']?.ms ?? 0) },
  };

  // (b) Isolated rememberReading cost (replica of the ChatView.jsx snippet), at 13 scroll positions.
  const rr = await evaluate(`(${benchRememberReading.toString()})(12)`);
  report.rememberReading = { clean: stat(rr.clean), dirtyLayout: stat(rr.dirty) };

  // (c) Typing: cost per keystroke with 40, 160 and all messages. Each size reloads the fictitious history.
  const sizes = [...new Set([40, 160, MESSAGES])];
  report.typingBySize = [];
  for (const size of sizes) {
    chat.history = history.slice(0, size);
    chat.revision++;
    await reloadRevision();
    await waitFor(`document.querySelectorAll('[data-message-id]').length === ${size}`);
    await pause(300);
    const keys = await typeTimed();
    const counted = await typeCounted();
    report.typingBySize.push({
      messages: size,
      ...keyReport(keys),
      messageRendersPerKey: r2((counted.renders.user + counted.renders.assistant) / TYPED.length),
      commitsPerKey: r2(counted.commits / TYPED.length),
      totalMessageRenders: counted.renders,
    });
  }
  chat.history = history;
  chat.revision++;
  await reloadRevision();
  await waitFor(`document.querySelectorAll('[data-message-id]').length === ${MESSAGES}`);
  report.typingFit = fit(report.typingBySize.map((s) => ({ x: s.messages, y: s.scriptMs.mean })));
  report.typingFitNote = 'custo por tecla (ms, média) = fixo + inclinação × mensagens; ajuste linear sobre os tamanhos medidos';

  // (d) Reload without content changes (the "flags" event ignores the revision): how many messages render again.
  await evaluate('window.__mcPerf.start()');
  const r0 = await readMetrics();
  await reloadRevision('flags');
  await pause(1500);
  const reloadMetrics = metricDelta(r0, await readMetrics());
  const reloadCounted = await evaluate('window.__mcPerf.stop()');
  report.reloadSameContent = { ...reloadMetrics, commits: reloadCounted.commits, messageRenders: reloadCounted.renders };

  // (e) Idle screen and "Thinking…": running animations and compositor frames produced in 2 s.
  const FRAME_EVENTS = ['BeginMainThreadFrame', 'DrawFrame', 'ActivateLayerTree'];
  const idleProbe = async () => {
    traceEvents = [];
    const done = new Promise((r) => { traceDone = r; });
    await cdp('Tracing.start', { categories: TRACE_CATS, transferMode: 'ReportEvents' });
    const anim = await evaluate(`new Promise((r) => setTimeout(() => r({ cssAnimations: document.getAnimations().length, running: document.getAnimations().filter((a) => a.playState === 'running').length }), 2000))`);
    await cdp('Tracing.end');
    await Promise.race([done, pause(15000)]);
    const frames = Object.fromEntries(FRAME_EVENTS.map((n) => [n, traceEvents.filter((e) => e.name === n).length]));
    traceEvents = null;
    traceDone = null;
    return { windowMs: 2000, ...anim, frames };
  };
  await pause(500);
  report.idle = { atRest: await idleProbe() };
  await evaluate(`document.querySelector('[aria-label="Continuar a cena"]').click()`);
  await waitFor(`document.querySelector('.mc-generation')`);
  await pause(500);
  report.idle.thinking = await idleProbe();

  report.apiRequestsUnknown = [...unknownApi];
  report.chatFetches = hits.chat;
  report.runtimeErrors = runtimeErrors.length;
  await fs.writeFile(path.join(out, 'perf-results.json'), JSON.stringify(report, null, 2));
  const typed = report.typingBySize.at(-1);
  console.log(JSON.stringify({
    open: { coldLastMsgMs: report.open.cold.marksMs.last, warmLastMsgMs: report.open.warm.marksMs.last, coldScriptMs: report.open.cold.metrics.scriptMs, coldLayoutMs: report.open.cold.metrics.layoutMs, warmScriptMs: report.open.warm.metrics.scriptMs },
    scrollScriptMsPerEvent: { median: report.scroll.blurSummary.withBlur.scriptMsPerScrollEvent, noBlur: report.scroll.blurSummary.withoutBlur.scriptMsPerScrollEvent, base: report.scroll.base.scriptMsPerScrollEvent },
    scrollRasterMs: { withBlur: report.scroll.blurSummary.withBlur.rasterMs, withoutBlur: report.scroll.blurSummary.withoutBlur.rasterMs },
    rememberReadingMs: { clean: report.rememberReading.clean?.median, dirtyLayout: report.rememberReading.dirtyLayout?.median },
    typingBySize: report.typingBySize.map((s) => ({ messages: s.messages, scriptMsMedian: s.scriptMs.median, layoutsPerKey: s.layoutsPerKey.median, messageRendersPerKey: s.messageRendersPerKey })),
    typingFit: report.typingFit,
    reloadSameContent: { scriptMs: reloadMetrics.scriptMs, messageRenders: reloadCounted.renders },
    idle: report.idle,
    headerOverlapPx: report.headerGeometry.overlapPx,
    runtimeErrors: runtimeErrors.length,
    results: path.join(out, 'perf-results.json'),
  }, null, 1));
  assert.deepEqual(runtimeErrors, []);
} finally {
  ws?.close();
  for (const p of pending.values()) clearTimeout(p.timer);
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
  edge.kill();
  await Promise.race([new Promise((r) => edge.once('exit', r)), pause(3000)]);
  await fs.rm(profile, { recursive: true, force: true, maxRetries: 4, retryDelay: 200 }).catch(() => {});
}
