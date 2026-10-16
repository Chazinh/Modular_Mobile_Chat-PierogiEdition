// Exercise both settings entry points, including HomeDrawer with the real API transport.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const client = path.join(root, 'client');
const work = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-pairing-ui-'));
const profile = path.join(work, 'edge');
await fs.mkdir(profile);
const modulePath = file => JSON.stringify(path.join(client, file).replaceAll('\\', '/'));
const entry = path.join(work, 'entry.jsx');
await fs.writeFile(entry, `
import React from 'react';
import { createRoot } from 'react-dom/client';
import GlobalSettings from ${modulePath('src/components/GlobalSettings.jsx')};
import HomeDrawer from ${modulePath('src/components/HomeDrawer.jsx')};
import ServerSetup from ${modulePath('src/components/ServerSetup.jsx')};
import { ConfirmProvider } from ${modulePath('src/components/ConfirmProvider.jsx')};
import { api } from ${modulePath('src/api.js')};
const drawer=new URLSearchParams(location.search).get('surface')==='drawer';
const setup=new URLSearchParams(location.search).get('surface')==='setup';
let host='pc', fail=false, hold=false, release, stale;
let rows={pc:[{id:'old',name:'Pareamento antigo'},{id:'second',name:'Segundo antigo'}],local:[{id:'android',name:'Host Android'}]};
window.__calls=[];
window.__fixture={setHost:value=>{host=value;window.dispatchEvent(new Event('mc:connection-change'))},fail:value=>{fail=value},hold:()=>{hold=true},release:()=>release?.(),removeElsewhere:id=>{rows[host]=rows[host].filter(row=>row.id!==id);window.dispatchEvent(new CustomEvent('mc:event',{detail:{type:'devices'}}))}};
if(!drawer)api.devices=async options=>{
  const captured=host, saved=stale?.host===host?stale.rows:rows[host].map(row=>({...row}));if(stale?.host===host)stale=null;
  if(hold){hold=false;await new Promise(resolve=>{release=resolve})}
  options.onTarget?.({server:captured==='pc'?'http://192.168.1.6:3000':'http://127.0.0.1:3011',kind:captured});
  return {devices:saved};
};
if(!drawer)api.revokeDevice=async(id,options)=>{
  window.__calls.push({id,origin:options.origin});
  const captured=options.origin.kind;
  await new Promise(resolve=>setTimeout(resolve,250));
  if(fail)throw Error('Falha simulada de remoção');
  stale={host:captured,rows:rows[captured].map(row=>({...row}))};
  rows[captured]=rows[captured].filter(row=>row.id!==id);
  return {ok:true,devices:rows[captured]};
};
api.status=async()=>({router:{ok:true,models:0},local:{chat:{ok:true},embed:{ok:true}},db:{jobs:{}}});api.trash=async()=>({days:30,items:[]});api.backup=async()=>({file:'fixture'});
api.peekGlobalSettings=()=>({dialoguePct:40,botChars:1000});
if(setup){
  window.Capacitor={isNativePlatform:()=>true,isPluginAvailable:()=>false};
  localStorage.setItem('mc.connection',JSON.stringify({local:'http://127.0.0.1:3000',remote:''}));
  const send=window.fetch.bind(window);window.__probeCalls=[];window.__connectCalls=[];
  window.fetch=(url,options)=>{window.__probeCalls.push({url,method:options.method});return send('/__pc-health',options)};
  api.health=async()=>{window.__connectCalls.push('health');return {ok:true}};
  api.pair=async()=>{window.__connectCalls.push('pair');return {}};
  api.switchToPc=async()=>{window.__connectCalls.push('switch');return {kind:'pc'}};
}
const Surface=setup?ServerSetup:drawer?HomeDrawer:GlobalSettings;
createRoot(document.getElementById('root')).render(<React.StrictMode><ConfirmProvider><Surface fontSize={16} onFontSize={()=>{}} onClose={()=>{}} onChanged={()=>{}} onResetServer={()=>{}} onDone={()=>{window.__connectCalls?.push('done')}}/></ConfirmProvider></React.StrictMode>);
`);
const { build } = await import(pathToFileURL(path.join(client, 'node_modules/vite/dist/node/index.js')).href);
await build({ root: client, configFile: false, logLevel: 'error', css: { postcss: { plugins: [] } },
  resolve: { alias: { react: path.join(client, 'node_modules/react'), 'react-dom': path.join(client, 'node_modules/react-dom') } },
  build: { outDir: path.join(work, 'dist'), emptyOutDir: true, rollupOptions: { input: entry, output: { entryFileNames: 'fixture.js' } } } });
const styles = (await fs.readdir(path.join(client, 'dist/assets'))).filter(name => name.endsWith('.css'));
let wireRows = [{id:'wire-old',name:'Antigo no menu inicial'}, {id:'wire-second',name:'Outro no menu inicial'}];
let wireStale, wireFail = false;
const wireDeletes = [];
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://fixture');
    if (url.pathname === '/__pc-health') {
      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify({ok:true,name:'modularchat',encrypted:true}));
      return;
    }
    if (url.pathname === '/api/devices' && req.method === 'GET') {
      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify({devices:wireStale || wireRows})); wireStale = null;
      return;
    }
    if (url.pathname.startsWith('/api/devices/') && req.method === 'DELETE') {
      const id = decodeURIComponent(url.pathname.slice('/api/devices/'.length));
      wireDeletes.push(id);
      await new Promise(resolve=>setTimeout(resolve,250));
      res.setHeader('Content-Type', 'application/json');
      if (wireFail) { res.writeHead(500); res.end(JSON.stringify({error:'Falha HTTP de remoção'})); return; }
      wireStale = wireRows.map(row=>({...row}));
      wireRows = wireRows.filter(row=>row.id!==id);
      res.end(JSON.stringify({ok:true,devices:wireRows}));
      return;
    }
    if (url.pathname === '/') {
      res.setHeader('Content-Type', 'text/html');
      res.end(`<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1">${styles.map(name=>`<link rel="stylesheet" href="/assets/${name}">`).join('')}<div id="root"></div><script type="module" src="/__fixture/fixture.js"></script>`);
      return;
    }
    const base = url.pathname.startsWith('/assets/') ? path.join(client, 'dist') : path.join(work, 'dist');
    const file = path.resolve(base, '.' + url.pathname.replace(/^\/__fixture/, ''));
    assert.ok(file.startsWith(base + path.sep));
    res.setHeader('Content-Type', file.endsWith('.css') ? 'text/css' : 'text/javascript');
    res.end(await fs.readFile(file));
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const edge = spawn(process.env.MC_EDGE || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', [
  '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
], { windowsHide: true, stdio: 'ignore' });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
let ws;
const runtimeErrors = [];
try {
  let port;
  for (let n=0;n<100;n++) { try { port=(await fs.readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0];break; } catch { await pause(100); } }
  assert.ok(port, 'Headless Edge started');
  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t=>t.type==='page');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
  let seq=0; const pending=new Map();
  ws.addEventListener('message',({data})=>{const m=JSON.parse(data),p=pending.get(m.id);if(m.method==='Runtime.exceptionThrown')runtimeErrors.push(m.params.exceptionDetails.exception?.description||m.params.exceptionDetails.text);if(p){pending.delete(m.id);clearTimeout(p.timer);m.error?p.reject(Error(m.error.message)):p.resolve(m.result);}});
  const cdp=(method,params={})=>new Promise((resolve,reject)=>{const id=++seq,timer=setTimeout(()=>reject(Error(method+' timed out')),10000);pending.set(id,{resolve,reject,timer});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async expression=>{const r=await cdp('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;};
  const waitFor=async expression=>{for(let n=0;n<80;n++){if(await evaluate(`!!(${expression})`))return;await pause(50);}throw Error('UI condition timed out: '+expression+'; '+runtimeErrors.join('; ')+'; '+await evaluate('document.body.innerText.slice(0,500)'));};
  const selector=name=>`button[aria-label="Remover pareamento de ${name}"]`;
  const tap=async name=>{
    const point=await evaluate(`(()=>{const b=document.querySelector(${JSON.stringify(selector(name))});b.scrollIntoView({block:'center'});const r=b.getBoundingClientRect();return {x:r.right-7,y:r.top+r.height/2,width:r.width,height:r.height}})()`);
    assert.ok(point.width>=44&&point.height>=44,'Removal has a 44px touch target');
    await cdp('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:point.x,y:point.y}]});
    await cdp('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  };
  await cdp('Page.enable');await cdp('Runtime.enable');
  await cdp('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  await cdp('Emulation.setTouchEmulationEnabled',{enabled:true});
  await cdp('Page.navigate',{url:`http://127.0.0.1:${server.address().port}`});
  await waitFor(`document.querySelector(${JSON.stringify(selector('Pareamento antigo'))})`);
  await evaluate(`window.__fixture.hold();window.dispatchEvent(new CustomEvent('mc:event',{detail:{type:'devices'}}))`);
  await tap('Pareamento antigo');
  await waitFor(`document.querySelector('[aria-busy="true"]')`);
  await tap('Pareamento antigo');
  await waitFor(`!document.querySelector(${JSON.stringify(selector('Pareamento antigo'))})`);
  await evaluate('window.__fixture.release()');await pause(100);
  assert.equal(await evaluate('window.__calls.length'),1,'Repeated tap dispatches one deletion');
  assert.equal(await evaluate(`!!document.querySelector(${JSON.stringify(selector('Pareamento antigo'))})`),false,'Late list cannot restore a removed row');
  assert.match(await evaluate(`document.querySelector('[role="status"]').textContent`),/Pareamento removido/);
  await evaluate('window.__fixture.fail(true)');await tap('Segundo antigo');
  await waitFor(`document.querySelector('[role="alert"]')?.textContent.includes('Falha simulada')`);
  assert.equal(await evaluate(`!!document.querySelector(${JSON.stringify(selector('Segundo antigo'))})`),true,'Failed removal preserves row');
  await evaluate('window.__fixture.fail(false)');await tap('Segundo antigo');
  await evaluate(`window.__fixture.setHost('local')`);
  await waitFor(`document.querySelector(${JSON.stringify(selector('Host Android'))})`);
  await pause(350);
  const last=await evaluate('window.__calls.at(-1)');
  assert.equal(last.origin.kind,'pc');assert.equal(last.origin.server,'http://192.168.1.6:3000');
  assert.equal(await evaluate(`!!document.querySelector(${JSON.stringify(selector('Host Android'))})`),true,'Origin change keeps the Android row');
  await evaluate(`window.__fixture.removeElsewhere('android')`);
  await waitFor(`!document.querySelector(${JSON.stringify(selector('Host Android'))})`);
  assert.match(await evaluate('document.body.innerText'),/Pareamentos ativos: 0/);
  console.log('PASS GlobalSettings: touch removal, busy/double-tap guard, stale reads before/after deletion, feedback, fixed origin, and live remote removal.');

  await cdp('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?surface=drawer`});
  await waitFor(`[...document.querySelectorAll('button')].some(b=>b.textContent.includes('Pareamento / Conectar PC'))`);
  await evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent.includes('Pareamento / Conectar PC')).click()`);
  await waitFor(`document.querySelector(${JSON.stringify(selector('Antigo no menu inicial'))})`);
  await pause(700); // Let the real drawer entrance finish before testing its touch target.
  await tap('Antigo no menu inicial');
  await waitFor(`document.querySelector('[aria-busy="true"]')`);
  await tap('Antigo no menu inicial');
  await waitFor(`!document.querySelector(${JSON.stringify(selector('Antigo no menu inicial'))})`);
  assert.deepEqual(wireDeletes,['wire-old'],'Home menu sends one real DELETE for the selected pairing');
  assert.match(await evaluate('document.body.innerText'),/Pareamentos ativos: 1/);
  await pause(150);
  assert.equal(await evaluate(`!!document.querySelector(${JSON.stringify(selector('Antigo no menu inicial'))})`),false,'Home menu suppresses the stale HTTP GET after deletion');
  assert.match(await evaluate(`document.querySelector('[role="status"]').textContent`),/Antigo no menu inicial/);
  wireFail = true;
  await tap('Outro no menu inicial');
  await waitFor(`document.querySelector('[role="alert"]')?.textContent.includes('Falha HTTP')`);
  assert.equal(await evaluate(`!!document.querySelector(${JSON.stringify(selector('Outro no menu inicial'))})`),true,'Home menu keeps the row after a real HTTP failure');
  wireRows = []; wireStale = null;
  await evaluate(`window.dispatchEvent(new CustomEvent('mc:event',{detail:{type:'devices'}}))`);
  await waitFor(`!document.querySelector(${JSON.stringify(selector('Outro no menu inicial'))})`);
  assert.match(await evaluate('document.body.innerText'),/Pareamentos ativos: 0/);
  assert.deepEqual(runtimeErrors,[],'No runtime errors in either settings entry point');
  console.log('PASS HomeDrawer: actual home-menu navigation and API transport, 44px touch, one DELETE, immediate removal/count, stale HTTP response, HTTP failure and live event.');

  await cdp('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?surface=setup`});
  const lanLabel='Endereço do PC na rede local (Wi-Fi)', remoteLabel='Endereço pela internet (opcional)';
  await waitFor(`document.querySelector('input[aria-label=${JSON.stringify(lanLabel)}]')`);
  const testButton=`[...document.querySelectorAll('button')].find(b=>b.textContent==='Testar')`;
  const setInput=async(label,value)=>{
    await evaluate(`(()=>{const input=document.querySelector('input[aria-label=${JSON.stringify(label)}]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(value)});input.dispatchEvent(new Event('input',{bubbles:true}))})()`);
    await pause(50);
  };
  await evaluate(`${testButton}.click()`);
  await waitFor(`document.querySelector('[role="status"]')?.textContent.includes('próprio celular')`);
  assert.equal(await evaluate('window.__probeCalls.length'),0,'Testar explains loopback before any network call');
  await setInput('Código de pareamento (6 dígitos)','123456');
  const connectButton=`document.querySelector('button[type="submit"]')`;
  await evaluate(`${connectButton}.click()`);
  await waitFor(`document.querySelector('[role="alert"]')?.textContent.includes('próprio celular')`);
  assert.deepEqual(await evaluate('window.__connectCalls'),[],'Conectar rejects loopback before health, pairing or sync');
  await setInput(lanLabel,'http://192.168.1.100:3000');
  await evaluate(`${testButton}.click()`);
  await waitFor(`document.querySelector('[role="status"]')?.textContent.includes('confirmada')`);
  assert.deepEqual(await evaluate('window.__probeCalls.map(call=>call.url)'),['http://192.168.1.100:3000/api/health']);
  await setInput(lanLabel,''); await setInput(remoteLabel,'https://chat.example.com');
  assert.equal(await evaluate(`${testButton}.disabled`),false,'Remote-only address can be tested');
  await evaluate(`${testButton}.click()`);
  await waitFor(`document.querySelector('[role="status"]')?.textContent.includes('confirmada')`);
  assert.equal(await evaluate('window.__probeCalls.at(-1).url'),'https://chat.example.com/api/health');
  await evaluate(`${connectButton}.click()`);
  await waitFor(`window.__connectCalls.includes('done')`);
  assert.deepEqual(await evaluate('window.__connectCalls'),['health','pair','switch','done'],'Connection completes only after pairing and PC promotion');
  assert.deepEqual(runtimeErrors,[]);
  console.log('PASS ServerSetup: real Testar/Conectar form, loopback feedback before requests, LAN health, remote-only test and pairing/promotion order.');
} finally {
  ws?.close();edge.kill();await new Promise(resolve=>server.close(resolve));
  await pause(250);
  const resolved=path.resolve(work);
  assert.ok(resolved.startsWith(path.resolve(os.tmpdir())+path.sep));
  await fs.rm(resolved,{recursive:true,force:true,maxRetries:3,retryDelay:200});
}
