// Real HTTP generation + local JSON jobs, with disposable DBs and loopback simulators.
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { mockRouter } from './helpers.js';
import { startRust } from './rust-http.js';
import { mira as f, previousBot, currentUser, falseBot, noFlags, attributionReview, extractionFixture, contaminatedExtraction, validExtraction, legacyExtraction, reviewAttributionContract, normalizeReviewSchema } from './provenance-fixture.js';

// The prompt projects provenance as text; IDs, roles and full evidence stay in the raw sources.
// This quote starts on a new line, so the projection must not invent a citing speaker.
const quoteProvenance = [
  '[PROVENIÊNCIA DE CITAÇÕES RECENTES — correspondência exata]',
  `- “${f.quote}” foi dita originalmente por Mira.`,
  'Citar/imitar não cria declaração nova. Comentários fora das aspas são separados; referentes não explicitados permanecem desconhecidos.',
].join('\n');
const originalSource = sourceId => `[INÍCIO DA FONTE id=${sourceId} role=assistant; interlocutores definidos SOMENTE pelo texto, não pelo role]\n${previousBot}\n[FIM DA FONTE id=${sourceId}]`;
const hasReviewProvenance = (text,sourceId) => typeof sourceId==='string'
  &&text.includes(originalSource(sourceId))&&text.includes(quoteProvenance)
  &&text.includes(currentUser)&&text.includes(falseBot);

const suiteDir = fs.mkdtempSync(path.join(os.tmpdir(),'mc-provenance-runtime-'));
const upstream = await mockRouter();
after(async()=>{await upstream.close();fs.rmSync(suiteDir,{recursive:true,force:true});});

const wait = async (fn,ms=20000) => {
  const deadline=Date.now()+ms;
  while(Date.now()<deadline){const result=await fn();if(result)return result;await new Promise(r=>setTimeout(r,30));}
  throw new Error('provenance runtime fixture deadline');
};
async function evidence(name,data){
  if(!process.env.MC_PROVENANCE_EVIDENCE_DIR)return;
  fs.mkdirSync(process.env.MC_PROVENANCE_EVIDENCE_DIR,{recursive:true});
  fs.writeFileSync(path.join(process.env.MC_PROVENANCE_EVIDENCE_DIR,`rust-${name}.json`),JSON.stringify({backend:'Rust',executable:process.env.MC_SERVER_EXE,simulation:true,...data},null,2),'utf8');
}

// `extraction`/`review` decide the JSON the simulated local model returns for the current turn; tests may
// swap them later (ctl) to simulate the retry with a valid answer.
async function fixture(t,mode,{extraction=extractionFixture,review=mode==='flag'?attributionReview:noFlags}={}){
  const dir=fs.mkdtempSync(path.join(suiteDir,'fixture-'));
  const requests=[];
  const ctl={extraction,review};
  let sourceId;
  const model=http.createServer(async(req,res)=>{
    let raw='';for await(const chunk of req)raw+=chunk;
    const body=JSON.parse(raw||'{}');requests.push(body);
    const schema=body.response_format?.json_schema?.schema;
    const text=body.messages?.at(-1)?.content||'';
    let result;
    if(req.url.endsWith('/embeddings')) result={data:[{embedding:[1,0,0]}]};
    else {
      if(schema?.properties?.agency){
        // Simulated reviewer only flags the documented exact original + exact target.
        result=mode.startsWith('flag')&&hasReviewProvenance(text,sourceId)
          ? ctl.review():noFlags();
      }else if(schema?.properties?.facts){
        result=text.includes(currentUser)?ctl.extraction():{...extractionFixture(),summary:'Mira fala com Alex na Diretoria.',facts:[],scene:null};
      }else result={cast:[]};
      result={choices:[{message:{content:JSON.stringify(result)},finish_reason:'stop'}]};
    }
    res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify(result));
  });
  await new Promise(r=>model.listen(0,'127.0.0.1',r));
  const localUrl=`http://127.0.0.1:${model.address().port}`;
  const runtime=await startRust({dataDir:dir,upstream:upstream.url,localUrl});
  t.after(async()=>{
    await runtime.close();
    model.closeAllConnections();await new Promise(r=>model.close(r));
    fs.rmSync(dir,{recursive:true,force:true});
  });
  const j=async(method,p,body)=>{
    const res=await fetch(runtime.base+p,{method,headers:{'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(30000)});
    const data=await res.json();assert.ok(res.ok,`${p}: ${JSON.stringify(data)}`);return data;
  };
  const c=await j('POST','/api/chats',{name:'Mira fixture',persona:'Alex',prompt:'Nome: Mira',routeId:'cc/claude-sonnet-5-5',presetId:'claude-sonnet-5-5'});
  const exported=await j('GET',`/api/chats/${c.id}/export`);
  const data=exported.conversations[0],iid=randomUUID(),pid=randomUUID(),vid=randomUUID(),time='2026-10-12T00:00:00.000Z';
  data.conversation.head_interaction_id=iid;
  data.interactions=[{id:iid,conv_id:c.id,parent_id:null,seq:1,kind:'opening',user_text:null,created_at:time}];
  data.parts=[{id:pid,interaction_id:iid,idx:0,selected_version_id:vid,created_at:time}];
  data.versions=[{id:vid,part_id:pid,content:previousBot,status:'complete',origin:'legacy',created_at:time,finished_at:time}];
  const imported=await j('POST','/api/import',exported),cid=imported.imported[0];
  try {await wait(async()=> (await j('GET',`/api/chats/${cid}/memory`)).sync?.synced);} catch(error) {
    const diagnostics=await j('GET',`/api/chats/${cid}/memory`);
    throw new Error(error.message+' '+JSON.stringify({sync:diagnostics.sync,jobs:diagnostics.jobs,requests:requests.length}));
  }
  await wait(()=>requests.some(b=>b.response_format?.json_schema?.schema?.properties?.cast&&!b.response_format.json_schema.schema.properties.facts));
  const actual=(await j('GET',`/api/chats/${cid}/export`)).conversations[0];
  sourceId=actual.parts[0].selected_version_id;
  upstream.set({chunks:[falseBot]});
  const reply=await fetch(`${runtime.base}/api/chats/${cid}/message`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({mode:'send',content:currentUser,opId:`mira-${mode}`}),signal:AbortSignal.timeout(30000)});
  assert.equal(reply.status,200);const sse=await reply.text();assert.ok(sse.includes('"done":true'));
  return {dir,requests,j,cid,sourceId,ctl};
}

const extractionCalls=requests=>requests.filter(b=>b.response_format?.json_schema?.schema?.properties?.facts&&b.messages.at(-1).content.includes(currentUser));
const memoryOf=(fxt)=>fxt.j('GET',`/api/chats/${fxt.cid}/memory`);
// Rust answers only the number of re-queued jobs.
const retry=async fxt=>fxt.j('POST',`/api/chats/${fxt.cid}/jobs/retry`);
const exportOf=async fxt=>(await fxt.j('GET',`/api/chats/${fxt.cid}/export`)).conversations[0];

// Nothing from the current turn (seq 2) was promoted: summary, note, facts, knowledge, scene, episodes and relationships.
async function assertNothingPromoted(fxt,memory){
  assert.deepEqual(memory.memories,[]);
  const data=await exportOf(fxt);
  assert.equal(data.interaction_notes.length,1,'só a nota do turno de abertura; o turno bloqueado não cria resumo');
  assert.equal(data.interaction_notes.find(n=>n.seq===2),undefined);
  for(const table of ['memories','knowledge_events','memory_knowledge','episodes','relationships','threads'])assert.deepEqual(data[table]||[],[],table);
  assert.ok(!(data.scene_states||[]).some(s=>s.seq===2),'sem cena do turno bloqueado');
  assert.ok(!JSON.stringify({...data,versions:undefined,interactions:undefined}).includes(f.wrongFact),'a falsa acusação não aparece em nenhuma tabela derivada');
  assert.deepEqual((await fxt.j('GET',`/api/chats/${fxt.cid}`)).history.map(m=>({role:m.role,content:m.content})),f.messages,'fontes e histórico preservados');
  return data;
}
// After the valid retry: only Mira's faithful fact, the correct summary and the knowledge tied to that fact.
async function assertRecovered(fxt){
  const memory=await wait(async()=>{const m=await memoryOf(fxt);return m.sync?.synced&&m.memories?.length&&!m.jobs.failed&&!m.jobs.pending&&!m.jobs.running?m:null;});
  assert.deepEqual(memory.memories.map(m=>m.content),[f.correctFact]);
  const data=await exportOf(fxt);
  const correctId=data.memories.find(m=>m.content===f.correctFact).id;
  assert.ok(data.knowledge_events.length>0&&data.knowledge_events.every(k=>k.memory_id===correctId));
  assert.equal(data.interaction_notes.find(n=>n.seq===2).summary,f.summary);
  assert.ok(!JSON.stringify({...data,versions:undefined,interactions:undefined}).includes(f.wrongFact));
  return {memory,data};
}

function assertRequests(requests,sourceId){
  const currentReview=requests.find(b=>b.response_format?.json_schema?.schema?.properties?.agency&&b.messages.at(-1).content.includes(currentUser));
  assert.ok(currentReview,'real local review request');
  const text=currentReview.messages.at(-1).content;
  assert.ok(text.includes(previousBot)&&text.includes(currentUser)&&text.includes(falseBot));
  assert.ok(text.includes(originalSource(sourceId)),'exact source ID, assistant role and complete Unicode original evidence');
  assert.ok(text.includes(quoteProvenance),'exact quote attributed to Mira, no invented citing speaker, comments kept separate');
  assert.ok(currentReview.response_format.json_schema.schema.properties.attribution);
  // The same schema fragment in the real request and in the Rust contract (fixture shared with provenance_contracts.rs).
  assert.deepEqual(normalizeReviewSchema(currentReview.response_format.json_schema.schema),{topLevelRequired:reviewAttributionContract.topLevelRequired,attribution:reviewAttributionContract.attribution});
  for(const b of requests.filter(b=>b.messages)){
    assert.ok(!/\{\{\s*persona\s*\}\}/i.test(b.messages[0].content),'system persona placeholder replaced');
    assert.ok(b.messages[0].content.includes('Alex'),'actual persona in local system prompt');
    const schema=b.response_format.json_schema.schema;
    if(schema.properties.facts){
      const fact=schema.properties.facts.items;
      assert.deepEqual(fact.properties.attribution.required,['quote','originalSpeaker']);
      assert.ok(!fact.required.includes('attribution'));
      assert.ok(b.messages[0].content.includes('ROLE=user é a fonte, não um rótulo de falante'));
    }
  }
  return currentReview;
}

test('runtime Mira/Alex: revisão local documentada bloqueia antes da extração e preserva Unicode',async t=>{
  const fxt=await fixture(t,'flag');
  const memory=await wait(async()=>{const m=await fxt.j('GET',`/api/chats/${fxt.cid}/memory`);return m.flags?.length&&m.jobs?.blocked?m:null;});
  assertRequests(fxt.requests,fxt.sourceId);
  assert.equal(memory.flags.length,1);assert.equal(memory.flags[0].kind,'continuity');
  assert.equal(memory.flags[0].evidence,f.attributionFlag.quote);
  assert.equal(memory.memories.length,0);
  assert.ok(!fxt.requests.some(b=>b.response_format?.json_schema?.schema?.properties?.facts&&b.messages.at(-1).content.includes(currentUser)),'blocked before extraction');
  const data=(await fxt.j('GET',`/api/chats/${fxt.cid}/export`)).conversations[0];
  assert.equal(data.interaction_notes.length,1,'blocked turn creates no summary');
  assert.deepEqual((await fxt.j('GET',`/api/chats/${fxt.cid}`)).history.map(m=>({role:m.role,content:m.content})),f.messages);
  await evidence('blocked',{requests:fxt.requests,memory,sourceId:fxt.sourceId});
});

// Before: the runner applied the filtered batch (Mira's fact + correct summary). Now any fact rejected for
// documented authorship blocks the WHOLE turn batch (the summary generated with it may be contaminated); the job stays
// recoverable (failed after the attempts) and the retry with valid JSON applies only Mira's faithful fact.
test('runtime Mira/Alex: fato de autoria invertida bloqueia o lote mesmo com resumo correto; retry válido aplica só o fato de Mira',async t=>{
  const fxt=await fixture(t,'clean');
  const blocked=await wait(async()=>{const m=await memoryOf(fxt);return m.jobs?.failed===1&&!m.jobs.pending&&!m.jobs.running?m:null;});
  assertRequests(fxt.requests,fxt.sourceId);
  assert.equal(blocked.flags.length,0);
  assert.ok(extractionCalls(fxt.requests).length>=3,'o job tentou de novo até esgotar as tentativas; nenhuma promovida');
  const extract=extractionCalls(fxt.requests)[0];
  assert.ok(extract.messages.at(-1).content.includes(previousBot)&&extract.messages.at(-1).content.includes(`id=${fxt.sourceId} role=assistant`));
  await assertNothingPromoted(fxt,blocked);
  const status=await fxt.j('GET','/api/status');
  assert.match(status.jobs.lastError,/conflito de autoria/);assert.ok(status.jobs.lastError.includes('Alex')&&status.jobs.lastError.includes('Mira'));
  assert.ok(!status.jobs.lastError.includes(f.wrongFact),'o erro não repete a falsa acusação');
  // retry with valid JSON (no inverted attribution)
  fxt.ctl.extraction=validExtraction;
  assert.equal(await retry(fxt),1);
  const {memory,data}=await assertRecovered(fxt);
  assert.equal(memory.flags.length,0);
  assert.deepEqual((await fxt.j('GET',`/api/chats/${fxt.cid}`)).history.map(m=>({role:m.role,content:m.content})),f.messages);
  await evidence('extract-guard',{requests:fxt.requests,blocked,memory,sourceId:fxt.sourceId,notes:data.interaction_notes,knowledge:data.knowledge_events});
});

test('runtime Mira/Alex: resumo explicitamente falso + fato invertido não grava memória, conhecimento, nota nem episódio e é recuperável',async t=>{
  const fxt=await fixture(t,'clean',{extraction:contaminatedExtraction});
  const blocked=await wait(async()=>{const m=await memoryOf(fxt);return m.jobs?.failed===1&&!m.jobs.pending&&!m.jobs.running?m:null;});
  assertRequests(fxt.requests,fxt.sourceId);
  assert.ok(extractionCalls(fxt.requests).every(b=>b.messages.at(-1).content.includes(currentUser)));
  const data=await assertNothingPromoted(fxt,blocked);
  assert.ok(!JSON.stringify(data).includes('Alex chamou Mira de carteira'),'resumo contaminado não foi gravado em lugar nenhum');
  assert.equal((await fxt.j('GET','/api/status')).jobs.lastError.includes('conflito de autoria'),true);
  // the raw history (including the bot's incorrect reply) stays intact; nothing is rewritten automatically
  assert.ok((await fxt.j('GET',`/api/chats/${fxt.cid}`)).history.some(m=>m.content===falseBot));
  // retry: still contaminated stays blocked and recoverable; only valid JSON applies
  const before=extractionCalls(fxt.requests).length;
  assert.equal(await retry(fxt),1);
  await wait(async()=>{const m=await memoryOf(fxt);return m.jobs?.failed===1&&!m.jobs.pending&&!m.jobs.running&&extractionCalls(fxt.requests).length>before?m:null;});
  await assertNothingPromoted(fxt,await memoryOf(fxt));
  fxt.ctl.extraction=validExtraction;
  assert.equal(await retry(fxt),1);
  const {data:fixed}=await assertRecovered(fxt);
  assert.ok(!JSON.stringify(fixed).includes('Alex chamou Mira de carteira'));
  await evidence('contaminated-summary',{requests:fxt.requests,blocked,notes:fixed.interaction_notes,knowledge:fixed.knowledge_events});
});

test('runtime Mira/Alex: nova tentativa automática do job com JSON válido aplica sem intervenção',async t=>{
  let calls=0;
  const fxt=await fixture(t,'clean',{extraction:()=>++calls===1?contaminatedExtraction():validExtraction()});
  const {memory}=await assertRecovered(fxt);
  assert.equal(extractionCalls(fxt.requests).length,2,'primeira tentativa contaminada descartada; segunda aplicada');
  assert.equal(memory.jobs.failed||0,0);
});

test('runtime Mira/Alex: lote válido sem metadado attribution não bloqueia (sem falso positivo)',async t=>{
  const fxt=await fixture(t,'clean',{extraction:legacyExtraction});
  const {memory}=await assertRecovered(fxt);
  assert.equal(extractionCalls(fxt.requests).length,1,'aplicado na primeira tentativa');
  assert.equal(memory.jobs.failed||0,0);
  assert.doesNotMatch((await fxt.j('GET','/api/status')).jobs.lastError||'',/conflito de autoria/);
});

// Unsupported authorship never becomes a flag. A schema-invalid review fails recoverably before
// extraction; only a valid review on retry lets that turn proceed.
for(const [name,review,invalidSchema=false] of [
  ['previousQuote ausente',()=>{const r=attributionReview();delete r.attribution[0].previousQuote;return r;},true],
  ['claimedSpeaker em branco',()=>{const r=attributionReview();r.attribution[0].claimedSpeaker='   ';return r;}],
  ['previousQuote como array coagível',()=>{const r=attributionReview();r.attribution[0].previousQuote=[r.attribution[0].previousQuote];return r;},true],
  ['originalSpeaker forjado',()=>{const r=attributionReview();r.attribution[0].originalSpeaker='Nora';return r;}],
])test(`runtime Mira/Alex: revisão de autoria com ${name} não bloqueia o turno`,async t=>{
  const originalFlag=structuredClone(f.attributionFlag);
  const expectedReview=review();
  assert.deepEqual(f.attributionFlag,originalFlag,'invalid review does not mutate the shared fixture');
  let reviewCalls=0;
  const fxt=await fixture(t,'flag-incompleta',{review:()=>{reviewCalls++;return expectedReview;},extraction:validExtraction});
  await wait(()=>fxt.requests.some(b=>b.response_format?.json_schema?.schema?.properties?.agency&&hasReviewProvenance(b.messages.at(-1).content,fxt.sourceId)));
  assertRequests(fxt.requests,fxt.sourceId);
  if(invalidSchema){
    const failed=await wait(async()=>{const m=await memoryOf(fxt);return m.jobs?.failed===1&&!m.jobs.pending&&!m.jobs.running?m:null;});
    assert.equal(failed.flags.length,0);assert.equal(failed.jobs.blocked||0,0);
    assert.ok(reviewCalls>=3,'schema-invalid review exhausts the normal attempts');
    assert.equal(extractionCalls(fxt.requests).length,0,'schema-invalid review never reaches extraction');
    await assertNothingPromoted(fxt,failed);
    assert.match((await fxt.j('GET','/api/status')).jobs.lastError,/incompatível com o schema; retry necessário/);
    fxt.ctl.review=noFlags;
    assert.equal(await retry(fxt),1);
  }
  const {memory}=await assertRecovered(fxt);
  assert.equal(memory.flags.length,0);assert.equal(memory.jobs.blocked||0,0);
  assert.equal(extractionCalls(fxt.requests).length,1,'valid batch applies exactly once');
  if(!invalidSchema)assert.equal(reviewCalls,1,'unsupported authorship is ignored on the first review');
  await evidence(`invalid-review-${invalidSchema?'schema':'authorship'}-${name}`,{memory,reviewCalls,sourceId:fxt.sourceId});
});
