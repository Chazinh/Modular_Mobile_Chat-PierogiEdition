import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import path from 'node:path';
import { newConv, turn, tempStore } from './helpers.js';

// Long-term memory in the Rust Store (retrieval, per-branch knowledge, extraction validity).
// Context assembly and the 720-turn simulation live in server-rust/tests/legacy_ports.rs.
async function setup(t) {
  const fixture = await tempStore();
  const db = new DatabaseSync(path.join(fixture.dir, 'modularchat.db'));
  t.after(async () => { db.close(); await fixture.cleanup(); });
  return { db, store: fixture.store };
}
const ext = (facts=[],scene=null) => ({summary:'Turno processado.',cast:[],facts,scene,relationships:[],openThreads:[],resolvedThreads:[]});
const fact = (content,extra={}) => ({kind:'SECRET',subject:'Lia',content,importance:.9,confidence:1,visibility:'private',knowledge:[],...extra});
const apply = (store,c,b,extraction,sourceText='') => store.call('applyExtraction',{convId:c.id,interactionId:b.interactionId,versionId:b.versionId,versionIds:[b.versionId],extraction,sourceText});
const retrieve = (store,c,queryText,over={}) => store.call('retrieve',{convId:c.id,queryText,windowStartSeq:10000,...over});

test('ranking real congela pesos semântica/overlap/entidade/importância/idade',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  const a=await turn(store,c.id,'A chave?','Lia: Guardei no jardim.');
  const content='Lia guarda a chave dourada no jardim.';
  await apply(store,c,a,ext([fact(content,{kind:'FACT',importance:.7,visibility:'public'})]));
  await turn(store,c.id,'Outro turno','Lia: Uma carta.');await turn(store,c.id,'Mais um turno','Lia: Boa noite.');
  const query='chave dourada jardim';const data=await retrieve(store,c,query,{entityKeys:['lia']});
  const m=data.candidates.find(m=>m.type==='memory'&&m.content===content);assert.ok(m);
  // the query words {chave,dourada,jardim} are all in "Lia guarda a chave dourada no jardim": overlap = 3/min(3,5) = 1
  const overlap=1;
  const expected=.48*m.parts.sem+.22*overlap+.15*1+.1*.7+.05/(1+2/60);
  assert.ok(Math.abs(m.score-expected)<1e-12,JSON.stringify({score:m.score,expected}));
});

test('revelação posterior atualiza conhecimento, mas edição para outro ramo reverte e exportação preserva',async t=>{
  const {store}=await setup(t); const c=await newConv(store);
  const content='Lia escondeu a chave dourada no porão.';
  const a=await turn(store,c.id,'Vamos começar','Lia: Ninguém pode saber.');
  await apply(store,c,a,ext([fact(content,{knowledge:[{who:'Laura',state:'unaware'}]})]));
  const b=await turn(store,c.id,'Conto a Laura onde Lia escondeu a chave.','Laura: Agora eu sei.');
  await apply(store,c,b,ext([fact(content,{knowledge:[{who:'Laura',state:'knows',via:'told',evidence:'Conto a Laura onde Lia escondeu a chave'}]})]),'Conto a Laura onde Lia escondeu a chave.');
  const knows=async conv=>(await retrieve(store,conv,'chave dourada')).candidates[0].knowledge.find(k=>k.name==='Laura')?.state;
  assert.equal(await knows(c),'knows');
  const copy=await store.call('importConversation',await store.call('exportConversation',c.id));
  assert.equal(await knows(copy),'knows');
  await turn(store,c.id,'Não conto o segredo.','Laura: Que dia bonito.',{mode:'edit',targetId:b.interactionId});
  assert.equal(await knows(c),'unaware');
  await store.call('activateBranch',{convId:c.id,interactionId:b.interactionId});
  assert.equal(await knows(c),'knows');
});

test('mudança pequena de objeto substitui estado; sujeitos diferentes não são duplicatas',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  const old='Lia guardou o casaco vermelho dentro do armário.';
  const a=await turn(store,c.id,'a','Lia: Vermelho.');
  await apply(store,c,a,ext([fact(old,{kind:'OBJECT'})]));
  const b=await turn(store,c.id,'b','Lia: Azul.');
  const changed='Lia guardou o casaco azul dentro do armário.';
  await apply(store,c,b,ext([fact(changed,{kind:'OBJECT',replaces:old}),fact(changed,{kind:'OBJECT',subject:'Laura'})]));
  const m=(await store.call('memoryOverview',c.id)).memories;
  assert.equal(m.length,2);assert.ok(m.every(x=>x.content===changed));
});

test('recupera fato lexical em corpus mínimo e trecho do fim de mensagem antiga sem palavra gatilho',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  const a=await turn(store,c.id,'a','Ambiente. '.repeat(450)+'A senha é açafrão-violeta.');
  await apply(store,c,a,ext([fact('O cristal de açafrão-violeta abre o cofre.',{kind:'WORLD_FACT',subject:null,visibility:'public'})]));
  const r=await retrieve(store,c,'açafrão violeta');
  assert.equal(r.candidates.length,1);
  assert.ok(r.raw.some(x=>x.text.includes('A senha é açafrão-violeta.')));
});

test('candidatos recentes não expulsam antigos antes do filtro da janela',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  for(let i=1;i<=45;i++) {const b=await turn(store,c.id,`Turno ${i}`,`Lia: Assunto ${i}.`);await apply(store,c,b,ext([fact(`Lia recebeu o artefato número ${i}.`,{kind:'OBJECT'})]));}
  const r=await retrieve(store,c,'artefato',{entityKeys:['lia'],windowStartSeq:16});
  assert.equal(r.candidates.length,15);assert.ok(r.candidates.every(x=>x.sourceSeq<16));
});

test('vetores incompatíveis não participam da busca e voltam para reindexação',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  const m=await store.call('addMemory',c.id,{content:'A chave dourada está guardada.',kind:'WORLD_FACT'});
  await store.call('saveEmbedding',{ownerType:'memory',ownerId:m.id,convId:c.id,model:'old',vec:[1,0,0]});
  const r=await retrieve(store,c,'paráfrase sem palavras em comum',{queryVec:[1,0,0],embeddingModel:'new'});
  assert.equal(r.candidates.length,0);
  assert.ok((await store.call('pendingEmbeddings',16,'new')).some(x=>x.id===m.id));
});

test('extração atrasada de versão substituída é recusada sem apagar estado novo',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  const a=await turn(store,c.id,'a','Lia: Resposta antiga.');
  await turn(store,c.id,null,'Lia: Resposta nova.',{mode:'regenerate'});
  assert.equal((await apply(store,c,a,ext([fact('A resposta antiga ensinou um segredo.')]))).reason,'versions-changed');
});

test('edição antiga não recebe cena futura',async t=>{
  const {store}=await setup(t);const c=await newConv(store);
  const a=await turn(store,c.id,'Oi','Lia: SENHA_PRIMEIRO_TURNO');
  const b=await turn(store,c.id,'Mudamos de país','Lia: Paris.');
  await apply(store,c,b,ext([], {location:'Paris',present:['Lia'],positions:{},holding:{},locations:{}}));
  const old=await store.call('getContextData',{convId:c.id,interactionId:a.interactionId,overrideUserText:'Outro começo'});
  assert.equal(old.scene,null);
});

test('editar resposta antiga invalida notas dependentes e cancela extração já em andamento',async t=>{
  const {store,db}=await setup(t);const c=await newConv(store);
  const a=await turn(store,c.id,'Primeiro','Lia: O armário é azul.');
  await apply(store,c,a,ext([fact('O armário de Lia é azul.',{kind:'OBJECT'})]));
  const b=await turn(store,c.id,'Segundo','Laura: Eu vi o armário azul.');
  await apply(store,c,b,ext());
  const job=db.prepare("SELECT id FROM jobs WHERE interaction_id=? AND kind='turn'").get(b.interactionId);
  db.prepare("UPDATE jobs SET status='running' WHERE id=?").run(job.id);
  const part=db.prepare('SELECT id FROM parts WHERE interaction_id=?').get(a.interactionId);
  await store.call('editMessage',{convId:c.id,messageId:part.id,content:'Lia: O armário é vermelho.'});
  assert.equal(db.prepare('SELECT count(*) n FROM interaction_notes').get().n,0);
  const stale=await store.call('applyExtraction',{convId:c.id,interactionId:b.interactionId,versionIds:[b.versionId],jobId:job.id,extraction:ext([fact('Laura sabe do armário azul.')])});
  assert.equal(stale.reason,'job-invalidated');
  assert.equal((await store.call('getConversation',c.id)).history.length,4);
  assert.equal((await store.call('syncLag',c.id)).lag,2);
});
