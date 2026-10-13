import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import path from 'node:path';
import { tempStore, newConv, turn } from './helpers.js';

// Narrative time derivation and the conservative review are verified in server-rust/tests/contracts.rs.
// Here are the Store guarantees: timeline scope and persisted cache.
test('timeline vazia e primeiro envio não acessam anchors fora do escopo',async()=>{
  const t=await tempStore();try{
    const c=await newConv(t.store,{prompt:'Eles chegaram há 3 meses. Ela nasceu 3 meses atrás.'});
    const d=await t.store.call('getContextData',{convId:c.id,pendingUserText:'Oi'});
    assert.equal(d.timeline.current,null);assert.equal(d.timeline.anchors.length,2);
    await turn(t.store,c.id,'Dia seguinte.','Mira: Bom dia.');
    assert.equal((await t.store.call('memoryOverview',c.id)).timeline.current.days,1);
  }finally{await t.cleanup();}
});
test('cache persistido respeita cutoff, versões, ramo e não grava outro snapshot no GET igual',async()=>{
  const t=await tempStore();try{
    const c=await newConv(t.store);const a=await turn(t.store,c.id,'Dia seguinte.','Lia: Bom dia.');
    const b=await turn(t.store,c.id,'Sete dias depois.','Lia: Oi.');
    let d=await t.store.call('memoryOverview',c.id);assert.equal(d.timeline.current.days,8);
    const db=new DatabaseSync(path.join(t.dir,'modularchat.db'));const count=()=>db.prepare('SELECT count(*) n FROM narrative_snapshots').get().n;
    const before=count();await t.store.call('memoryOverview',c.id);assert.equal(count(),before);db.close();
    d=await t.store.call('getContextData',{convId:c.id,interactionId:b.interactionId});assert.equal(d.timeline.current.days,1);
    const branch=await turn(t.store,c.id,'Dia seguinte.','Lia: Novo ramo.',{mode:'edit',targetId:a.interactionId});
    assert.equal((await t.store.call('memoryOverview',c.id)).timeline.current.days,1);
    await turn(t.store,c.id,null,'Sete dias depois. Novo cenário.',{mode:'regenerate'});
    assert.equal((await t.store.call('memoryOverview',c.id)).timeline.current.days,8);
    const versions=await t.store.call('listVersions',{convId:c.id,partId:branch.partId});
    await t.store.call('selectVersion',{convId:c.id,partId:branch.partId,versionId:versions[0].id});
    assert.equal((await t.store.call('memoryOverview',c.id)).timeline.current.days,1);
  }finally{await t.cleanup();}
});
