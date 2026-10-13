// Reproducible UI check: built React app, fictitious HTTP/SSE, disposable headless Edge profile.
// No personal data, provider, production server, or additional browser/UI dependency.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { randomBytes } from 'node:crypto';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.resolve(process.env.MC_UI_VERIFICATION_DIR || await fs.mkdtemp(path.join(os.tmpdir(), 'mc-ui-evidence-')));
await fs.mkdir(out, { recursive: true });
const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-ui-edge-'));
const privateFixture = process.env.MC_UI_PRIVACY === '1';
const privateCacheKey = randomBytes(32).toString('base64');
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
const initial = () => ({ id: 'fixture', name: 'Cena fictícia', persona: 'Alex', presetId: 'claude-sonnet-5-5', routeId: 'cc/claude-sonnet-5-5', effort:'medium', thinkingEnabled:true,settings:{thinkingEnabled:true}, revision: 1, characterRev: 1, memorySummary: {}, history: [
  { id: 'u1', role: 'user', content: '*Entro na biblioteca.* O que você encontrou?', interactionId: 'i1' },
  { id: 'a1', role: 'assistant', content: 'Mira: Encontrei uma carta.\n*Ela abre o envelope com cuidado.*', status: 'complete', versionCount: 1 },
  { id: 'a2', role: 'assistant', content: 'Mira: Podemos ler juntas. A biblioteca guarda histórias de várias épocas.\n*Uma luz violeta ilumina a mesa.* ' + 'palavra'.repeat(36), status: 'complete', versionCount: 1 },
] });
let chat = initial(), failChat = false, failPresets=false, delayChat=null, active = null, generation=0, failEdit=false;
let globalStyle={botChars:8000,dialoguePct:65,content:'',lengthMode:'maximum',monologueChars:300,turnPacing:'interactive'};
let lastDone=null;
const calls = [], errors = [], widths = [], feedback = [];
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
      res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
      const partId=body.mode==='regenerate'&&chat.history.at(-1)?.role==='assistant'?chat.history.at(-1).id:`a-new-${++generation}`;
      const userMessage=['send','edit'].includes(body.mode)?{id:`u-new-${generation}`,role:'user',content:body.content,interactionId:`i-new-${generation}`}:null;
      if(body.mode==='edit'){const i=chat.history.findIndex(m=>m.interactionId===body.messageId&&m.role==='user');chat.history=chat.history.slice(0,i);}
      if(userMessage)chat.history.push(userMessage);
      active = { res, mode: body.mode,partId };
      res.write(`data: ${JSON.stringify({ ack: { generationId: 'g1', mode: body.mode, partId, userMessage,revision: ++chat.revision } })}\n\n`);
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
      res.write(`data: ${JSON.stringify({ack:{generationId:'g1',mode:active.mode,partId:active.partId}})}\n\n`);
      const offset=Number(url.searchParams.get('from'));
      res.write(`data: ${JSON.stringify({delta:visibleReply.slice(offset),offset:visibleReply.length})}\n\n`);return;
    }
    if (url.pathname.endsWith('/g1/stop')) {
      calls.push({ stop: true });
      complete('interrupted');
      return json(res, { ok: true });
    }
    if (url.pathname.startsWith('/api/')) { res.writeHead(404); return res.end('{"error":"fixture route absent"}'); }
    const dist = process.env.MC_UI_DIST ? path.resolve(process.env.MC_UI_DIST) : path.join(root, 'client/dist');
    const file = path.resolve(dist, url.pathname === '/' ? 'index.html' : '.' + url.pathname);
    assert.ok(file.startsWith(dist + path.sep));
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
let ws, seq = 0; const pending = new Map();
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
  await cdp('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  await cdp('Page.navigate', { url: base });
  await waitFor(`Array.from(document.querySelectorAll('button')).some(b => b.textContent.includes('Cena fictícia'))`);
  await screenshot('list-refined-390');

  // Interface language, colors and confirmations use the app's own sheets.
  const textButton = text => evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(text)});if(!b)throw Error('Button missing: '+${JSON.stringify(text)});b.click()})()`);
  const inputValue = (label, value) => evaluate(`(()=>{const i=document.querySelector('input[aria-label=${JSON.stringify(label)}]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(i,${JSON.stringify(value)});i.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  const authoredBefore = JSON.stringify(chat);
  const callsBefore = calls.length;
  await click('Configurações'); await waitFor(`document.querySelector('[aria-label="Idioma da interface"]')`);
  assert.equal(await evaluate(`document.querySelectorAll('select,input[type=color]').length`), 0);
  await click('Idioma da interface'); await waitFor(`document.querySelector('[role="radiogroup"]')`);
  await pause(600); await screenshot('language-sheet-390');
  assert.equal(await evaluate(`(()=>{const b=document.querySelector('[role="radio"][aria-checked="true"]'),r=b.getBoundingClientRect();return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest('[role="radio"]')===b})()`), true, 'language choice must render and receive taps above neighboring cards');
  await evaluate(`document.querySelector('[role="radio"][lang="en"]').click()`);
  await waitFor(`document.documentElement.lang==='en' && document.querySelector('[aria-label="Interface language"]')`);
  assert.equal(await evaluate(`localStorage.getItem('mc.language')`), 'en');
  assert.equal(await evaluate(`document.querySelector('h2').textContent`), 'Settings');
  assert.equal(JSON.stringify(chat), authoredBefore);
  await screenshot('settings-english-390');
  await cdp('Page.reload'); await waitFor(`document.querySelector('[aria-label="Settings"]')`);
  assert.equal(await evaluate(`document.documentElement.lang`), 'en');
  assert.ok(await evaluate(`document.body.textContent.includes('Cena fictícia')`));
  await click('Settings'); await waitFor(`document.querySelector('[aria-label="Interface language"]')`);
  await click('Interface language'); await waitFor(`document.querySelector('[role="radio"][lang="pt-BR"]')`);
  await evaluate(`document.querySelector('[role="radio"][lang="pt-BR"]').click()`);
  await waitFor(`document.documentElement.lang==='pt-BR'`);
  await click('Escolher outra cor'); await waitFor(`document.querySelector('[aria-label="Código da cor"]')`);
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

  await evaluate(`Array.from(document.querySelectorAll('button')).find(b => b.textContent.includes('Cena fictícia')).click()`);
  await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  await screenshot('chat-refined-390');
  await click('Memória'); await waitFor(`document.querySelector('h2')?.textContent==='Memória do PC'`);
  await textButton('Memórias'); await waitFor(`document.querySelector('[aria-label="Adicionar memória"]')`);
  await click('Adicionar memória'); await waitFor(`document.querySelector('[aria-label="Tipo de memória"]')`);
  await click('Tipo de memória'); await waitFor(`document.querySelector('[role="radiogroup"]')`);
  assert.equal(await evaluate(`document.querySelectorAll('select').length`), 0);
  await pause(600); await screenshot('memory-type-sheet-390');
  assert.equal(await evaluate(`(()=>{const b=document.querySelector('[role="radio"][aria-checked="true"]'),r=b.getBoundingClientRect();return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest('[role="radio"]')===b})()`), true, 'nested choice stays above the underlying form');
  await evaluate(`[...document.querySelectorAll('[role="radio"]')].find(b=>b.textContent.trim()==='evento').click()`);
  await waitFor(`document.querySelector('[aria-label="Tipo de memória"]').textContent.trim()==='evento'`);
  await evaluate(`history.back()`); await waitFor(`!document.querySelector('[role="dialog"]')`);
  await click('Voltar'); await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  if (process.env.MC_UI_INTERFACE_ONLY === '1') {
    assert.equal(errors.length, 0, JSON.stringify(errors));
    console.log(`PASS: interface pt-BR/en, persistence, authored-content preservation, HEX palette, cancellation/Back/one confirmed request and memory selector. Screenshots: ${out}`);
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
  // The side menu only opens when pulled from the strip along the right edge (20px); x=383 avoids headless Edge's classic scrollbar.
  await swipe(310,300,-110,8);await pause(250);
  assert.equal(await evaluate(`!!document.querySelector('.mc-drawer')`),false,'drawer opens only from the right edge strip');
  await swipe(383,300,-220,8);await waitFor(`document.querySelector('.mc-drawer')`);await pause(800);
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
  // subtracts the elastic stretch in progress (data-elastic-y), like the app's own reading anchor (ChatView.rememberReading)
const anchor=()=>evaluate(`(() => {const el=document.querySelector('.mc-scroll'),top=el.getBoundingClientRect().top,ey=Number(el.dataset.elasticY)||0,a=[...el.querySelectorAll('[data-message-id]')].find(a=>a.getBoundingClientRect().bottom-ey>top);return {id:a?.dataset.messageId,offset:a?.getBoundingClientRect().top-top-ey,top:el.scrollTop};})()`);
  const stable=async before=>{await pause(100);const after=await anchor();assert.equal(after.id,before.id,'reading anchor preserved');assert.ok(Math.abs(after.offset-before.offset)<2,'reading offset preserved '+JSON.stringify({before,after}));};
  const noDelta=async()=>{assert.equal(await evaluate(`document.body.textContent.includes('RESPOSTA_PRONTA_')`),false,'no buffered delta text before done');assert.equal(await evaluate(`document.querySelectorAll('.streaming-cursor').length`),0);};
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=0;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);const beforeContinue=await anchor();
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
  await evaluate(`(() => {const t=document.querySelector('textarea');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(t,${JSON.stringify('Mensagem de envio.\n'+('compositor grande '.repeat(30)))});t.dispatchEvent(new Event('input',{bubbles:true}));})()`);await stable(beforeSend);
  await click('Enviar');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await waitFor(`document.querySelector('textarea').value===''`);await noDelta();await stable(beforeSend);
  for(let n=0;n<60&&!calls.some(c=>c.reconnect===15);n++)await pause(50);assert.ok(calls.some(c=>c.reconnect===15),'reconnection uses UTF-16 offset after emoji');
  complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);await stable(beforeSend);
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-message-id]')).at(-1).textContent.includes('RESPOSTA_PRONTA_')`),true,'done presents complete reply in one burst');
  // Frozen socket: the native event resumes without duplicating the generation.
  await click('Continuar a cena');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);
  const operations=calls.filter(c=>c.mode).length,reconnections=calls.filter(c=>c.reconnect!=null).length;
  await evaluate(`window.dispatchEvent(new Event('mc:resume'))`);
  await waitFor(`!document.body.textContent.includes('Sem conexão com o PC')`);
  for(let n=0;n<40&&calls.filter(c=>c.reconnect!=null).length===reconnections;n++)await pause(50);
  assert.ok(calls.filter(c=>c.reconnect!=null).length>reconnections);
  assert.equal(calls.filter(c=>c.mode).length,operations,'resume attaches instead of regenerating');
  complete();await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  // WebView recreated after the reply finished while the process was not running.
  await click('Refazer a última resposta');await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);
  const reloadOperations=calls.filter(c=>c.mode).length;
  await cdp('Page.navigate',{url:'about:blank'});complete();await cdp('Page.navigate',{url:base});
  await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Cena fictícia'))`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Cena fictícia')).click()`);
  await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  assert.equal(calls.filter(c=>c.mode).length,reloadOperations,'completed generation recovered without new model operation');
  assert.equal(await evaluate(`localStorage.getItem('mc.pending.fixture')`),null);
  const resumedAnchor=await anchor();
  chat.revision++;chat.history.push({id:'remote',role:'assistant',content:'Atualização remota '+('história '.repeat(90)),status:'complete'});
  await evaluate(`window.dispatchEvent(new CustomEvent('mc:event',{detail:{convId:'fixture',revision:${chat.revision},type:'message'}}))`);await pause(500);await stable(resumedAnchor);
  chat.history = [{ id: 'u1', role: 'user', content: 'Mensagem sem resposta após falha', interactionId: 'i1' }]; await refresh();
  await waitFor(`document.querySelector('[aria-label="Tentar responder de novo"]')`);
  assert.equal(await evaluate(`document.querySelectorAll('.mc-chat-action').length`), 1);
  await screenshot('user-tail-retry'); await noActionFlash(); await click('Tentar responder de novo'); await waitFor(`document.querySelector('[aria-label="Parar geração"]')`);await noDelta(); assert.equal(calls.at(-1).mode, 'regenerate');
  await click('Parar geração'); await waitFor(`document.querySelector('[aria-label="Continuar a cena"]')`);
  failChat = true; await refresh(); await waitFor(`document.body.textContent.includes('Sem conexão com o PC')`);
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('.mc-chat-action')).every(b=>b.disabled)`), true);
  assert.equal(await evaluate(`!!document.querySelector('[aria-label="Modelo sem conexão confirmada"].bg-gray-500')`),true);
  await evaluate(`(() => { const t=document.querySelector('textarea'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(t,'Rascunho offline'); t.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  assert.equal(await evaluate(`document.querySelector('[aria-label="Enviar"]').disabled`), true); await screenshot('offline-draft');
  failChat = false; await refresh(); await waitFor(`!document.body.textContent.includes('Sem conexão com o PC')`);
  await waitFor(`document.querySelector('[aria-label="Modelo conectado"].bg-emerald-400')`);
  await cdp('Emulation.setDeviceMetricsOverride', { width: 360, height: 420, deviceScaleFactor: 1, mobile: true });
  await evaluate(`document.documentElement.style.setProperty('--safe-area-inset-top','24px');document.documentElement.style.setProperty('--safe-area-inset-bottom','20px');document.querySelector('textarea').focus()`);
  const composer = await evaluate(`(() => {const t=document.querySelector('textarea').getBoundingClientRect();return {bottom:t.bottom,height:innerHeight,overflow:document.documentElement.scrollWidth>innerWidth};})()`);
  assert.ok(composer.bottom < composer.height - 20); assert.equal(composer.overflow, false); await screenshot('keyboard-height-safe-area');
  await evaluate(`document.documentElement.style.setProperty('--safe-area-inset-top','0px');document.documentElement.style.setProperty('--safe-area-inset-bottom','0px')`);
  // Real cached-first opening, with a held GET and a stale public/generic preset cache.
  chat=initial();chat.history[2].content+='\n'+('Trecho de leitura no cache. '.repeat(220));await refresh();await pause(200);
  const stalePresets={presets:[
    {id:'generic',label:'LEGADO_GENERICO',efforts:['high'],routes:[{id:'hidden'}]},
    {id:'unknown',label:'LEGADO_OUTRO',efforts:['high'],routes:[{id:'hidden2'}]},
    {id:'deepseek-v41-flash',label:'PRESET_PERMITIDO_OFFLINE',efforts:['high'],defaultEffort:'high',routes:[{id:'cbai/deepseek-v4.1-flash'}]},
    // api.cachedPresets() only trusts a cache with the full Gemini preset (3 levels, 3 routes); without it, it falls back to the defaults
    {id:'gemini-38-flash',label:'GEMINI_CACHE',efforts:['low','medium','high'],defaultEffort:'low',routes:[{id:'ag/gemini-3.8-flash-low'},{id:'ag/gemini-3.8-flash-medium'},{id:'ag/gemini-3.8-flash-high'}]},
  ]};
  await evaluate(`new Promise((resolve,reject)=>{const req=indexedDB.open('modularchat',1);req.onsuccess=()=>{const db=req.result,tx=db.transaction('kv','readwrite');tx.objectStore('kv').put(${JSON.stringify(stalePresets)},'presets');tx.oncomplete=()=>{db.close();resolve()};tx.onerror=reject};req.onerror=reject})`);
  let releaseChat;delayChat=new Promise(r=>releaseChat=r);failPresets=true;
  await cdp('Page.navigate',{url:base});
  await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Cena fictícia'))`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Cena fictícia')).click()`);
  await waitFor(`document.querySelector('.mc-scroll [data-message-id="a2"]')`);
  await evaluate(`document.querySelector('.mc-scroll').scrollTop=140;document.querySelector('.mc-scroll').dispatchEvent(new Event('scroll'))`);
  const cachedAnchor=await anchor();
  chat.history.unshift({id:'inserted-cache',role:'assistant',content:'NOVO_GET '+('Texto novo antes da âncora. '.repeat(16)),status:'complete'});chat.revision++;
  delayChat=null;releaseChat();await waitFor(`document.body.textContent.includes('NOVO_GET')`);await stable(cachedAnchor);
  await click('Opções');await waitFor(`Array.from(document.querySelectorAll('button')).some(b=>b.textContent.includes('Modelo e esforço'))`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.includes('Modelo e esforço')).click()`);
  await waitFor(`document.body.textContent.includes('PRESET_PERMITIDO_OFFLINE')`);
  assert.equal(await evaluate(`document.body.textContent.includes('LEGADO_GENERICO')||document.body.textContent.includes('LEGADO_OUTRO')`),false,'offline cache hides generic and unsupported presets');
  await screenshot('cached-get-anchor-offline-presets');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false }); await screenshot('desktop');
  if(privateFixture) {
    const protectedCache=await evaluate(`new Promise((resolve,reject)=>{const req=indexedDB.open('modularchat',1);req.onsuccess=()=>{const db=req.result,tx=db.transaction('kv','readonly'),r=tx.objectStore('kv').getAll();tx.oncomplete=()=>{db.close();resolve(r.result.map(v=>!!v?.sealed))};tx.onerror=reject};req.onerror=reject})`);
    assert.ok(protectedCache.length&&protectedCache.every(Boolean),'all cache values sealed');
    assert.equal(await evaluate(`!!localStorage.getItem('mc.private.v1')&& !localStorage.getItem('mc.private.v1').includes('Rascunho offline')`),true,'drafts sealed at rest');
    assert.equal(await evaluate(`Array.from({length:localStorage.length},(_,i)=>localStorage.key(i)).some(k=>k.startsWith('mc.draft.')||k.startsWith('mc.pending.'))`),false,'no plaintext draft/pending slots');
  }
  assert.deepEqual(errors, []);
  await fs.writeFile(path.join(out, 'ui-results.json'), JSON.stringify({ status: 'passed', widths, feedback, requests: calls.map(c=>c.mode || 'stop'), runtimeErrors: errors, limits: 'Viewport reduzido simula espaço do teclado. IME/teclado e WebView Android físicos não testados.' }, null, 2));
  console.log(`PASS: edição user/bot e falha recuperável, cópia Markdown, slider salvo ao sair, gestos verticais diagonais e gaveta lateral, UI widths/font, icon aria/title/44px/focus, buffer invisible before done, regeneration target hidden, native resume and completed generation recovery without duplication, UTF-16 reconnect, scroll anchors, offline cached presets, long press, offline, safe areas, simulated keyboard height. Screenshots: ${out}`);
  }
} finally {
  ws?.close(); for (const p of pending.values()) clearTimeout(p.timer);
  if (active) active.res.destroy(); server.closeAllConnections(); await new Promise(r=>server.close(r));
  edge.kill(); await Promise.race([new Promise(r=>edge.once('exit',r)), pause(3000)]);
  await fs.rm(profile, { recursive: true, force: true, maxRetries: 4, retryDelay: 200 }).catch(() => {});
}
