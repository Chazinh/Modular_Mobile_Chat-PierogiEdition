// Same fixture and same queries on the Rust store (mc-contract): wall-clock p50/p95; no inference, personal database or RAM.
// To compare two executables (before/after an optimization) use scripts/compare-context-golden.mjs and repeat with MC_CONTRACT_EXE.
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { createHash,randomUUID } from 'node:crypto';
import { RustStore } from '../tests/rust-store.js';
const out=process.env.MC_BENCHMARK_OUT || path.join(os.tmpdir(),'mc-context-benchmark.json');
const fixtureDir=process.env.MC_BENCHMARK_FIXTURE_DIR;
const rustExecutable=process.env.MC_CONTRACT_EXE||path.resolve('server-rust/target/debug/mc-contract.exe');
const percentile=(a,p)=>a.slice().sort((x,y)=>x-y)[Math.ceil(a.length*p)-1];
const results=[],fixtureHashes={};
for(const count of [100,720,2000]){
  let fixture;
  const fixtureFile=fixtureDir?path.join(fixtureDir,`context-${count}.json`):null;
  if(fixtureFile){try{fixture=JSON.parse(await fs.readFile(fixtureFile,'utf8'));}catch(e){if(e.code!=='ENOENT')throw e;}}
  if(!fixture){
  const seedDir=await fs.mkdtemp(path.join(os.tmpdir(),'mc-bench-seed-'));const seed=new RustStore(path.join(seedDir,'seed.db'));await seed.ready();
  const c=await seed.call('createConversation',{name:'Benchmark fictício',persona:'Alex',routeId:'cbai/deepseek-v4.1-flash',presetId:'deepseek-v41-flash',effort:'high',prompt:'Lia e Laura estudam magia.'});
  fixture=await seed.call('exportConversation',c.id);await seed.close();await fs.rm(seedDir,{recursive:true,force:true});
  fixture.interactions=[];fixture.parts=[];fixture.versions=[];fixture.interaction_notes=[];
  let parent=null;const date='2026-01-01T00:00:00.000Z';
  for(let seq=1;seq<=count;seq++){
    const iid=randomUUID(),pid=randomUUID(),vid=randomUUID();
    const text=`Lia: No turno ${seq}, a chave dourada continua guardada no jardim.\n*Ela pesquisa a carta azul.* `+('Descrição fictícia para contexto. '.repeat(12));
    fixture.interactions.push({id:iid,conv_id:c.id,parent_id:parent,seq,kind:'turn',user_text:`Examino a carta do turno ${seq}.`,user_ts:date,created_at:date});
    fixture.parts.push({id:pid,interaction_id:iid,idx:0,selected_version_id:vid,created_at:date});
    fixture.versions.push({id:vid,part_id:pid,content:text,status:'complete',origin:'legacy',created_at:date,finished_at:date});
    // Leave old pending turns outside the 50-pair window on purpose.
    if(seq!==2 && seq!==7)fixture.interaction_notes.push({interaction_id:iid,conv_id:c.id,seq,summary:`Turno ${seq}: Lia examina a carta.`,version_ids_json:JSON.stringify([vid]),created_at:date});
    parent=iid;
  }
  fixture.conversation.head_interaction_id=parent;
  if(fixtureFile){await fs.mkdir(fixtureDir,{recursive:true});await fs.writeFile(fixtureFile,JSON.stringify(fixture),{flag:'wx'});}
  }
  fixtureHashes[count]=createHash('sha256').update(JSON.stringify(fixture)).digest('hex');
  for(const name of ['rust']){
    const dir=await fs.mkdtemp(path.join(os.tmpdir(),'mc-bench-'));const store=new RustStore(path.join(dir,'test.db'));await store.ready();
    try{
      const imported=await store.call('importConversation',fixture);const input={convId:imported.id,pendingUserText:'Onde está a chave dourada?',maxPairs:50};
      const first=performance.now();const context=await store.call('getContextData',input);const cold=performance.now()-first;
      if(context.pairs.length!==50||context.unprocessed.length!==2)throw new Error(`Benchmark invalid fixture ${name}/${count}: ${context.pairs.length}/${context.unprocessed.length}`);
      for(let i=0;i<5;i++)await store.call('getContextData',input);
      const samples=[];for(let i=0;i<40;i++){const start=performance.now();await store.call('getContextData',input);samples.push(performance.now()-start);}
      results.push({backend:name,turns:count,fixtureHash:fixtureHashes[count],query:input.pendingUserText,recent:context.pairs.length,pending:context.unprocessed.length,coldMs:cold,iterations:samples.length,p50Ms:percentile(samples,.5),p95Ms:percentile(samples,.95),samplesMs:samples});
      console.log(`${name} ${count}: cold=${cold.toFixed(2)} p50=${percentile(samples,.5).toFixed(2)} p95=${percentile(samples,.95).toFixed(2)} ms`);
    }finally{await store.close();await fs.rm(dir,{recursive:true,force:true});}
  }
}
await fs.writeFile(out,JSON.stringify({date:new Date().toISOString(),node:process.version,rustExecutable,rustProfile:/[\\/]release[\\/]/.test(rustExecutable)?'release':/[\\/]debug[\\/]/.test(rustExecutable)?'debug':'custom',metric:'wall-clock store context including IPC and JSON',fixtureHashes,results,limits:'40 warm samples; first includes derived timeline. No RAM/VRAM/device/inference measurement.'},null,2));
