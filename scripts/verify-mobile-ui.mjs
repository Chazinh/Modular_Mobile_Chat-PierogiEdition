// Reproducible UI check: built React app, fictitious HTTP/SSE, disposable headless Edge profile.
// No personal data, provider, production server, or additional browser/UI dependency.
import strictAssert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { randomBytes } from 'node:crypto';

let assertionCount = 0;
const assert = new Proxy(strictAssert, {
  get(target, key) {
    const value = Reflect.get(target, key);
    return typeof value === 'function' ? (...args) => { assertionCount++; return Reflect.apply(value, target, args); } : value;
  },
});

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.resolve(process.env.MC_UI_VERIFICATION_DIR || await fs.mkdtemp(path.join(os.tmpdir(), 'mc-ui-evidence-')));
await fs.mkdir(out, { recursive: true });
const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-ui-edge-'));
const privateFixture = process.env.MC_UI_PRIVACY === '1';
const privateCacheKey = randomBytes(32).toString('base64');
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
const dist = process.env.MC_UI_DIST ? path.resolve(process.env.MC_UI_DIST) : path.join(root, 'client/dist');
// HomeDrawer replaced the old settings entry. Keep coverage of GlobalSettings' colors,
// trash confirmations and language without adding a production route or removing assertions.
const harness = path.join(out, 'settings-fixture');
await fs.mkdir(harness, { recursive: true });
const client = path.join(root, 'client');
const modulePath = file => JSON.stringify(path.join(client, file).replaceAll('\\', '/'));
const entry = path.join(harness, 'entry.jsx');
await fs.writeFile(entry, `
  import React, { useState } from 'react';
  import { createRoot } from 'react-dom/client';
  import GlobalSettings from ${modulePath('src/components/GlobalSettings.jsx')};
  import { ConfirmProvider } from ${modulePath('src/components/ConfirmProvider.jsx')};
  import { tr, useLanguage } from ${modulePath('src/i18n.js')};
  function Fixture() {
    useLanguage();
    const [open, setOpen] = useState(false), [fontSize, setFontSize] = useState(22);
    return <ConfirmProvider>{open
      ? <GlobalSettings fontSize={fontSize} onFontSize={setFontSize} onClose={() => setOpen(false)} onChanged={() => {}} onResetServer={() => {}} />
      : <button aria-label={tr('Configurações')} onClick={() => setOpen(true)}>{tr('Configurações')}</button>}</ConfirmProvider>;
  }
  createRoot(document.getElementById('root')).render(<Fixture />);
`);
const { build } = await import(pathToFileURL(path.join(client, 'node_modules/vite/dist/node/index.js')).href);
await build({ root: client, configFile: false, logLevel: 'error', css: { postcss: { plugins: [] } },
  resolve: { alias: { react: path.join(client, 'node_modules/react'), 'react-dom': path.join(client, 'node_modules/react-dom') } },
  build: { outDir: path.join(harness, 'dist'), emptyOutDir: true, rollupOptions: { input: entry, output: { entryFileNames: 'settings.js' } } },
});
const styles = (await fs.readdir(path.join(dist, 'assets'))).filter(file => file.endsWith('.css'));
const harnessHtml = `<!doctype html><html lang="pt-BR"><head><meta name="viewport" content="width=device-width, initial-scale=1">${styles.map(file => `<link rel="stylesheet" href="/assets/${file}">`).join('')}</head><body><div id="root"></div><script type="module" src="/__settings-assets/settings.js"></script></body></html>`;
const initial = () => ({ id: 'fixture', name: 'Cena fictícia', persona: 'Alex', presetId: 'claude-sonnet-5-5', routeId: 'cc/claude-sonnet-5-5', effort:'medium', thinkingEnabled:true,settings:{thinkingEnabled:true}, revision: 1, characterRev: 1, memorySummary: {}, history: [
  { id: 'u1', role: 'user', content: '*Entro na biblioteca.* O que você encontrou?', interactionId: 'i1' },
  { id: 'a1', role: 'assistant', content: 'Mira: Encontrei uma carta.\n*Ela abre o envelope com cuidado.*', status: 'complete', versionCount: 1 },
  { id: 'a2', role: 'assistant', content: 'Mira: Podemos ler juntas. A biblioteca guarda histórias de várias épocas.\n*Uma luz violeta ilumina a mesa.* ' + 'palavra'.repeat(36), status: 'complete', versionCount: 1 },
] });
let chat = initial(), failChat = false, failPresets=false, delayChat=null, active = null, generation=0, failEdit=false, failMessage=false;
let globalStyle={botChars:8000,dialoguePct:65,content:'',lengthMode:'maximum',monologueChars:300,turnPacing:'interactive'};
let lastDone=null;
let destroyAccepted;
const calls = [], httpRequests = [], errors = [], widths = [], feedback = [], languageDropdown = [], readingChecks = [], scaleChecks = [], transportChecks = [];
let purges = 0;
const visibleReply='Mira: 🚀 RESPOSTA_PRONTA_✨\n*Ela sorri.*';
function complete(status='complete'){
  if(!active)return;
  const msg={id:active.partId,role:'assistant',content:visibleReply,status,versionCount:2};
  const idx=chat.history.findIndex(m=>m.id===msg.id);if(idx>=0)chat.history[idx]=msg;else chat.history.push(msg);
  chat.revision++;
  lastDone={done:true,status,message:msg,revision:chat.revision};
  active.res.end(`data: ${JSON.stringify(lastDone)}\n\n`);active=null;
}
const json = (res, obj) => { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(obj)); };
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://fixture');
    if (url.pathname.startsWith('/api/')) httpRequests.push({ method: req.method, path: url.pathname });
    if (url.pathname === '/__settings-fixture') { res.writeHead(200, { 'Content-Type': 'text/html' }); return res.end(harnessHtml); }
    if (url.pathname === '/api/devices') return json(res, { devices: [] });
    if (url.pathname === '/api/trash') return json(res, { days: 30, items: [{ id: 'trash-fixture', type: 'conversation', name: 'Cena fictícia', count: 3, deletedAt: new Date().toISOString(), preview: 'Mira: *Olá.*' }] });
    if (url.pathname === '/api/trash/purge' && req.method === 'POST') { purges++; return json(res, { ok: true }); }
    if (url.pathname === '/api/status') return json(res, { router: { ok: true, models: 4 }, local: { chat: { ok: false }, embed: { ok: false } }, db: { jobs: {} } });
    if (url.pathname === '/api/chats/fixture/memory') return json(res, { state: null, sync: { synced: true, lag: 0 }, cast: [], memories: [], relationships: [], threads: [], episodes: [], jobs: [], local: { chat: { ok: false, reason: 'offline' }, embed: { ok: false } } });
    if(privateFixture&&url.pathname==='/api/privacy/cache-key')return json(res,{key:privateCacheKey});
    if (url.pathname === '/api/chats') return json(res, { chats: [{ ...chat, history: undefined, snippet: 'Conversa fictícia para verificação' }] });
    if (url.pathname === '/api/chats/fixture') { if (failChat) return req.socket.destroy(); if(req.method==='PUT'){let raw='';for await(const c of req)raw+=c;const patch=JSON.parse(raw);chat={...chat,...patch,settings:{...chat.settings,...patch.settings},revision:chat.revision+1};chat.thinkingEnabled=chat.settings.thinkingEnabled!==false;calls.push({thinkingEnabled:chat.thinkingEnabled});} if(delayChat)await delayChat; return json(res, {...chat,generation:active?{id:'g1',mode:active.mode,partId:active.partId}:null}); }
    if(url.pathname==='/api/settings/global'){
      if(req.method==='PUT'){let raw='';for await(const c of req)raw+=c;globalStyle={...globalStyle,...JSON.parse(raw)};calls.push({style:globalStyle.dialoguePct});}
      return json(res,{settings:globalStyle});
    }
    if(req.method==='PUT'&&url.pathname.includes('/messages/')){
      let raw='';for await(const c of req)raw+=c;
      if(failEdit){res.writeHead(503,{'Content-Type':'application/json'});return res.end('{"error":"Falha simulada no salvamento"}');}
      const id=url.pathname.split('/').at(-1),m=chat.history.find(m=>m.id===id);
      m.content=JSON.parse(raw).content;m.edited=true;m.versionCount=2;chat.revision++;
      calls.push({edited:id,content:m.content});return json(res,{...m,revision:chat.revision});
    }
    if (url.pathname === '/api/presets') {if(failPresets)return req.socket.destroy();return json(res, { presets: [{ id: 'claude-sonnet-5-5', label: 'Fixture', thinkingCanDisable:true,thinking:'adaptive',efforts:['low','medium'],defaultEffort:'medium',routes:[{id:'cc/claude-sonnet-5-5'}] }] });}
    if (url.pathname === '/api/chats/fixture/cast') return json(res, { cast: [] });
    if (url.pathname === '/api/events') { res.writeHead(200, { 'Content-Type': 'text/event-stream' }); return res.end('data: {"type":"hello"}\n\n'); }
    if (url.pathname.endsWith('/message')) {
      let raw = ''; for await (const chunk of req) raw += chunk;
      const body = JSON.parse(raw); calls.push(body);
      if(failMessage==='rejected') {
        res.writeHead(503, {'Content-Type':'application/json'});
        return res.end(JSON.stringify({error:'FALHA_PROVEDOR_TESTE'}));
      }
      if(failMessage==='pre-headers')return req.socket.destroy(); // Chromium may retry below fetch; preserve every physical opId.
      res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
      if(failMessage==='accepted-sse') {
        res.flushHeaders(); res.write(': accepted transport, no operation ack\n\n');
        destroyAccepted=()=>res.destroy(); // The test drops it only after fetch has observed HTTP 200.
        return;
      }
      const partId=body.mode==='regenerate'&&chat.history.at(-1)?.role==='assistant'?chat.history.at(-1).id:`a-new-${++generation}`;
      const userMessage=['send','edit'].includes(body.mode)?{id:`u-new-${generation}`,role:'user',content:body.content,interactionId:`i-new-${generation}`}:null;
      if(body.mode==='edit'){const i=chat.history.findIndex(m=>m.interactionId===body.messageId&&m.role==='user');chat.history=chat.history.slice(0,i);}
      if(userMessage)chat.history.push(userMessage);
      active = { res, mode: body.mode,partId,opId:body.opId };
      res.write(`data: ${JSON.stringify({ ack: { opId:body.opId,generationId: 'g1', mode: body.mode, partId, userMessage,revision: ++chat.revision } })}\n\n`);
      res.write(`data: ${JSON.stringify({ delta: visibleReply.slice(0,15),offset:15 })}\n\n`);
      if(body.mode==='send'){res.end();return;}
      res.write(`data: ${JSON.stringify({delta:visibleReply.slice(15),offset:visibleReply.length})}\n\n`);
      return;
    }
    if(url.pathname.endsWith('/g1/stream')){
      calls.push({reconnect:Number(url.searchParams.get('from'))});
      res.writeHead(200,{'Content-Type':'text/event-stream'});
      if(!active)return res.end(`data: ${JSON.stringify(lastDone)}\n\n`);
      active.res=res;
      res.write(`data: ${JSON.stringify({ack:{opId:active.opId,generationId:'g1',mode:active.mode,partId:active.partId}})}\n\n`);
      const offset=Number(url.searchParams.get('from'));
      res.write(`data: ${JSON.stringify({delta:visibleReply.slice(offset),offset:visibleReply.length})}\n\n`);return;
    }
    if (url.pathname.endsWith('/g1/stop')) {
      calls.push({ stop: true });
      complete('interrupted');
      return json(res, { ok: true });
    }
    if (url.pathname.startsWith('/api/')) { res.writeHead(404); return res.end('{"error":"fixture route absent"}'); }
    const staticRoot = url.pathname.startsWith('/__settings-assets/') ? path.join(harness, 'dist') : dist;
    const pathname = staticRoot === dist ? url.pathname : url.pathname.slice('/__settings-assets'.length);
    const file = path.resolve(staticRoot, pathname === '/' ? 'index.html' : '.' + pathname);
    assert.ok(file.startsWith(staticRoot + path.sep));
    const data = await fs.readFile(file);
    res.writeHead(200, { 'Content-Type': file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html' }); res.end(data);
  } catch { if (!res.headersSent) res.writeHead(500); res.end(); }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;
const edge = spawn(process.env.MC_EDGE || 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', [
  '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
], { windowsHide: true, stdio: 'ignore' });
let ws, seq = 0, inspectFailure; const pending = new Map();
try {
  let port;
  for (let n = 0; n < 120; n++) { try { port = (await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; break; } catch { await pause(100); } }
  assert.ok(port, 'Edge debugger started within 12 seconds');
  const target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === 'page');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve, { once: true }); ws.addEventListener('error', reject, { once: true }); });
  ws.addEventListener('message', ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.id) { const p = pending.get(msg.id); if (!p) return; pending.delete(msg.id); clearTimeout(p.timer); msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result); }
    if (msg.method === 'Runtime.exceptionThrown') errors.push(msg.params.exceptionDetails.text + ':' + (msg.params.exceptionDetails.exception?.description || ''));
  });
  const cdp = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++seq; const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 12000);
    pending.set(id, { resolve, reject, timer }); ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const r = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  const waitFor = async (expression) => { for (let n = 0; n < 80; n++) { if (await evaluate(`!!(${expression})`)) return; await pause(75); } throw new Error(`DOM timeout: ${expression}`); };
  const click = (label) => evaluate(`document.querySelector('button[aria-label=${JSON.stringify(label)}]').click()`);
  const screenshot = async (name) => { const { data } = await cdp('Page.captureScreenshot', { format: 'png' }); await fs.writeFile(path.join(out, name + '.png'), Buffer.from(data, 'base64')); };
  inspectFailure = async () => { await screenshot('failure'); return evaluate(`({url:location.href,text:document.body.innerText.slice(0,2400),focus:document.activeElement?.outerHTML.slice(0,400)})`); };
  const refresh = async () => { await evaluate(`document.dispatchEvent(new Event('visibilitychange'))`); await pause(180); };
  await cdp('Page.enable'); await cdp('Runtime.enable');
  await cdp('Emulation.setEmulatedMedia', {features:[{name:'prefers-reduced-motion',value:'no-preference'}]});
  await cdp('DOM.enable'); await cdp('CSS.enable');
  const noActionFlash = async () => {
    const {root:doc} = await cdp('DOM.getDocument');
    const {nodeIds} = await cdp('DOM.querySelectorAll',{nodeId:doc.nodeId,selector:'.mc-chat-action'});
    assert.ok(nodeIds.length,'message actions present for feedback verification');
    for (const nodeId of nodeIds) {
      for (const forcedPseudoClasses of [['active'],['hover'],['active','hover']]) {
        await cdp('CSS.forcePseudoState',{nodeId,forcedPseudoClasses});
        const {object} = await cdp('DOM.resolveNode',{nodeId});
        const {result} = await cdp('Runtime.callFunctionOn',{objectId:object.objectId,returnByValue:true,functionDeclaration:`function(){const s=getComputedStyle(this);return {label:this.getAttribute('aria-label'),background:s.backgroundColor,image:s.backgroundImage,filter:s.filter,tap:s.webkitTapHighlightColor,before:getComputedStyle(this,'::before').content,after:getComputedStyle(this,'::after').content}}`});
        const style=result.value;feedback.push({...style,state:forcedPseudoClasses});
        assert.equal(style.background,'rgba(0, 0, 0, 0)',`${style.label}: transparent feedback`);
        assert.equal(style.image,'none');assert.equal(style.filter,'none');assert.equal(style.tap,'rgba(0, 0, 0, 0)');
        assert.ok(['none','normal'].includes(style.before)&&['none','normal'].includes(style.after),'no generated overlay');
        await cdp('Runtime.releaseObject',{objectId:object.objectId});
      }
      await cdp('CSS.forcePseudoState',{nodeId,forcedPseudoClasses:[]});
    }
  };
  await cdp('Page.addScriptToEvaluateOnNewDocument', { source: 'try{localStorage.setItem("mc.font", "22")}catch{}' });
  await cdp('Page.addScriptToEvaluateOnNewDocument', { source: 'Object.defineProperty(navigator,"clipboard",{value:{writeText:async text=>{window.__copied=text}}});' });
  // Install before any navigation/app code. These are application fetch calls, not physical HTTP attempts.
  await cdp('Page.addScriptToEvaluateOnNewDocument', { source: `
    window.__fetchCalls=[];
    const originalFetch=window.fetch;
    window.fetch=function(input,init) {
      const request={method:init?.method||input?.method||'GET',path:new URL(input?.url||input,location.href).pathname};
      if(typeof init?.body==='string') { try { const body=JSON.parse(init.body);request.opId=body.opId;request.mode=body.mode; } catch {} }
      window.__fetchCalls.push(request);
      return Reflect.apply(originalFetch,this,arguments).then(response=>{
        request.status=response.status;request.contentType=response.headers.get('content-type');return response;
      },error=>{request.error=error.message;throw error;});
    };
  ` });
  await cdp('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  await cdp('Page.navigate', { url: base });
  await waitFor(`Array.from(document.querySelectorAll('button')).some(b => b.textContent.includes('Cena fictícia'))`);
  await screenshot('list-refined-390');

  // Only interface language uses an anchored list; colors and other choices retain sheets.
  const textButton = text => evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(text)});if(!b)throw Error('Button missing: '+${JSON.stringify(text)});b.click()})()`);
  const inputValue = (label, value) => evaluate(`(()=>{const i=document.querySelector('input[aria-label=${JSON.stringify(label)}]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(i,${JSON.stringify(value)});i.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  const layoutFrame = () => evaluate(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
  const pressKey = async key => {
    const keys = { Enter: ['Enter', 13], ' ': ['Space', 32], Escape: ['Escape', 27], ArrowDown: ['ArrowDown', 40], ArrowUp: ['ArrowUp', 38], Home: ['Home', 36], End: ['End', 35], Tab: ['Tab', 9] };
    const [code, windowsVirtualKeyCode] = keys[key];
    const text = key === 'Enter' ? '\r' : key === ' ' ? ' ' : undefined;
    await cdp('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key, code, windowsVirtualKeyCode, nativeVirtualKeyCode: windowsVirtualKeyCode, text });
    await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode, nativeVirtualKeyCode: windowsVirtualKeyCode });
  };
  const tap = async selector => {
    const point = await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
    await cdp('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [point] });
    await cdp('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  };
  const languageSelector = '.mc-choice-trigger[aria-haspopup="listbox"]';
  const dropdownGeometry = async scenario => {
    await layoutFrame();
    const result = await evaluate(`(()=>{
      const b=document.querySelector(${JSON.stringify(languageSelector)}),p=document.querySelector('[role="listbox"]');
      const box=e=>{const r=e.getBoundingClientRect();return {top:r.top,bottom:r.bottom,left:r.left,right:r.right,width:r.width,height:r.height}};
      const viewport=visualViewport;
      return {trigger:box(b),list:box(p),viewport:{left:viewport.offsetLeft,top:viewport.offsetTop,width:viewport.width,height:viewport.height},
        linked:b.getAttribute('aria-controls')===p.id&&p.getAttribute('aria-labelledby')===b.id,expanded:b.getAttribute('aria-expanded'),
        options:[...p.querySelectorAll('[role="option"]')].map(o=>({lang:o.lang,selected:o.getAttribute('aria-selected'),hit:(()=>{const r=o.getBoundingClientRect();return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest('[role="option"]')===o})()})),
        sheets:document.querySelectorAll('.mc-control-sheet,.mc-sheet-overlay').length,scroll:b.closest('.overflow-y-auto').scrollTop};
    })()`);
    const { trigger: b, list: p, viewport: v } = result;
    assert.ok(result.linked && result.expanded === 'true', 'listbox has an accessible trigger relationship');
    assert.equal(result.sheets, 0, 'language creates neither a bottom sheet nor a full-screen scrim');
    assert.equal(result.options.length, 2);
    assert.equal(result.options.filter(o => o.selected === 'true').length, 1);
    assert.ok(Math.abs(p.left - b.left) <= 1 && Math.abs(p.width - b.width) <= 1, 'list aligns with the language field');
    assert.ok(Math.abs(p.top - b.bottom - 6) <= 1 || Math.abs(b.top - p.bottom - 6) <= 1, 'list stays beside the field, above or below');
    assert.ok(p.top >= v.top + 7 && p.bottom <= v.top + v.height - 7 && p.left >= v.left + 7 && p.right <= v.left + v.width - 7, 'list fits the visual viewport');
    assert.ok(result.options.every(o => o.hit), 'language options render and receive taps above neighboring cards');
    languageDropdown.push({ scenario, ...result });
    return result;
  };
  const dropdownClosed = async (language, depth, context = 'selection') => {
    await waitFor(`!document.querySelector('[role="listbox"]') && history.state.mcDepth===${depth}`);
    assert.equal(await evaluate(`document.activeElement===document.querySelector(${JSON.stringify(languageSelector)})`), true, `focus returns to the language field (${context})`);
    assert.equal(await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).getAttribute('aria-expanded')`), 'false');
    assert.equal(await evaluate(`document.documentElement.lang`), language);
    assert.ok(await evaluate(`!!document.querySelector('.mc-screen')`), 'only the dropdown closes');
  };
  const authoredBefore = JSON.stringify(chat);
  const callsBefore = calls.length;
  await cdp('Page.navigate', { url: `${base}/__settings-fixture` });
  await waitFor(`document.querySelector('[aria-label="Configurações"]')`);
  await click('Configurações'); await waitFor(`document.querySelector('[aria-label="Idioma da interface"]')`);
  assert.equal(await evaluate(`document.querySelectorAll('select,input[type=color]').length`), 0);
  let settingsDepth = await evaluate(`history.state.mcDepth`);
  await tap(languageSelector); await waitFor(`document.querySelector('[role="listbox"]') && history.state.mcDepth===${settingsDepth + 1}`);
  const originalDropdown = await dropdownGeometry('mobile-390');
  assert.equal(originalDropdown.options.find(o => o.selected === 'true').lang, 'pt-BR');
  await screenshot('language-dropdown-390');
  await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).closest('.overflow-y-auto').scrollTop=24`);
  const scrolledDropdown = await dropdownGeometry('scroll-24');
  assert.ok(scrolledDropdown.trigger.top < originalDropdown.trigger.top - 8, 'the settings scroller actually moved');
  assert.ok(Math.abs((scrolledDropdown.list.top-originalDropdown.list.top)-(scrolledDropdown.trigger.top-originalDropdown.trigger.top)) <= 1, 'scroll moves the list with its origin');
  await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).closest('.overflow-y-auto').scrollTop=0`);
  for (const [width, height, scenario] of [[360,420,'keyboard-space-360'],[844,390,'landscape-844'],[320,300,'compact-320'],[390,844,'restored-390']]) {
    await cdp('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: true });
    await dropdownGeometry(scenario);
    if (scenario === 'compact-320') await screenshot('language-dropdown-compact-320');
  }
  await tap('[role="option"][lang="en"]');
  await waitFor(`document.documentElement.lang==='en' && document.querySelector('[aria-label="Interface language"]')`);
  await dropdownClosed('en', settingsDepth);
  assert.equal(await evaluate(`localStorage.getItem('mc.language')`), 'en');
  assert.equal(await evaluate(`document.querySelector('h2').textContent`), 'Settings');
  assert.equal(JSON.stringify(chat), authoredBefore);
  await screenshot('settings-english-390');
  await cdp('Page.reload'); await waitFor(`document.querySelector('[aria-label="Settings"]')`);
  assert.equal(await evaluate(`document.documentElement.lang`), 'en');
  await cdp('Page.navigate', { url: base }); await waitFor(`document.querySelector('[aria-label="Settings"]')`);
  assert.ok(await evaluate(`document.body.textContent.includes('Cena fictícia')`));
  await cdp('Page.navigate', { url: `${base}/__settings-fixture` }); await waitFor(`document.querySelector('[aria-label="Settings"]')`);
  await click('Settings'); await waitFor(`document.querySelector('[aria-label="Interface language"]')`);
  settingsDepth = await evaluate(`history.state.mcDepth`);
  for (const dismissal of ['Escape', 'outside', 'Back', 'offscreen-scroll']) {
    await click('Interface language'); await waitFor(`document.querySelector('[role="listbox"]') && history.state.mcDepth===${settingsDepth + 1}`);
    assert.equal(await evaluate(`document.querySelector('[role="option"][aria-selected="true"]').lang`), 'en');
    if (dismissal === 'Escape') await pressKey('Escape');
    if (dismissal === 'outside') await tap('.mc-screen section h3');
    if (dismissal === 'Back') await evaluate(`history.back()`);
    if (dismissal === 'offscreen-scroll') await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).closest('.overflow-y-auto').scrollTop=340`);
    await dropdownClosed('en', settingsDepth, dismissal);
    assert.equal(await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).closest('.overflow-y-auto').scrollTop`), dismissal === 'offscreen-scroll' ? 340 : 0, 'focus restoration never scrolls the viewport');
    if (dismissal === 'offscreen-scroll') { await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).closest('.overflow-y-auto').scrollTop=0`); await layoutFrame(); }
  }
  await pressKey('Enter'); await waitFor(`document.querySelector('[role="listbox"]')`);
  for (const [key, language] of [['Home','pt-BR'],['ArrowDown','en'],['ArrowUp','pt-BR'],['End','en']]) {
    await pressKey(key);
    assert.equal(await evaluate(`document.activeElement.lang`), language, `${key} moves option focus`);
    assert.equal(await evaluate(`localStorage.getItem('mc.language')`), 'en', 'navigation alone never selects a language');
  }
  await pressKey('Enter'); await dropdownClosed('en', settingsDepth);
  await pressKey(' '); await waitFor(`document.querySelector('[role="listbox"]')`);
  await pressKey('Home'); await pressKey(' '); await dropdownClosed('pt-BR', settingsDepth);
  assert.equal(await evaluate(`localStorage.getItem('mc.language')`), 'pt-BR');
  await cdp('Page.reload'); await waitFor(`document.querySelector('[aria-label="Configurações"]')`);
  assert.equal(await evaluate(`document.documentElement.lang`), 'pt-BR', 'Portuguese persists after reload');
  await click('Configurações'); await waitFor(`document.querySelector('[aria-label="Idioma da interface"]')`);
  assert.equal(await evaluate(`document.querySelector(${JSON.stringify(languageSelector)}).textContent.trim()`), 'Português (Brasil)');
  await click('Escolher outra cor'); await waitFor(`document.querySelector('[aria-label="Código da cor"]')`);
  assert.equal(await evaluate(`!!document.querySelector('.mc-control-sheet [aria-label="Código da cor"]')`), true, 'colors retain their sheet');
  await inputValue('Código da cor', '#a1b2c3');
  await textButton('Aplicar cor'); await waitFor(`!document.querySelector('[aria-label="Código da cor"]')`);
  assert.equal(await evaluate(`JSON.parse(localStorage.getItem('mc.eyecare')).custom`), '#a1b2c3');
  await click('Escolher outra cor'); await waitFor(`document.querySelector('[aria-label="Código da cor"]')`);
  await inputValue('Código da cor', '#bad');
  await waitFor(`document.querySelector('[aria-invalid="true"]')`);
  assert.equal(await evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Aplicar cor').disabled`), true);
  await cdp('Emulation.setDeviceMetricsOverride', { width: 360, height: 844, deviceScaleFactor: 1, mobile: true });
  await pause(600); await screenshot('color-sheet-360');
  assert.equal(await evaluate(`document.documentElement.scrollWidth>innerWidth`), false);
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))`);
  await waitFor(`!document.querySelector('[role="dialog"]')`);
  assert.equal(await evaluate(`JSON.parse(localStorage.getItem('mc.eyecare')).custom`), '#a1b2c3');
  await textButton('Restaurar padrão');
  await textButton('Esvaziar lixeira'); await waitFor(`document.querySelector('[role="dialog"]')`);
  await pause(600); await screenshot('confirmation-sheet-360');
  await textButton('Cancelar'); await waitFor(`!document.querySelector('[role="dialog"]')`);
  assert.equal(purges, 0, 'cancel never executes a destructive request');
  await textButton('Esvaziar lixeira'); await waitFor(`document.querySelector('[role="dialog"]')`);
  await evaluate(`history.back()`); await waitFor(`!document.querySelector('[role="dialog"]')`);
  assert.ok(await evaluate(`!!document.querySelector('[aria-label="Idioma da interface"]')`), 'Back cancels only the top sheet');
  assert.equal(purges, 0);
  await textButton('Esvaziar lixeira'); await waitFor(`document.querySelector('[role="dialog"]')`);
  await textButton('Confirmar'); await waitFor(`!document.querySelector('[role="dialog"]')`);
  await waitFor(`document.body.textContent.includes('Lixeira esvaziada.')`);
  assert.equal(purges, 1, 'explicit acceptance sends exactly one request');
  assert.equal(calls.length, callsBefore, 'local language and colors never change server/model settings');
  assert.equal(JSON.stringify(chat), authoredBefore);
  await click('Voltar'); await waitFor(`document.querySelector('[aria-label="Configurações"]')`);
  await cdp('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });

  // Also exercise the current user-facing entry, whose language field lives in HomeDrawer.
  await cdp('Page.navigate', { url: base }); await waitFor(`document.querySelector('[aria-label="Configurações"]')`);
  await click('Configurações'); await waitFor(`document.querySelector('.mc-drawer')`);
  await evaluate(`[...document.querySelectorAll('.mc-drawer button')].find(b=>b.textContent.includes('Idioma')).click()`);
  await waitFor(`document.querySelector('[aria-label="Idioma da interface"]')`);
  await pause(600);
  await tap(languageSelector); await waitFor(`document.querySelector('[role="listbox"]')`);
  await dropdownGeometry('home-drawer-390'); await screenshot('home-language-dropdown-390');
  await tap('[role="option"][lang="en"]'); await waitFor(`document.documentElement.lang==='en' && !document.querySelector('[role="listbox"]')`);
  assert.equal(await evaluate(`document.activeElement===document.querySelector(${JSON.stringify(languageSelector)})`), true);
  assert.equal(await evaluate(`localStorage.getItem('mc.language')`), 'en');
  await click('Interface language'); await waitFor(`document.querySelector('[role="listbox"]')`);
  await tap('[role="option"][lang="pt-BR"]'); await waitFor(`document.documentElement.lang==='pt-BR' && !document.querySelector('[role="listbox"]')`);
  assert.equal(await evaluate(`localStorage.getItem('mc.language')`), 'pt-BR');
  await click('Idioma da interface'); await waitFor(`document.querySelector('[role="listbox"]')`);
  await evaluate(`history.back()`); await waitFor(`!document.querySelector('[role="listbox"]')`);
  assert.ok(await evaluate(`!!document.querySelector('[aria-label="Idioma da interface"]')`), 'Back leaves the current language subpage open');
  assert.equal(JSON.stringify(chat), authoredBefore);
  assert.equal(calls.length, callsBefore);
  await cdp('Page.navigate', { url: base }); await waitFor(`document.querySelector('[aria-label="Configurações"]')`);

  await evaluate(`Array.from(document.querySelectorAll('button')).find(b => b.textContent.includes('Cena fictícia')).click()`);
  await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  await screenshot('chat-refined-390');
  await click('Opções'); await waitFor(`document.querySelector('.mc-drawer .mc-row')`);
  await evaluate(`[...document.querySelectorAll('.mc-drawer button')].find(b=>b.textContent.includes('Contexto')).click()`);
  await waitFor(`document.body.textContent.includes('Orçamento orientativo:')`);
  assert.ok(await evaluate(`document.body.textContent.includes('Conteúdo obrigatório não é bloqueado por esse orçamento; o limite real é o da janela de contexto do provedor.')`));
  // Simulate a language change from another tab while this text is mounted.
  await evaluate(`window.dispatchEvent(new StorageEvent('storage',{key:'mc.language',newValue:'en'}))`);
  await waitFor(`document.body.textContent.includes('Guideline budget:')`);
  assert.ok(await evaluate(`document.body.textContent.includes("Required content is not blocked by this budget; the actual limit is the provider's context window.")`));
  await evaluate(`window.dispatchEvent(new StorageEvent('storage',{key:'mc.language',newValue:'pt-BR'}))`);
  await waitFor(`document.body.textContent.includes('Orçamento orientativo:')`);
  await evaluate(`document.querySelector('.mc-drawer > .mc-scrim').click()`); await waitFor(`!document.querySelector('.mc-drawer')`);
  assert.equal(JSON.stringify(chat), authoredBefore);
  assert.equal(calls.length, callsBefore, 'budget text and UI language never modify generation settings');
  await click('Memória'); await waitFor(`document.querySelector('h2')?.textContent==='Memória do servidor ativo'`);
  await textButton('Memórias'); await waitFor(`document.querySelector('[aria-label="Adicionar memória"]')`);
  await click('Adicionar memória'); await waitFor(`document.querySelector('[aria-label="Tipo de memória"]')`);
  await click('Tipo de memória'); await waitFor(`document.querySelector('[role="radiogroup"]')`);
  assert.equal(await evaluate(`!!document.querySelector('.mc-control-sheet [role="radiogroup"]') && !document.querySelector('[role="listbox"]')`), true, 'memory retains the default choice sheet');
  assert.equal(await evaluate(`document.querySelectorAll('select').length`), 0);
  await pause(600); await screenshot('memory-type-sheet-390');
  assert.equal(await evaluate(`(()=>{const b=document.querySelector('[role="radio"][aria-checked="true"]'),r=b.getBoundingClientRect();return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest('[role="radio"]')===b})()`), true, 'nested choice stays above the underlying form');
  await evaluate(`[...document.querySelectorAll('[role="radio"]')].find(b=>b.textContent.trim()==='evento').click()`);
  await waitFor(`document.querySelector('[aria-label="Tipo de memória"]').textContent.trim()==='evento'`);
  await evaluate(`history.back()`); await waitFor(`!document.querySelector('[role="dialog"]')`);
  await click('Voltar'); await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  assert.equal(errors.length, 0, JSON.stringify(errors));
  const interfaceResults = { status: 'passed', scope: 'interface', languageDropdown, budgetTranslations: ['pt-BR', 'en'], runtimeErrors: errors,
    limits: 'Viewport reduzido simula espaço do teclado. IME/teclado e WebView Android físicos não testados.' };
  await fs.writeFile(path.join(out, 'interface-results.json'), JSON.stringify(interfaceResults, null, 2));
  console.log(`PASS: anchored language geometry/scroll/viewport, touch/Enter/Space/arrows/Home/End, Escape/outside/Back/focus, pt-BR/en persistence and budget copy, authored-content preservation, HEX palette, cancellation/one confirmed request and memory sheet. Screenshots: ${out}`);
  if (process.env.MC_UI_INTERFACE_ONLY === '1') {
    await fs.writeFile(path.join(out, 'ui-results.json'), JSON.stringify(interfaceResults, null, 2));
  } else {
  const compact=await evaluate(`({header:document.querySelector('.mc-chat > header').getBoundingClientRect().height,composer:document.querySelector('.mc-composer-input').closest('.mc-glass-pill').getBoundingClientRect().height,capsule:document.querySelector('.mc-identity-pill').getBoundingClientRect().width})`);
  assert.ok(compact.header<=54&&compact.composer<=48&&compact.capsule<230,JSON.stringify(compact));
  assert.equal(await evaluate(`!!document.querySelector('[aria-label="Modelo conectado"].bg-emerald-400')`),true);

  // Real menu -> editor switch, literal copy and recovery from a save error.
  const menu = async id => {
    await evaluate(`(()=>{const p=document.querySelector('[data-message-id="${id}"] p');p.dispatchEvent(new TouchEvent('touchstart',{bubbles:true,touches:[]}));p.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true}));p.dispatchEvent(new TouchEvent('touchend',{bubbles:true,touches:[]}));})()`);
    await waitFor(`document.querySelector('.mc-sheet')`);
  };
  const enter = (selector,text) => evaluate(`(()=>{const t=document.querySelector(${JSON.stringify(selector)});Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(t,${JSON.stringify(text)});t.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  // A rejected request must render the error without unmounting the chat (send AND regenerate).
  // This exercises isNative() in ChatView's error branch, which used to throw ReferenceError.
  const preservedHistory = JSON.stringify(chat.history);
  for (const mode of ['regenerate', 'send']) {
    failMessage='rejected';
    if(mode==='send') {await enter('textarea','RASCUNHO_ERRO_PRESERVADO');await click('Enviar');}
    else await click('Refazer a última resposta');
    await waitFor(`document.querySelector('.mc-scroll')?.textContent.includes('FALHA_PROVEDOR_TESTE')`);
    assert.equal(calls.at(-1).mode,mode);
    assert.equal(await evaluate(`!!document.querySelector('.mc-chat')&&!!document.querySelector('textarea')`),true,`${mode}: error preserves the screen`);
    assert.equal(JSON.stringify(chat.history),preservedHistory,`${mode}: rejected request preserves conversation`);
    if(mode==='send')assert.equal(await evaluate(`document.querySelector('textarea').value`),'RASCUNHO_ERRO_PRESERVADO');
    assert.deepEqual(errors,[],`${mode}: no render exception`);
    await textButton('fechar');
    failMessage=false;
  }
  chat.history[1].content='Mira: **Fala com markdown**\n*Uma ação.*';chat.revision++;await refresh();
  await menu('a1');await textButton('Copiar');await waitFor(`window.__copied`);
  assert.equal(await evaluate('window.__copied'),chat.history[1].content);
  await waitFor(`!document.querySelector('.mc-sheet')`);
  await menu('a1');await textButton('Editar');await waitFor(`document.querySelector('.mc-editor')`);await pause(180);
  assert.equal(await evaluate(`!!document.querySelector('.mc-editor')`),true,'editor survives old menu history traversal');
  await enter('[aria-label="Texto da mensagem"]','Mira: **Correção salva**\n*Ela abre a carta.*');
  failEdit=true;await textButton('Salvar');await waitFor(`document.querySelector('.mc-editor [role="alert"]')`);
  assert.ok(await evaluate(`document.querySelector('[aria-label="Texto da mensagem"]').value.includes('Correção salva')`),'failed edit preserves draft');
  failEdit=false;await textButton('Salvar');await waitFor(`!document.querySelector('.mc-editor')`);
  assert.ok(chat.history[1].content.includes('**Correção salva**'));
  await menu('u1');await textButton('Editar e reenviar (novo ramo)');await waitFor(`document.querySelector('.mc-editor')`);
  await cdp('Emulation.setDeviceMetricsOverride',{width:360,height:420,deviceScaleFactor:1,mobile:true});
  const editBox=await evaluate(`(()=>{const t=document.querySelector('.mc-edit-text').getBoundingClientRect(),b=Array.from(document.querySelector('.mc-editor').querySelectorAll('button')).at(-1).getBoundingClientRect();return {left:t.left,right:t.right,bottom:t.bottom,saveBottom:b.bottom,w:innerWidth,h:innerHeight}})()`);
  assert.ok(editBox.left>=0&&editBox.right<=editBox.w&&editBox.bottom<=editBox.h&&editBox.saveBottom<editBox.h);
  await screenshot('edit-keyboard-360');
  await enter('[aria-label="Texto da mensagem"]','*Entro na biblioteca corrigida.* **O que encontrou?**');
  await textButton('Reenviar');await waitFor(`!document.querySelector('.mc-editor')&&document.querySelector('[aria-label="Parar geração"]')`);
  assert.equal(calls.at(-1).mode,'edit');assert.equal(calls.at(-1).messageId,'i1');complete();
  await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  chat=initial();await refresh();
  await cdp('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  const swipe=async(x,y,dx,dy)=>{
    await cdp('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x,y}]});
    for(let i=1;i<=4;i++)await cdp('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:x+dx*i/4,y:y+dy*i/4}]});
    await cdp('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  };
  // Preserve the authored 36px activation handle: 37px away is outside, 35px is inside.
  await swipe(353,300,-110,8);await pause(250);
  assert.equal(await evaluate(`!!document.querySelector('.mc-drawer')`),false,'drawer opens only from the right edge strip');
  await swipe(355,300,-220,8);await waitFor(`document.querySelector('.mc-drawer')`);await pause(800);
  const floating=await evaluate(`(()=>{const p=document.querySelector('.mc-drawer-panel'),r=p.getBoundingClientRect();return {right:innerWidth-r.right,top:r.top,bottom:innerHeight-r.bottom,radius:parseFloat(getComputedStyle(p).borderTopLeftRadius)}})()`);
  assert.ok(floating.right>=8&&floating.top>=8&&floating.bottom>=8&&floating.radius>=30,'drawer floats away from the edges: '+JSON.stringify(floating));
  await screenshot('drawer-glass-390');
  // The panel follows the finger both ways (you can go back and forth without releasing) and the spring decides on release.
  const panelX=()=>evaluate(`new DOMMatrix(getComputedStyle(document.querySelector('.mc-drawer-panel')).transform).m41`);
  await cdp('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:150,y:400}]});
  for(const x of [200,260])await cdp('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x,y:402}]});await pause(60);
  const dragRight=await panelX();
  await cdp('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:170,y:402}]});await pause(60);
  const dragBack=await panelX();
  await cdp('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});await pause(900);
  assert.ok(dragRight>80&&dragBack<dragRight-50,`panel follows the finger both ways (${dragRight} -> ${dragBack})`);
  assert.ok(Math.abs(await panelX())<1&&await evaluate(`!!document.querySelector('.mc-drawer')`),'released near the open side: the drawer springs back open');
  await swipe(120,300,240,8);await waitFor(`!document.querySelector('.mc-drawer')`);
  await click('Opções');await waitFor(`document.querySelector('.mc-drawer')`);
  await waitFor(`document.querySelector('[aria-label="Pensamento do modelo"]')`);
  await click('Pensamento do modelo');
  await waitFor(`document.querySelector('[aria-label="Pensamento do modelo"]').getAttribute('aria-checked')==='false'`);
  assert.equal(chat.settings.thinkingEnabled,false);assert.equal(chat.effort,'medium','turning thinking off preserves effort');
  await screenshot('thinking-switch-off');
  await click('Pensamento do modelo');await waitFor(`document.querySelector('[aria-label="Pensamento do modelo"]').getAttribute('aria-checked')==='true'`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Modelo e esforço')).click()`);
  await waitFor(`document.querySelector('[aria-label="Pensamento do modelo selecionado"]')`);
  await click('Pensamento do modelo selecionado');
  assert.equal(await evaluate(`document.querySelector('[aria-label="Pensamento do modelo selecionado"]').getAttribute('aria-checked')`),'false');
  await screenshot('thinking-model-choice');
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Usar Fixture')).click()`);
  await waitFor(`document.querySelector('[aria-label="Pensamento do modelo"]')`);
  assert.equal(chat.settings.thinkingEnabled,false,'model-page selection is saved');
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Estilo e limites')).click()`);
  await waitFor(`document.querySelector('[aria-label="Proporção entre narração e falas dos personagens"]')`);
  assert.equal(await evaluate(`document.querySelector('[aria-label="Como usar o tamanho da resposta"] [aria-pressed="true"]').textContent`),'Até este tamanho');
  assert.equal(await evaluate(`document.querySelector('[aria-label="Limite de falas seguidas por personagem"]').value`),'300');
  for (const width of [320, 390]) {
    await cdp('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:true});
    await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
    assert.equal(await evaluate('document.documentElement.scrollWidth>innerWidth'),false,'style controls fit mobile width');
    await screenshot(`response-controls-${width}`);
  }
  await evaluate(`document.querySelector('[aria-label="Como usar o tamanho da resposta"]').querySelectorAll('button')[1].click()`);
  await evaluate(`document.querySelector('[aria-label="Avanço da cena por resposta"]').querySelectorAll('button')[1].click()`);
  await evaluate(`(()=>{const t=document.querySelector('[aria-label="Limite de falas seguidas por personagem"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(t,'150');t.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await evaluate(`(()=>{const t=document.querySelector('[aria-label="Proporção entre narração e falas dos personagens"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(t,'90');t.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await click('Voltar');await pause(50);await swipe(120,300,240,8);await waitFor(`!document.querySelector('.mc-drawer')`);
  await waitFor(`!document.body.textContent.includes('Salvando')`);
  assert.equal(globalStyle.dialoguePct,90,'closing settings immediately flushes the slider');
  assert.equal(globalStyle.lengthMode,'target');assert.equal(globalStyle.turnPacing,'balanced');assert.equal(globalStyle.monologueChars,150,'new controls flush before returning to the chat');
  chat.history[2].content+='\n'+('Leitura longa. '.repeat(150));chat.revision++;await refresh();
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=200;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  const verticalStart=await evaluate(`document.querySelector('.mc-scroll').scrollTop`);
  await swipe(130,390,80,-50);await pause(60);
  assert.ok(await evaluate(`document.querySelector('.mc-scroll').scrollTop`)>verticalStart+30,'diagonal upward swipe scrolls down');
  assert.equal(await evaluate(`!!document.querySelector('.mc-drawer')`),false,'vertical semicircle does not open drawer');
  await swipe(130,390,80,50);await pause(60);
  await cdp('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:130,y:390}]});
  await cdp('Input.dispatchTouchEvent',{type:'touchCancel',touchPoints:[]});
  chat=initial();await refresh();
  assert.equal(await evaluate(`document.querySelectorAll('.mc-chat-actions').length`), 1, 'only last reply has actions');
  assert.equal(await evaluate(`document.querySelector('.mc-chat-actions').parentElement.classList.contains('bg-cardDark')`), true, 'footer inside card');
  assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('.mc-chat-action')).map(b => ({text:b.textContent,aria:!!b.getAttribute('aria-label'),title:!!b.title,w:b.getBoundingClientRect().width,h:b.getBoundingClientRect().height}))`), [{ text: '', aria: true, title: true, w: 44, h: 44 }, { text: '', aria: true, title: true, w: 44, h: 44 }]);
  await noActionFlash();
  for (const width of [320, 360, 390, 412]) {
    await cdp('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: true });
    await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
    for (const font of [14, 22]) {
      await evaluate(`document.documentElement.style.setProperty('--mc-font','${font}px')`);
      const box = await evaluate(`(() => { const a=document.querySelector('.mc-assistant').getBoundingClientRect(); return {left:a.left,right:innerWidth-a.right,width:a.width,font:getComputedStyle(document.querySelector('.mc-read')).fontSize,overflow:document.documentElement.scrollWidth>innerWidth || document.querySelector('.mc-read').scrollWidth>document.querySelector('.mc-read').clientWidth}; })()`);
      assert.equal(box.overflow, false, `no overflow ${width}/${font}`); assert.equal(box.font, `${font}px`);
      // Geometry: smaller left margin (~8%) and more room on the right (~14%).
      assert.ok(box.left / width > .065 && box.left / width < .09, 'left margin ~8% '+JSON.stringify({width,...box})); assert.ok(box.right / width > .13 && box.right / width < .16, 'right margin ~14% '+JSON.stringify({width,...box}));
      assert.ok(box.right > box.left, 'larger space is on the right '+JSON.stringify({width,...box}));
      widths.push({ viewport: width, font, ...box });
    }
    await screenshot(`saved-${width}-font22`);
  }
  await cdp('Input.dispatchKeyEvent', {type:'keyDown',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});
  await cdp('Input.dispatchKeyEvent', {type:'keyUp',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});
  await evaluate(`document.querySelector('[aria-label="Continuar a cena"]').focus()`);
  assert.equal(await evaluate(`getComputedStyle(document.activeElement).outlineStyle`), 'solid', 'visible keyboard focus');
  await evaluate(`document.querySelector('.mc-chat-action').dispatchEvent(new Event('touchstart', {bubbles:true}))`); await pause(560);
  await noActionFlash();
  assert.equal(await evaluate(`!!document.querySelector('.mc-sheet')`), false, 'long press creates no overlay while held');
  await evaluate(`document.querySelector('.mc-chat-action').dispatchEvent(new Event('touchend', {bubbles:true}))`);
  assert.equal(await evaluate(`!!document.querySelector('.mc-sheet')`), false, 'long press action cannot open message menu');
  // rects are visual pixels; scrollTop and elasticY are layout units, even during opening scale.
  const anchor=()=>evaluate(`(() => {const el=document.querySelector('.mc-scroll'),rect=el.getBoundingClientRect(),scaleY=el.offsetHeight?rect.height/el.offsetHeight||1:1,ey=Number(el.dataset.elasticY)||0,a=[...el.querySelectorAll('[data-message-id]')].find(a=>(a.getBoundingClientRect().bottom-rect.top)/scaleY-ey>0);return {id:a?.dataset.messageId,offset:a?(a.getBoundingClientRect().top-rect.top)/scaleY-ey:null,visualOffset:a?a.getBoundingClientRect().top-rect.top:null,scaleY,top:el.scrollTop,height:el.scrollHeight,client:el.clientHeight,gap:el.scrollHeight-el.clientHeight-el.scrollTop,elastic:ey};})()`);
  const sameReading=(before,after,label)=>{assert.equal(after.id,before.id,label+' anchor preserved '+JSON.stringify({before,after}));assert.ok(Math.abs(after.offset-before.offset)<2,label+' layout offset preserved '+JSON.stringify({before,after}));};
  const stable=async before=>{await pause(100);const after=await anchor();readingChecks.push({mode:'reading',before,after});sameReading(before,after,'reading');return after;};
  const atEnd=async label=>{
    await waitFor(`Math.abs(document.querySelector('.mc-scroll').scrollHeight-document.querySelector('.mc-scroll').clientHeight-document.querySelector('.mc-scroll').scrollTop)<=1`);
    const position=await anchor();readingChecks.push({mode:'following',label,position});
    assert.ok(Math.abs(position.gap)<=1,'follows the end: '+label+' '+JSON.stringify(position));
  };
  const noDelta=async()=>{assert.equal(await evaluate(`document.body.textContent.includes('RESPOSTA_PRONTA_')`),false,'no buffered delta text before done');assert.equal(await evaluate(`document.querySelectorAll('.streaming-cursor').length`),0);};
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=0;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);const beforeContinue=await anchor();
  assert.ok(beforeContinue.height-beforeContinue.client>0&&beforeContinue.height-beforeContinue.client<140,'fixture is scrollable even though the entire range is inside the former 140px threshold: '+JSON.stringify(beforeContinue));
  await click('Continuar a cena'); await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await noDelta();await stable(beforeContinue);
  await noActionFlash();
  assert.equal(calls.at(-1).mode, 'continue');
  assert.equal(await evaluate(`document.querySelectorAll('[aria-label="Continuar a cena"]').length`), 0, 'saved actions hidden while streaming');
  const streamWidth = await evaluate(`Array.from(document.querySelectorAll('.mc-assistant')).at(-1).getBoundingClientRect().width`);
  assert.ok(Math.abs(streamWidth - widths.at(-1).width) < 1, 'stream uses saved reply width');
  const waiting=await evaluate(`(() => {const el=document.querySelector('.mc-generation'),label=el.querySelector('[role="status"]'),stop=el.querySelector('button');return {height:el.getBoundingClientRect().height,width:el.getBoundingClientRect().width,label:label.getAttribute('aria-label'),left:label.getBoundingClientRect().left,right:stop.getBoundingClientRect().left,dots:el.querySelectorAll('.mc-thinking-dots > span').length,lit:el.querySelectorAll('.mc-thinking-dots > span.on').length,cssAnimations:el.getAnimations({subtree:true}).filter(a=>a instanceof CSSAnimation||a.effect?.getTiming().iterations===Infinity).length}})()`);
  assert.ok(waiting.height<=44&&waiting.width<=180,'compact waiting indicator, not a message-size card');
  assert.equal(waiting.label,'Pensando...');assert.equal(waiting.dots,3);assert.equal(waiting.cssAnimations,0,'waiting indicator holds no continuous CSS animation');
  const litSeen=new Set([waiting.lit]);for(let n=0;n<30&&litSeen.size<2;n++){await pause(100);litSeen.add(await evaluate(`document.querySelectorAll('.mc-generation .mc-thinking-dots > span.on').length`));}
  assert.ok(litSeen.size>1,'thinking dots advance in steps');
  assert.ok(waiting.left<waiting.right,'thinking text left, stop icon right');
  await cdp('Emulation.setEmulatedMedia', {features:[{name:'prefers-reduced-motion',value:'reduce'}]});
  await pause(900);
  assert.equal(await evaluate(`document.querySelectorAll('.mc-thinking-dots > span.on').length`),3,'respects reduced-motion preference: dots hold still');
  assert.equal(await evaluate(`document.querySelector('.mc-generation').getAnimations({subtree:true}).filter(a=>a instanceof CSSAnimation||a.effect?.getTiming().iterations===Infinity).length`),0,'no CSS animation under reduced motion');
  await cdp('Emulation.setEmulatedMedia', {features:[{name:'prefers-reduced-motion',value:'no-preference'}]});
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=document.querySelector('.mc-scroll').scrollHeight`);
  await screenshot('generation-compact-412');
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=${beforeContinue.top}`);
  await screenshot('buffer-before-done-412'); await click('Parar geração'); await waitFor(`document.querySelector('[aria-label="Refazer a última resposta"]')`);await stable(beforeContinue);
  assert.ok(calls.some(c => c.stop));
  const savedText=chat.history.at(-1).content;const oldCount=await evaluate(`document.querySelectorAll('.mc-assistant').length`);
  await click('Refazer a última resposta'); await waitFor(`document.querySelector('[aria-label="Parar geração"]')`); assert.equal(calls.at(-1).mode, 'regenerate');
  assert.equal(await evaluate(`document.querySelectorAll('.mc-assistant').length`),oldCount,'old target hidden and replaced by waiting card');
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-message-id]')).some(m=>m.textContent.includes('RESPOSTA_PRONTA_'))`),false,'old answer disappears during regeneration');
  await screenshot('regeneration-refined-412');
  await stable(beforeContinue);complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);await stable(beforeContinue);assert.equal(chat.history.at(-1).content,savedText);
  chat=initial();chat.history[2].content+='\n'+('Leitura longa para manter a posição. '.repeat(180));await refresh();
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=120;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);const beforeSend=await anchor();
  assert.ok(beforeSend.gap>140,'long reading fixture is away from the end');
  await evaluate(`(() => {const t=document.querySelector('textarea');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(t,${JSON.stringify('Mensagem de envio.\n'+('compositor grande '.repeat(30)))});t.dispatchEvent(new Event('input',{bubbles:true}));})()`);await stable(beforeSend);
  await click('Enviar');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await waitFor(`document.querySelector('textarea').value===''`);await noDelta();await stable(beforeSend);
  for(let n=0;n<60&&!calls.some(c=>c.reconnect===15);n++)await pause(50);assert.ok(calls.some(c=>c.reconnect===15),'reconnection uses UTF-16 offset after emoji');
  complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);await stable(beforeSend);
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-message-id]')).at(-1).textContent.includes('RESPOSTA_PRONTA_')`),true,'done presents complete reply in one burst');
  // Frozen socket: the native event resumes without duplicating the generation.
  const beforeResume=await anchor();
  await click('Continuar a cena');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);
  await stable(beforeResume);
  const operations=calls.filter(c=>c.mode).length,reconnections=calls.filter(c=>c.reconnect!=null).length;
  await evaluate(`window.dispatchEvent(new Event('mc:resume'))`);
  await waitFor(`!document.body.textContent.includes('Sem conexão com o servidor ativo')`);
  for(let n=0;n<40&&calls.filter(c=>c.reconnect!=null).length===reconnections;n++)await pause(50);
  assert.ok(calls.filter(c=>c.reconnect!=null).length>reconnections);
  assert.equal(calls.filter(c=>c.mode).length,operations,'resume attaches instead of regenerating');
  complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  await stable(beforeResume);
  // WebView recreated after the reply finished while the process was not running.
  await click('Refazer a última resposta');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);
  const reloadOperations=calls.filter(c=>c.mode).length;
  await cdp('Page.navigate',{url:'about:blank'});complete();await cdp('Page.navigate',{url:base});
  await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Cena fictícia'))`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Cena fictícia')).click()`);
  await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  assert.equal(calls.filter(c=>c.mode).length,reloadOperations,'completed generation recovered without new model operation');
  assert.equal(await evaluate(`localStorage.getItem('mc.pending.fixture')`),null);
  await atEnd('reopened chat');
  // Reopening intentionally starts at the end. Establish a reading position before testing a remote append.
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=120;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  const resumedAnchor=await anchor();
  assert.ok(resumedAnchor.gap>140,'remote append starts while reading above the end');
  chat.revision++;chat.history.push({id:'remote',role:'assistant',content:'Atualização remota '+('história '.repeat(90)),status:'complete'});
  await evaluate(`window.dispatchEvent(new CustomEvent('mc:event',{detail:{convId:'fixture',revision:${chat.revision},type:'message'}}))`);await pause(500);await stable(resumedAnchor);
  // A disconnect/reload adds and removes the notice above the messages without moving the reader.
  failChat=true;await refresh();await waitFor(`document.querySelector('.mc-scroll').textContent.includes('Sem conexão com o servidor ativo')`);await stable(resumedAnchor);
  failChat=false;await refresh();await waitFor(`!document.querySelector('.mc-scroll').textContent.includes('Sem conexão com o servidor ativo')`);await stable(resumedAnchor);
  await cdp('Emulation.setDeviceMetricsOverride',{width:412,height:600,deviceScaleFactor:1,mobile:true});await stable(resumedAnchor);
  await cdp('Emulation.setDeviceMetricsOverride',{width:412,height:844,deviceScaleFactor:1,mobile:true});await stable(resumedAnchor);
  // No overflow means there is no reading position above the end. Growth must keep following.
  chat={...initial(),history:[{id:'short-u',role:'user',content:'CURTA_USER',interactionId:'short-i'},{id:'short-a',role:'assistant',content:'CURTA_BOT',status:'complete'}]};
  await refresh();await waitFor(`document.body.textContent.includes('CURTA_BOT')`);await click('Ir para o fim');
  const short=await anchor();assert.equal(short.height,short.client,'short fixture has no overflow');
  await evaluate(`document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  await click('Continuar a cena');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await atEnd('short continue waiting');
  complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);await atEnd('short continue complete');
  assert.equal((await anchor()).height,(await anchor()).client,'completed short fixture still has no overflow');
  chat.history.push({id:'short-grown',role:'assistant',content:'CRESCEU_CURTA '+('História para crescer depois da conversa curta. '.repeat(120)),status:'complete'},{id:'short-tail',role:'assistant',content:'Fim da cena curta.',status:'complete'});chat.revision++;
  await evaluate(`window.dispatchEvent(new CustomEvent('mc:event',{detail:{convId:'fixture',revision:${chat.revision},type:'message'}}))`);await waitFor(`document.body.textContent.includes('CRESCEU_CURTA')`);await atEnd('short becomes scrollable on remote append');
  assert.ok((await anchor()).top>140,'following fixture now has substantial overflow');
  for(const label of ['Continuar a cena','Refazer a última resposta']){
    await click(label);await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await atEnd(label+' waiting');
    complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);await atEnd(label+' complete');
  }
  await enter('textarea','Envio seguindo o fim');await click('Enviar');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await atEnd('send waiting');
  complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);await atEnd('send complete');
  chat.history = [{ id: 'u1', role: 'user', content: 'Mensagem sem resposta após falha', interactionId: 'i1' }]; await refresh();
  await waitFor(`document.querySelector('[aria-label="Tentar responder de novo"]')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mc-chat-action').length`), 1);
  await screenshot('user-tail-retry'); await noActionFlash(); await click('Tentar responder de novo'); await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await noDelta(); assert.equal(calls.at(-1).mode, 'regenerate');
  await click('Parar geração'); await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  failChat = true; await refresh(); await waitFor(`document.body.textContent.includes('Sem conexão com o servidor ativo')`);
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('.mc-chat-action')).every(b=>b.disabled)`), true);
  assert.equal(await evaluate(`!!document.querySelector('[aria-label="Modelo sem conexão confirmada"].bg-gray-500')`),true);
  await evaluate(`(() => { const t=document.querySelector('textarea'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(t,'Rascunho offline'); t.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  assert.equal(await evaluate(`document.querySelector('[aria-label="Enviar"]').disabled`), true); await screenshot('offline-draft');
  failChat = false; await refresh(); await waitFor(`!document.body.textContent.includes('Sem conexão com o servidor ativo')`);
  await waitFor(`document.querySelector('[aria-label="Modelo conectado"].bg-emerald-400')`);
  await cdp('Emulation.setDeviceMetricsOverride', { width: 360, height: 420, deviceScaleFactor: 1, mobile: true });
  await evaluate(`document.documentElement.style.setProperty('--safe-area-inset-top','24px');document.documentElement.style.setProperty('--safe-area-inset-bottom','20px');document.querySelector('textarea').focus()`);
  const composer = await evaluate(`(() => {const t=document.querySelector('textarea').getBoundingClientRect();return {bottom:t.bottom,height:innerHeight,overflow:document.documentElement.scrollWidth>innerWidth};})()`);
  assert.ok(composer.bottom < composer.height - 20); assert.equal(composer.overflow, false); await screenshot('keyboard-height-safe-area');
  await evaluate(`document.documentElement.style.setProperty('--safe-area-inset-top','0px');document.documentElement.style.setProperty('--safe-area-inset-bottom','0px')`);
  // Real cached-first opening, with a held GET and a stale public/generic preset cache.
  chat=initial();chat.history[2].content+='\n'+('Trecho de leitura no cache. '.repeat(220));await refresh();await pause(200);
  const stalePresets={presets:[
    {id:'generic',selectable:false,label:'LEGADO_GENERICO',efforts:['high'],routes:[{id:'hidden'}]},
    {id:'unknown',label:'LEGADO_OUTRO',efforts:['high'],routes:[{id:'hidden2'}]},
    {id:'deepseek-v41-flash',label:'PRESET_PERMITIDO_OFFLINE',efforts:['high'],defaultEffort:'high',routes:[{id:'cbai/deepseek-v4.1-flash'}]},
    {id:'future-family',selectable:true,label:'MODELO_MODULAR_OFFLINE',efforts:[],routes:[{id:'oc/deepseek-v4-flash'}]},
    {id:'gemini-38-flash',label:'GEMINI_CACHE',efforts:['low','medium','high'],defaultEffort:'low',routes:[{id:'ag/gemini-3.8-flash-low'},{id:'ag/gemini-3.8-flash-medium'},{id:'ag/gemini-3.8-flash-high'}]},
  ]};
  await evaluate(`new Promise((resolve,reject)=>{const req=indexedDB.open('modularchat',1);req.onsuccess=()=>{const db=req.result,tx=db.transaction('kv','readwrite');tx.objectStore('kv').put(${JSON.stringify(stalePresets)},'presets');tx.oncomplete=()=>{db.close();resolve()};tx.onerror=reject};req.onerror=reject})`);
  let releaseChat;delayChat=new Promise(r=>releaseChat=r);failPresets=true;
  await cdp('Page.navigate',{url:base});
  await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Cena fictícia'))`);
  // Pause the actual opening animation, not a synthetic transform or a wait for completion.
  await evaluate(`(()=>{const original=HTMLElement.prototype.animate;HTMLElement.prototype.animate=function(frames,options){const animation=Reflect.apply(original,this,arguments);if(this.matches('.mc-chat')&&frames.some(f=>f.transform)){animation.pause();animation.currentTime=options.duration*.1;window.__cachedOpening=animation;HTMLElement.prototype.animate=original;}return animation;};})()`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Cena fictícia')).click()`);
  await waitFor(`document.querySelector('.mc-scroll [data-message-id="a2"]')`);
  assert.equal(await evaluate(`window.__cachedOpening?.playState`),'paused','cached-first is tested before opening finishes');
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=140;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  const cachedAnchor=await anchor();
  assert.ok(cachedAnchor.id&&cachedAnchor.scaleY>0&&cachedAnchor.scaleY<.8,'reading captured under nontrivial scale');
  scaleChecks.push({phase:'before-get',position:cachedAnchor});
  // An actual scroll must still change the layout offset; normalization cannot hide it.
  await evaluate(`document.querySelector('.mc-scroll').scrollTop+=32;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  const scrolledAnchor=await anchor();
  assert.equal(scrolledAnchor.id,cachedAnchor.id,'scroll probe stays in the same message');
  assert.ok(Math.abs(scrolledAnchor.offset-cachedAnchor.offset+32)<2,'helper detects a real 32px layout scroll');
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=140;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'));window.__cachedOpening.currentTime=window.__cachedOpening.effect.getTiming().duration*.3`);
  const duringScale=await anchor();sameReading(cachedAnchor,duringScale,'scale before GET');
  assert.ok(Math.abs(duringScale.scaleY-cachedAnchor.scaleY)>.1,'scale changes between remember and restore');
  scaleChecks.push({phase:'during-scale-before-get',position:duringScale});
  // Exercise layout-unit elasticY together with scale and the incoming message insertion.
  await evaluate(`(()=>{const el=document.querySelector('.mc-scroll'),list=el.querySelector('[data-message-id]').parentElement;window.__elasticProbe={list,transform:list.style.transform};list.style.transform='translate3d(0,24px,0)';el.dataset.elasticY='24';el.dispatchEvent(new Event('scroll'));})()`);
  const elasticAnchor=await anchor();sameReading(cachedAnchor,elasticAnchor,'scale plus elastic');
  scaleChecks.push({phase:'during-scale-elastic',position:elasticAnchor});
  chat.history.unshift({id:'inserted-cache',role:'assistant',content:'NOVO_GET '+('Texto novo antes da âncora. '.repeat(16)),status:'complete'});chat.revision++;
  delayChat=null;releaseChat();await waitFor(`document.body.textContent.includes('NOVO_GET')`);
  const mergedAnchor=await stable(cachedAnchor);
  assert.equal(await evaluate(`window.__cachedOpening.playState`),'paused','GET restores reading while scale is still in progress');
  assert.ok(mergedAnchor.top>cachedAnchor.top+100,'inserted message causes real scroll compensation');
  scaleChecks.push({phase:'during-scale-after-get',position:mergedAnchor});
  await evaluate(`window.__elasticProbe.list.style.transform=window.__elasticProbe.transform;document.querySelector('.mc-scroll').dataset.elasticY='';window.__cachedOpening.currentTime=window.__cachedOpening.effect.getTiming().duration*.65`);
  const laterScale=await anchor();sameReading(cachedAnchor,laterScale,'later scale');
  scaleChecks.push({phase:'later-scale',position:laterScale});
  await evaluate(`window.__cachedOpening.play();window.__cachedOpening.finished`);
  const afterScale=await stable(cachedAnchor);assert.ok(Math.abs(afterScale.scaleY-1)<.001,'opening scale completed');
  scaleChecks.push({phase:'after-scale',position:afterScale});
  await click('Opções');await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Modelo e esforço'))`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Modelo e esforço')).click()`);
  await waitFor(`document.body.textContent.includes('PRESET_PERMITIDO_OFFLINE')`);
  assert.equal(await evaluate(`document.body.textContent.includes('LEGADO_GENERICO')||document.body.textContent.includes('LEGADO_OUTRO')`),false,'offline cache hides generic and unsupported presets');
  assert.equal(await evaluate(`document.body.textContent.includes('MODELO_MODULAR_OFFLINE')`),true,'new server presets survive cache and render in model selection');
  await screenshot('cached-get-anchor-offline-presets');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false }); await screenshot('desktop');
  // Separate accepted SSE without an operation ACK from Chromium's pre-header HTTP retries.
  await evaluate(`document.querySelector('.mc-drawer .mc-scrim').click()`);await waitFor(`!document.querySelector('.mc-drawer')`);
  await cdp('Emulation.setDeviceMetricsOverride',{width:412,height:844,deviceScaleFactor:1,mobile:true});
  await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
  const fetchCalls=()=>evaluate(`window.__fetchCalls`);
  for(const fault of ['accepted-sse','pre-headers']) {
    if(fault==='pre-headers') {
      // Reset only the disposable fixture's origin; retain the first scenario's evidence.
      failMessage=false;failPresets=false;
      await cdp('Page.navigate',{url:'about:blank'});
      await cdp('Storage.clearDataForOrigin',{origin:base,storageTypes:'all'});
      await cdp('Page.navigate',{url:base});
      await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Cena fictícia'))`);
      await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Cena fictícia')).click()`);
      await waitFor(`document.querySelector('.mc-scroll [data-message-id="a2"]')`);
    }
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=140;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  const beforePendingError=await anchor(),beforeFailedSend=calls.filter(c=>c.mode).length,beforeFetch=(await fetchCalls()).length;
  const proof={fault,status:'running'};transportChecks.push(proof);
  failMessage=fault;await enter('textarea','RASCUNHO_PENDING_FALHA');await click('Enviar');
  if(fault==='accepted-sse') {
    await waitFor(`window.__fetchCalls.slice(${beforeFetch}).some(r=>r.path.endsWith('/message')&&r.status===200)`);
    assert.equal(await evaluate(`!!document.querySelector('[aria-label="Parar geração"]')`),true,'accepted SSE is still awaiting operation ACK');
    assert.equal(await evaluate(`document.querySelector('textarea').value`),'RASCUNHO_PENDING_FALHA','draft retained before ACK while SSE is open');
  } else await waitFor(`!document.querySelector('[aria-label="Parar geração"]')&&document.querySelector('.mc-scroll').textContent.includes('Sem conexão com o servidor ativo')`);
  let savedPending;
  if(!privateFixture){
    savedPending=await evaluate(`JSON.parse(localStorage.getItem('mc.pending.fixture'))`);
    assert.ok(savedPending?.opId,'pending is durable before ACK');
    assert.equal(savedPending.originServer,'');assert.equal(savedPending.originKind,'browser');
    assert.equal(savedPending.generationId,undefined,'no operation ACK was recorded');
    proof.pending=savedPending;
  }
  if(fault==='accepted-sse') {assert.equal(typeof destroyAccepted,'function');destroyAccepted();destroyAccepted=null;}
  await waitFor(`!document.querySelector('[aria-label="Parar geração"]')&&document.querySelector('.mc-scroll').textContent.includes('Sem conexão com o servidor ativo')`);
  assert.equal(await evaluate(`document.querySelector('textarea').value`),'RASCUNHO_PENDING_FALHA','draft survives missing ack');
  const appWrites=(await fetchCalls()).slice(beforeFetch).filter(c=>c.method==='POST'&&c.path.endsWith('/message'));
  const physicalWrites=calls.filter(c=>c.mode).slice(beforeFailedSend);
  Object.assign(proof,{applicationFetches:appWrites,physicalPosts:physicalWrites.length,physicalOpIds:physicalWrites.map(c=>c.opId)});
  assert.equal(appWrites.length,1,'uncertain write dispatches exactly one application fetch');
  assert.ok(appWrites[0].opId,'application sends an operation id');
  assert.ok(physicalWrites.length>=1,'mock received the POST');
  assert.ok(physicalWrites.every(c=>c.opId===appWrites[0].opId),'every physical duplicate retains the same operation id');
  if(fault==='accepted-sse') {
    assert.equal(physicalWrites.length,1,'accepted SSE receives exactly one physical POST');
    assert.equal(appWrites[0].status,200);assert.equal(appWrites[0].contentType,'text/event-stream');
  } else {assert.equal(appWrites[0].status,undefined,'pre-header failure never produces a response');assert.ok(appWrites[0].error,'pre-header fetch rejects');}
  if(!privateFixture){
    assert.equal(savedPending.opId,appWrites[0].opId);
    assert.deepEqual(await evaluate(`JSON.parse(localStorage.getItem('mc.pending.fixture'))`),savedPending,'failed receipt preserves the original pending operation');
  }
  await stable(beforePendingError);
  const beforeRecoveryFetch=(await fetchCalls()).length,beforeRecoveryHttp=httpRequests.length;
  await refresh();await waitFor(`document.querySelector('.mc-scroll').textContent.includes('O recebimento do envio ficou incerto')`);
  assert.equal(calls.filter(c=>c.mode).length,beforeFailedSend+physicalWrites.length,'pending recovery never starts a new generation');
  const recoveryFetches=(await fetchCalls()).slice(beforeRecoveryFetch),recoveryHttp=httpRequests.slice(beforeRecoveryHttp);
  assert.ok(recoveryFetches.some(c=>c.path==='/api/chats/fixture'),'recovery queries the original conversation');
  assert.ok(recoveryFetches.every(c=>c.method==='GET'),'application recovery uses GET only');
  assert.ok(recoveryHttp.length&&recoveryHttp.every(c=>c.method==='GET'),'physical recovery uses GET only');
  assert.equal((await fetchCalls()).slice(beforeFetch).filter(c=>c.method==='POST'&&c.path.endsWith('/message')).length,1,'no application replay after recovery');
  if(!privateFixture)assert.deepEqual(await evaluate(`JSON.parse(localStorage.getItem('mc.pending.fixture'))`),savedPending,'pending origin and operation remain intact');
  assert.equal(await evaluate(`document.querySelector('textarea').value`),'RASCUNHO_PENDING_FALHA','draft survives GET-only recovery');
  await stable(beforePendingError);await screenshot('pending-error-reading-anchor-'+fault);
  Object.assign(proof,{status:'passed',recoveryFetches,recoveryHttp});
  }
  if(privateFixture) {
    const protectedCache=await evaluate(`new Promise((resolve,reject)=>{const req=indexedDB.open('modularchat',1);req.onsuccess=()=>{const db=req.result,tx=db.transaction('kv','readonly'),r=tx.objectStore('kv').getAll();tx.oncomplete=()=>{db.close();resolve(r.result.map(v=>!!v?.sealed))};tx.onerror=reject};req.onerror=reject})`);
    assert.ok(protectedCache.length&&protectedCache.every(Boolean),'all cache values sealed');
    assert.equal(await evaluate(`!!localStorage.getItem('mc.private.v1')&& !localStorage.getItem('mc.private.v1').includes('Rascunho offline')`),true,'drafts sealed at rest');
    assert.equal(await evaluate(`Array.from({length:localStorage.length},(_,i)=>localStorage.key(i)).some(k=>k.startsWith('mc.draft.')||k.startsWith('mc.pending.'))`),false,'no plaintext draft/pending slots');
  }
  assert.deepEqual(errors, []);
  await fs.writeFile(path.join(out, 'ui-results.json'), JSON.stringify({ status: 'passed', assertionCount, widths, feedback, languageDropdown, readingChecks, scaleChecks, transportChecks, edgeStripPx:36, requests: calls.map(c=>c.mode || 'stop'), runtimeErrors: errors, limits: 'Viewport reduzido simula espaço do teclado. IME/teclado e WebView Android físicos não testados. HTTP físico observado somente no mock Edge; idempotência real do backend e corrida jobs.stop pertencem ao lote backend final.' }, null, 2));
  console.log(`PASS: edição user/bot e falha recuperável, cópia Markdown, slider salvo ao sair, gestos verticais diagonais e gaveta lateral, UI widths/font, icon aria/title/44px/focus, buffer invisible before done, regeneration target hidden, native resume and completed generation recovery without duplication, UTF-16 reconnect, scroll anchors, offline cached presets, long press, offline, safe areas, simulated keyboard height. Screenshots: ${out}`);
  }
} catch (error) {
  const failure = await inspectFailure?.().catch(() => null);
  await fs.writeFile(path.join(out, 'ui-results.json'), JSON.stringify({ status: 'failed', assertionCount, error: error.stack, languageDropdown, readingChecks, scaleChecks, transportChecks, runtimeErrors: errors, failure }, null, 2));
  throw error;
} finally {
  ws?.close(); for (const p of pending.values()) clearTimeout(p.timer);
  if (active) active.res.destroy(); server.closeAllConnections(); await new Promise(r=>server.close(r));
  edge.kill(); await Promise.race([new Promise(r=>edge.once('exit',r)), pause(3000)]);
  await fs.rm(profile, { recursive: true, force: true, maxRetries: 4, retryDelay: 200 }).catch(() => {});
}
