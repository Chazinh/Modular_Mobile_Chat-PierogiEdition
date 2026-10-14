// Encrypted vault (MCVAULT1 + DPAPI on Windows / private sandbox key elsewhere): the contract executable creates the vault, the Rust server opens it,
// and the same master key reopens the vault later. Node is only the fixture driver.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {RustStore} from './rust-store.js';
import {newConv,turn,mockRouter} from './helpers.js';
import {startRust} from './rust-http.js';

const keySuffix=process.platform==='win32'?'.key':'.sandbox.key';

test('cofre Rust: dados e pareamento preservados, cookie/cache/export/ficha/backup protegidos e reabertura com a mesma chave',async()=>{
  const dir=await fs.mkdtemp(path.join(os.tmpdir(),'mc-rust-private-')),file=path.join(dir,'modularchat.db');
  const previousMode=process.env.MC_ENCRYPTED_STORAGE;process.env.MC_ENCRYPTED_STORAGE='1';
  let store=new RustStore(file),runtime,upstream;
  try {
    await store.ready();const c=await newConv(store,{prompt:'FICHA_PRIVADA_UM',routeId:'cc/claude-sonnet-5-5',presetId:'claude-sonnet-5-5',effort:'low'});
    await turn(store,c.id,'USUARIO_PRIVADO_UM','Lia: BOT_PRIVADO_UM.');
    await store.call('setGlobalSettings',{botChars:1500,dialoguePct:40});
    await store.close();store=null;
    assert.equal(await fs.stat(file).then(()=>true,()=>false),false,'nenhum SQLite em texto claro');
    const beforeKey=await fs.readFile(file+keySuffix);
    const vault=await fs.readFile(file+'.vault');assert.ok(!vault.includes(Buffer.from('USUARIO_PRIVADO_UM')),'o cofre não guarda texto claro');
    upstream=await mockRouter();runtime=await startRust({dataDir:dir,upstream:upstream.url,review:false});
    let token=null;
    const j=async(method,p,body,headers=token?{Authorization:`Bearer ${token}`}:{})=>{
      const res=await fetch(runtime.base+p,{method,headers:{...headers,'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(15000)});
      return {status:res.status,body:await res.json(),headers:res.headers};
    };
    assert.equal((await j('GET','/api/chats',undefined,{})).status,401);
    assert.equal((await j('GET','/api/privacy/cache-key',undefined,{})).status,401);
    const paired=await j('POST','/api/pair',{code:runtime.pairingCode,deviceName:'PC fixture'},{});
    assert.equal(paired.status,201);const cookie=paired.headers.get('set-cookie');assert.ok(cookie.includes('HttpOnly')&&cookie.includes('SameSite=Strict'));
    token=paired.body.token;assert.ok(token);
    assert.equal((await j('GET','/api/chats',undefined,{Cookie:cookie.split(';')[0]})).status,200);
    const chat=await j('GET',`/api/chats/${c.id}`);assert.equal(chat.status,200);
    assert.ok(chat.body.history.some(m=>m.content.includes('BOT_PRIVADO_UM')));assert.equal(chat.body.effort,'low');
    const cache=await j('GET','/api/privacy/cache-key');assert.equal(typeof cache.body.key,'string');assert.ok(cache.body.key.length>=32);assert.equal(cache.headers.get('cache-control'),'no-store');
    const bot=chat.body.history.find(m=>m.role==='assistant');
    assert.equal((await j('PUT',`/api/chats/${c.id}/messages/${bot.id}`,{content:'Lia: CORRECAO_PRIVADA_RUST.'})).status,200);
    assert.equal((await j('PUT',`/api/chats/${c.id}`,{prompt:'FICHA_PRIVADA_RUST'})).status,200);
    assert.equal(await fs.stat(path.join(dir,'chats',c.id,'character.txt')).then(()=>true,()=>false),false);
    const card=await fs.readFile(path.join(dir,'chats',c.id,'character.txt.sealed'));assert.ok(!card.includes(Buffer.from('FICHA_PRIVADA_RUST')));
    const exported=await j('GET','/api/export');assert.equal(exported.body.format,'modularchat-encrypted-export');
    assert.ok(!JSON.stringify(exported.body).includes('CORRECAO_PRIVADA_RUST'));
    const imported=await j('POST','/api/import',exported.body);assert.equal(imported.status,201);assert.equal(imported.body.imported.length,1);
    const backup=await j('POST','/api/backup');assert.equal(backup.status,200);
    assert.ok(!(await fs.readFile(backup.body.file)).includes(Buffer.from('CORRECAO_PRIVADA_RUST')));
    assert.equal(await fs.stat(file).then(()=>true,()=>false),false);
    await runtime.close();runtime=null;
    assert.deepEqual(await fs.readFile(file+keySuffix),beforeKey,'master key remains unchanged');
    store=new RustStore(file);await store.ready();
    const reopened=await store.call('getConversation',c.id);
    assert.equal(reopened.prompt,'FICHA_PRIVADA_RUST');assert.ok(reopened.history.some(m=>m.content.includes('CORRECAO_PRIVADA_RUST')));
    const after=await store.call('getGlobalSettings');assert.equal(after.botChars,1500);assert.equal(after.dialoguePct,40);
    const devices=await store.call('listDevices');assert.equal(devices.length,1);
    assert.equal(devices[0].name,'PC fixture');assert.notEqual(devices[0].token_hash,token,'o token em claro não é guardado');
    await store.close();store=null;
    // pairing preserved and the same master key: another start accepts the same token and returns the same cache key
    runtime=await startRust({dataDir:dir,upstream:upstream.url,review:false});
    const again=await fetch(runtime.base+'/api/privacy/cache-key',{headers:{Authorization:`Bearer ${token}`}});
    assert.equal(again.status,200);assert.equal((await again.json()).key,cache.body.key);
  } finally {
    if(previousMode===undefined)delete process.env.MC_ENCRYPTED_STORAGE;else process.env.MC_ENCRYPTED_STORAGE=previousMode;
    if(runtime)await runtime.close();if(store)await store.close();if(upstream)await upstream.close();await fs.rm(dir,{recursive:true,force:true});
  }
});

// Vault persistence: what the client sees as confirmed is written before the ack (survives a process killed by
// force); derived state goes in batches and is flushed on a clean shutdown.
test('cofre Rust: ack durável sobrevive a processo morto, derivado é descarregado no encerramento e gravações caem por turno',async()=>{
  const dir=await fs.mkdtemp(path.join(os.tmpdir(),'mc-rust-durable-')),file=path.join(dir,'modularchat.db');
  const previousMode=process.env.MC_ENCRYPTED_STORAGE;process.env.MC_ENCRYPTED_STORAGE='1';
  let store=new RustStore(file);
  try {
    await store.ready();
    const c=await newConv(store);
    const base=await store.call('persistStats');
    for(let i=0;i<5;i++){
      const b=await store.call('beginTurn',{convId:c.id,opId:`op-${i}`,mode:'send',content:`Pergunta ${i}`,routeId:'cbai/deepseek-v4.1-flash',presetId:'deepseek-v41-flash',effort:'high'});
      await store.call('flushVersion',b.versionId,'Lia: parcial');await store.call('flushVersion',b.versionId,'Lia: parcial');
      await store.call('finishGeneration',{generationId:b.generationId,text:`Lia: resposta ${i}.`,status:'complete',usage:{},flags:[]});
      const job=await store.call('claimNextJob');if(job)await store.call('finishJob',job.id,'done');
    }
    const now=await store.call('persistStats');
    const changing=now.changingRequests-base.changingRequests,saves=now.saves-base.saves;
    assert.ok(changing>=25,`o cenário altera várias vezes por turno: ${changing}`);
    assert.ok(saves<=12&&saves*2<=changing,`gravações por turno devem cair (antes: uma por mudança): ${saves} de ${changing}`);
    // Accepts a message and kills the PID this test started, without any flush.
    const pending=await store.call('beginTurn',{convId:c.id,opId:'op-aceita',mode:'send',content:'MENSAGEM_ACEITA_ANTES_DO_KILL',routeId:'cbai/deepseek-v4.1-flash',presetId:'deepseek-v41-flash',effort:'high'});
    assert.ok(pending.generationId);
    const exited=new Promise(resolve=>store.process.once('exit',resolve));
    store.process.kill();await exited;store=null;
    store=new RustStore(file);await store.ready();
    const reopened=await store.call('getConversation',c.id);
    assert.ok(reopened.history.some(m=>m.content&&m.content.includes('MENSAGEM_ACEITA_ANTES_DO_KILL')),'mensagem aceita sobrevive ao kill');
    assert.ok(reopened.history.some(m=>m.content&&m.content.includes('resposta 4')),'respostas concluídas sobrevivem ao kill');
    assert.deepEqual((await fs.readdir(dir)).filter(n=>n.startsWith('.mc-')),[],'nenhum temporário órfão');
    // Derived state (a claimed job) is flushed on a clean shutdown and does not come back as pending.
    await store.call('recoverInterrupted');
    const b2=await store.call('beginTurn',{convId:c.id,opId:'op-final',mode:'send',content:'Última',routeId:'cbai/deepseek-v4.1-flash',presetId:'deepseek-v41-flash',effort:'high'});
    await store.call('finishGeneration',{generationId:b2.generationId,text:'Lia: fim.',status:'complete',usage:{},flags:[]});
    let claimed;do{claimed=await store.call('claimNextJob');}while(claimed);
    await store.close();store=null;
    store=new RustStore(file);await store.ready();
    assert.equal(await store.call('claimNextJob'),null,'jobs reivindicados antes do encerramento limpo ficaram gravados');
    await store.close();store=null;
  } finally {
    if(previousMode===undefined)delete process.env.MC_ENCRYPTED_STORAGE;else process.env.MC_ENCRYPTED_STORAGE=previousMode;
    if(store)await store.close();await fs.rm(dir,{recursive:true,force:true});
  }
});
