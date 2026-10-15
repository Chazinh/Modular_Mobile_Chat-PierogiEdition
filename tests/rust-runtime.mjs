import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { mockRouter } from './helpers.js';
import { startRust } from './rust-http.js';
const wait=async(fn,timeout=20000)=>{const deadline=Date.now()+timeout;while(Date.now()<deadline){const x=await fn();if(x)return x;await new Promise(r=>setTimeout(r,30));}throw new Error('Rust runtime deadline');};
async function fixture(t,{local=false,maxGenerations=4,review=true}={}){
  const dir=await fs.mkdtemp(path.join(os.tmpdir(),'mc-native-'));const upstream=await mockRouter();const localRequests=[];let mode='clean',gate=null;
  const model=http.createServer(async(req,res)=>{let raw='';for await(const c of req)raw+=c;const body=JSON.parse(raw||'{}');localRequests.push(body);res.setHeader('content-type','application/json');
    if(req.url.endsWith('/embeddings'))return res.end(JSON.stringify({data:[{embedding:[1,0,0]}]}));
    const schema=body.response_format?.json_schema?.schema;
    if(schema?.properties?.decisions){
      // curation: merges the first group (keeps the second id) and tries a decision with an invented id in the second
      const ids=[...String(body.messages?.[1]?.content||'').matchAll(/id=([0-9a-f-]+):/g)].map(m=>m[1]);
      const decisions=[{group:1,action:'merge',keep_id:ids[1],merged:'Laura0 escondeu o frasco0 verde de poção0 atrás do armário0 da cozinha0.'},{group:2,action:'merge',keep_id:'id-inventado',merged:'texto qualquer sem relação'}];
      return res.end(JSON.stringify({choices:[{message:{content:JSON.stringify({decisions})},finish_reason:'stop'}]}));
    }
    const currentMode=mode;
    if(schema?.properties?.facts&&gate){const held=gate;gate=null;await held;}
    if(currentMode==='invalid')return res.end(JSON.stringify({choices:[{message:{content:'{invalid json'},finish_reason:'stop'}]}));
    const result=schema?.properties?.agency?mode==='clean'?{agency:[],knowledge:[],continuity:[]}:{agency:[{quote:'Alex: Texto proibido.',reason:'agência'}],knowledge:[],continuity:[]}:
      {summary:'Lia guarda uma chave no porão.',cast:[],scene:{location:'Quarto',time:'',mood:'',objects:[],present:['Lia'],characters:[{name:'Lia',position:'sentada',location:'Quarto',holding:[]}]},facts:[{kind:'OBJECT',subject:'Lia',content:currentMode==='newfact'?'A chave de Lia está no jardim.':'A chave de Lia está guardada no porão.',importance:.8,confidence:1,visibility:'public',knowledge:[],replaces:'',existing_id:''}],relationships:[],open_threads:[],resolved_threads:[]};
    res.end(JSON.stringify({choices:[{message:{content:JSON.stringify(result)},finish_reason:'stop'}]}));
  });await new Promise(r=>model.listen(0,'127.0.0.1',r));
  const localUrl=`http://127.0.0.1:${model.address().port}`;
  let runtime=await startRust({dataDir:dir,upstream:upstream.url,localUrl:local?localUrl:undefined,maxGenerations,review});
  const j=async(method,p,body)=>{const r=await fetch(runtime.base+p,{method,headers:{'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(30000)});return {status:r.status,body:await r.json().catch(()=>({}))};};
  const chat=async()=>{const r=await j('POST','/api/chats',{name:'Rust fixture',prompt:'Nome: Lia\nLia guarda a chave.'});assert.equal(r.status,201);return r.body;};
  const jobs=c=>{const db=new DatabaseSync(path.join(dir,'modularchat.db'),{readOnly:true});try{return db.prepare('SELECT kind,status,error,attempts FROM jobs WHERE conv_id=?').all(c);}finally{db.close();}};
  t.after(async()=>{await runtime.close();await upstream.close();model.closeAllConnections();await new Promise(r=>model.close(r));await fs.rm(dir,{recursive:true,force:true});});
  return {dir,localUrl,get runtime(){return runtime;},upstream,localRequests,j,chat,jobs,setMode(v){mode=v;},holdExtraction(){gate=new Promise(r=>this.releaseExtraction=r);},async restart(){await runtime.close();runtime=await startRust({dataDir:dir,upstream:upstream.url,localUrl,maxGenerations,review});}};
}
test('supervisor Rust: revisão, extração integral, embeddings e bloqueio persistido',async t=>{
  const f=await fixture(t,{local:true});const c=await f.chat();const fullReply='Lia: '+('narrativa '.repeat(880))+'CAUDA_REAL';
  // Preserve the entire long-turn fixture through a real continuation; each new part must fit 8k.
  f.upstream.set({chunks:[fullReply.slice(0,7998)],delayMs:1});
  const res=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'a'.repeat(5900),opId:'native-jobs'}),signal:AbortSignal.timeout(30000)});assert.equal(res.status,200);await res.text();
  f.upstream.set({chunks:[fullReply.slice(7998)]});
  const continuation=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'continue',content:null,opId:'native-jobs-continuation'}),signal:AbortSignal.timeout(30000)});assert.equal(continuation.status,200);await continuation.text();
  const saved=(await f.j('GET',`/api/chats/${c.id}`)).body;
  assert.equal(saved.history.filter(m=>m.role==='assistant').map(m=>m.content).join(''),fullReply,'no long-turn text lost across parts');
  assert.ok(saved.history.some(m=>m.role==='assistant'&&m.content.endsWith('CAUDA_REAL')), 'complete tail persisted: '+JSON.stringify(saved.history.map(m=>({role:m.role,status:m.status,length:m.content?.length,tail:m.content?.slice(-40)}))));
  const mem=await wait(async()=>{const d=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return d.sync?.synced&&d.memories?.length?d:null;});
  assert.ok(mem.memories.some(m=>m.content.includes('chave')));assert.ok(f.localRequests.some(b=>b.messages?.at(-1)?.content.includes('CAUDA_REAL')), 'tail absent in '+JSON.stringify(f.localRequests.map(b=>({kind:Object.keys(b.response_format?.json_schema?.schema?.properties||{}),length:b.messages?.at(-1)?.content.length,tail:b.messages?.at(-1)?.content.slice(-120)}))));
  assert.ok(f.localRequests.filter(b=>b.response_format?.json_schema?.schema?.properties?.facts).length>=3,'every fragment extracted');
  await wait(()=>f.localRequests.some(b=>Array.isArray(b.input)));
  f.setMode('flags');f.upstream.set({chunks:['Alex: Texto proibido.']});const c2=await f.chat();
  const response=await fetch(`${f.runtime.base}/api/chats/${c2.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'native-flags'})});await response.text();
  const flagged=await wait(async()=>{const x=(await f.j('GET',`/api/chats/${c2.id}/memory`)).body;return x.flags?.length?x:null;});assert.ok(flagged.flags.some(x=>x.kind==='agency'));
  assert.equal((await f.j('POST',`/api/chats/${c2.id}/flags/${flagged.flags[0].id}/resolve`,{action:'dismiss'})).status,200);
  // once the flag is dismissed, the turn is extracted without being blocked again
  const released=await wait(async()=>{const x=(await f.j('GET',`/api/chats/${c2.id}/memory`)).body;return x.memories?.length&&!x.jobs?.blocked?x:null;});
  assert.ok(released.memories.some(m=>m.content.includes('chave')));
});
test('Rust (padrão de produção): sem revisão automática — o turno vai direto à extração, sem sinalizações nem chamada de revisão',async t=>{
  const f=await fixture(t,{local:true,review:false});const c=await f.chat();f.setMode('flags');f.upstream.set({chunks:['Alex: Texto proibido.']});
  const r=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'no-review'})});await r.text();
  const mem=await wait(async()=>{const m=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return m.sync?.synced&&m.memories?.length?m:null;});
  assert.deepEqual(mem.flags,[],'nenhuma sinalização "a revisar"');
  assert.ok(!f.localRequests.some(b=>b.response_format?.json_schema?.schema?.properties?.agency),'o modelo local não é chamado para revisar');
  assert.ok(f.localRequests.some(b=>b.response_format?.json_schema?.schema?.properties?.facts),'a extração continua acontecendo');
  assert.ok(f.jobs(c.id).every(j=>j.status!=='blocked'),'nada fica bloqueado esperando revisão');
});
test('Rust (padrão de produção): turnos bloqueados por revisões antigas são liberados na subida, sem apagar nada',async t=>{
  const f=await fixture(t,{local:true,review:true});const c=await f.chat();f.setMode('flags');f.upstream.set({chunks:['Alex: Texto proibido.']});
  const r=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'legacy-review'})});await r.text();
  await wait(async()=>(await f.j('GET',`/api/chats/${c.id}/memory`)).body.flags?.length);
  assert.ok(f.jobs(c.id).some(j=>j.status==='blocked'),'o turno ficou bloqueado pela revisão antiga');
  await f.runtime.close();
  // restart as in production (review off): releases the blocked turn and dismisses the flag
  f.setMode('clean');
  const next=await startRust({dataDir:f.dir,upstream:f.upstream.url,localUrl:f.localUrl,review:false});
  try{
    const memory=async()=>(await (await fetch(next.base+`/api/chats/${c.id}/memory`)).json());
    const done=await wait(async()=>{const m=await memory();return m.sync?.synced&&m.memories?.length?m:null;});
    assert.deepEqual(done.flags,[]);
    assert.ok(f.jobs(c.id).every(j=>j.status!=='blocked'));
  }finally{await next.close();}
});
test('supervisor Rust: limite global antes do commit e replay idempotente',async t=>{
  const f=await fixture(t,{maxGenerations:1});const c=await f.chat(),other=await f.chat();let release;f.upstream.set({hold:new Promise(r=>release=r)});
  const active=fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'native-slot'})});
  await wait(async()=>!!(await f.j('GET',`/api/chats/${c.id}`)).body.generation);
  const r=await f.j('POST',`/api/chats/${other.id}/message`,{mode:'send',content:'Oi',opId:'overload'});assert.equal(r.status,503);assert.equal(r.body.code,'GENERATION_OVERLOADED');assert.equal((await f.j('GET',`/api/chats/${other.id}`)).body.history.length,0);
  release();await(await active).text();const count=f.upstream.requests.length;
  const replay=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'native-slot'})});assert.ok((await replay.text()).includes('"replay":true'));assert.equal(f.upstream.requests.length,count);
  f.upstream.set({hold:null});
  const next=await fetch(`${f.runtime.base}/api/chats/${other.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Novo envio',opId:'slot-after-done'})});assert.equal(next.status,200,'readonly delayed verification must not reserve a generation slot');await next.text();
});
test('Rust: parser limitado rejeita raciocínio enorme sem destruir a mensagem',async t=>{
  const f=await fixture(t);const c=await f.chat();f.upstream.set({reasoning:['x'.repeat(300000)],chunks:['Não deve aparecer.']});
  const res=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'native-parser'})});const sse=await res.text();assert.ok(sse.includes('"done":true'));assert.ok(sse.includes('limite'));assert.ok(!sse.includes('Não deve aparecer'));
  const chat=(await f.j('GET',`/api/chats/${c.id}`)).body;assert.equal(chat.history[0].content,'Oi');
});
test('Rust: snapshot SQLite consistente preserva IDs e não sobrescreve destino',async t=>{
  const f=await fixture(t);const c=await f.chat();const backup=await f.j('POST','/api/backup',{});assert.equal(backup.status,200);
  const a=new DatabaseSync(path.join(f.dir,'modularchat.db'),{readOnly:true}),b=new DatabaseSync(backup.body.file,{readOnly:true});try{assert.equal(b.prepare('PRAGMA integrity_check').get().integrity_check,'ok');assert.deepEqual(a.prepare('SELECT id FROM conversations ORDER BY id').all(),b.prepare('SELECT id FROM conversations ORDER BY id').all());assert.ok(b.prepare('SELECT id FROM conversations WHERE id=?').get(c.id));}finally{a.close();b.close();}
});

test('Rust: jobs indisponíveis sobrevivem ao reinício sem repetir upstream',async t=>{
  const f=await fixture(t);const c=await f.chat();f.upstream.set({chunks:['Lia: A chave fica no porão.']});
  const r=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Onde está a chave?',opId:'recover-job'})});await r.text();
  const pending=await wait(async()=>{const m=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return f.jobs(c.id).some(j=>j.status==='pending'&&j.error?.includes('indisponível'))?m:null;});
  assert.equal(pending.memories.length,0);assert.equal(f.localRequests.length,0);const count=f.upstream.requests.length;
  await f.restart();await wait(async()=>{const m=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return m.sync?.synced&&m.memories?.length;});assert.equal(f.upstream.requests.length,count);
  const replay=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Onde está a chave?',opId:'recover-job'})});const replayText=await replay.text();assert.ok(replayText.includes('"replay":true'));assert.ok(replayText.includes('Lia: A chave fica no porão.'));assert.equal(f.upstream.requests.length,count);
});
test('Rust: JSON inválido falha após tentativas limitadas e retry recupera',async t=>{
  const f=await fixture(t,{local:true});const c=await f.chat();f.setMode('invalid');
  const r=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'invalid-job'})});await r.text();
  const failed=await wait(async()=>{const m=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return f.jobs(c.id).some(j=>j.kind==='turn'&&j.status==='failed')?m:null;});
  assert.equal(failed.memories.length,0);assert.ok(f.jobs(c.id).some(j=>j.kind==='turn'&&j.attempts===3&&j.error));
  f.setMode('clean');assert.equal((await f.j('POST',`/api/chats/${c.id}/jobs/retry`,{})).status,200);
  await wait(async()=>{const m=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return m.sync?.synced&&m.memories?.length;});
});
test('Rust: extração em andamento da versão antiga não contamina edição',async t=>{
  const f=await fixture(t,{local:true});const c=await f.chat();f.holdExtraction();
  const r=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'stale-job'})});await r.text();
  await wait(()=>f.localRequests.some(b=>b.response_format?.json_schema?.schema?.properties?.facts));
  const chat=(await f.j('GET',`/api/chats/${c.id}`)).body;const part=chat.history.find(m=>m.role==='assistant');
  f.setMode('newfact');assert.equal((await f.j('PUT',`/api/chats/${c.id}/messages/${part.id}`,{content:'Lia: A chave está no jardim.'})).status,200);f.releaseExtraction();
  const mem=await wait(async()=>{const m=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;return m.sync?.synced&&m.memories?.some(m=>m.content.includes('jardim'))?m:null;});
  assert.ok(!mem.memories.some(m=>m.content.includes('porão')));assert.ok(f.jobs(c.id).some(j=>j.status==='skipped'));
});
test('Rust: supervisor produz episódios com fontes integrais e não deixa embeddings passar fome',async t=>{
  const f=await fixture(t,{local:true});const c=await f.chat();const data=(await f.j('GET',`/api/chats/${c.id}/export`)).body.conversations[0];
  data.interactions=[];data.parts=[];data.versions=[];data.interaction_notes=[];let parent=null;const date='2026-01-01T00:00:00.000Z';
  for(let seq=1;seq<=70;seq++){const id=randomUUID(),pid=randomUUID(),vid=randomUUID();data.interactions.push({id,conv_id:c.id,parent_id:parent,seq,kind:'turn',user_text:`Pergunta ${seq}`,user_ts:date,created_at:date});data.parts.push({id:pid,interaction_id:id,idx:0,selected_version_id:vid,created_at:date});data.versions.push({id:vid,part_id:pid,content:`Lia: FONTE_INTEGRAL_${seq} fala sobre a chave.`,status:'complete',origin:'legacy',created_at:date,finished_at:date});parent=id;}
  data.conversation.head_interaction_id=parent;const imported=await f.j('POST','/api/import',{format:'modularchat-export',version:1,conversations:[data]});assert.equal(imported.status,201);const cid=imported.body.imported[0];
  const overview=await wait(async()=>{const m=(await f.j('GET',`/api/chats/${cid}/memory`)).body;return m.sync?.synced&&m.episodes?.length?m:null;},90000);
  assert.ok(overview.episodes.length>0);const episode=f.localRequests.find(b=>b.messages?.[0]?.content.includes('Resuma em até 4 frases'));assert.ok(episode?.messages.at(-1).content.includes('FONTE_INTEGRAL_1'),'episode summarizes original source');
  const embedding=f.localRequests.findIndex(b=>Array.isArray(b.input));const lastExtraction=f.localRequests.findLastIndex(b=>b.response_format?.json_schema?.schema?.properties?.facts);assert.ok(embedding>=0&&embedding<lastExtraction,'embeddings get work during turn backlog');
});
test('Rust: curadoria faz uma única chamada ao modelo local e aplica só a fusão segura',async t=>{
  const f=await fixture(t,{local:true,review:false});const c=await f.chat();
  const words=['jardim','biblioteca','torre','lago','cozinha','estufa','ponte','porão','varanda','altar','galpão','mercado','oficina','cripta'];
  const add=async m=>{const r=await f.j('POST',`/api/chats/${c.id}/memory`,{visibility:'public',importance:.5,...m});assert.equal(r.status,201);return r.body.id;};
  for(let i=0;i<66;i++)await add({kind:'WORLD_FACT',subject:`Lugar ${i}`,content:`${words[i%words.length]} número ${i} tem a marca ${i*7} entalhada na porta ${i+100}`});
  const dup=[];
  for(let p=0;p<4;p++){const base=`Laura${p} escondeu o frasco${p} verde de poção${p} atrás do armário${p} da cozinha${p}`;
    dup.push([await add({kind:'OBJECT',subject:`Laura${p}`,content:base,importance:.4}),await add({kind:'OBJECT',subject:`Laura${p}`,content:`${base} ontem`,importance:.8})]);}
  // a finished turn is what schedules curation (too many memories and new ones)
  const r=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'Oi',opId:'curation-turn'})});await r.text();
  const curated=()=>f.localRequests.filter(b=>b.response_format?.json_schema?.schema?.properties?.decisions);
  await wait(()=>f.jobs(c.id).some(j=>j.kind==='curate'&&['done','skipped'].includes(j.status)));
  assert.equal(curated().length,1,'uma chamada só');
  assert.match(curated()[0].messages[1].content,/GRUPO 1:/);
  const mem=(await f.j('GET',`/api/chats/${c.id}/memory`)).body;
  const current=new Set(mem.memories.map(m=>m.id));
  assert.equal(dup.flat().filter(id=>!current.has(id)).length,1,'só a fusão segura foi aplicada; a decisão com id inventado foi ignorada');
  assert.ok(current.has(dup[0][1])&&!current.has(dup[0][0]),'a mais nova do primeiro grupo foi mantida');
});
test('Rust: resposta cortada pelo limite (length) não passa pela correção de estilo e mantém o status',async t=>{
  const f=await fixture(t);const original='Lia: '+'Encontramos uma carta com o endereço. '.repeat(10);
  const corrected='Lia: Achei uma carta com o endereço. Podemos conferir.';
  assert.equal((await f.j('PUT','/api/settings/global',{botChars:600,dialoguePct:100,monologueChars:150,content:'',lengthMode:'maximum',turnPacing:'interactive'})).status,200);
  const send=async(c,opId,finish)=>{f.upstream.set({chunks:[original],finish,correctionText:corrected,correctionStatus:null,delayMs:1});
    const n=f.upstream.correctionRequests.length;const res=await fetch(`${f.runtime.base}/api/chats/${c.id}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:'O que encontrou?',opId}),signal:AbortSignal.timeout(30000)});await res.text();
    const last=(await f.j('GET',`/api/chats/${c.id}`)).body.history.filter(m=>m.role==='assistant').at(-1);return {last,corrections:f.upstream.correctionRequests.length-n};};
  // control: the same finished reply (stop) is rewritten once
  const done=await send(await f.chat(),'style-complete','stop');
  assert.equal(done.corrections,1);assert.equal(done.last.content,corrected);
  // cut by the limit: no fix call, original text and status
  const cut=await send(await f.chat(),'style-length','length');
  assert.equal(cut.corrections,0,'resposta cortada não é reescrita');
  assert.equal(cut.last.status,'length');assert.equal(cut.last.content.trim(),original.trim());
});
