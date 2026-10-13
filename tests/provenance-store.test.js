import test from 'node:test';
import assert from 'node:assert/strict';
import { tempStore, newConv, turn } from './helpers.js';
import { mira as f, previousBot, currentUser, falseBot } from './provenance-fixture.js';

// Quote provenance in the Rust Store. Quote location (provenance::quotes) is verified in
// server-rust/tests/provenance_contracts.rs; here the sources returned by the Store are checked by ID and text.
// Authorship conflict in the supervisor (recoverable failure): tests/provenance-runtime.test.js.
const sourcesOf = (pairs) => pairs.flatMap((p) => p.sources || []);
// Source (assistant) containing the quoted sentence, as the origin lookup does over the Store sources.
const sourceOf = (sources, quote) => sources.find((x) => x.role === 'assistant' && String(x.text || '').includes(quote));

test('store proveniência: partes, seleção, refazer e replay conservam IDs individuais', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const first = await turn(t.store,c.id,'Na Diretoria.',previousBot,{opId:'original'});
    const second = await turn(t.store,c.id,null,`Nora: ${f.secondQuote}`,{mode:'continue',opId:'second'});
    const replacement = await turn(t.store,c.id,null,'Nora: O caderno sumiu.',{mode:'regenerate',opId:'replace'});
    let d = await t.store.call('getContextData',{convId:c.id,pendingUserText:currentUser});
    assert.deepEqual(d.pairs[0].sources.map(s=>s.id),[first.interactionId,first.versionId,replacement.versionId]);
    assert.equal(d.pairs[0].assistant,`${previousBot}\n\nNora: O caderno sumiu.`);
    assert.equal(sourceOf(sourcesOf(d.pairs),f.quote).id,first.versionId);
    await t.store.call('selectVersion',{convId:c.id,partId:second.partId,versionId:second.versionId});
    d = await t.store.call('getContextData',{convId:c.id,pendingUserText:currentUser});
    const picked = sourcesOf(d.pairs);
    assert.deepEqual([sourceOf(picked,f.quote).id,sourceOf(picked,f.secondQuote).id],[first.versionId,second.versionId]);
    const replay = await t.store.call('beginTurn',{convId:c.id,mode:'send',content:'Na Diretoria.',opId:'original'});
    assert.equal(replay.replay,true);
    assert.deepEqual((await t.store.call('getContextData',{convId:c.id,pendingUserText:currentUser})).pairs[0].sources,d.pairs[0].sources);
  } finally { await t.cleanup(); }
});

test('store proveniência: getTurnInput leva fontes anteriores intactas, sem futuro ou versão não selecionada', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const first = await turn(t.store,c.id,'Na Diretoria.',previousBot);
    const current = await turn(t.store,c.id,currentUser,falseBot);
    await turn(t.store,c.id,'FUTURO_NÃO_ENTRAR','Mira: FUTURO_NÃO_ENTRAR');
    const job = await t.store.call('enqueueJob',{convId:c.id,interactionId:current.interactionId,versionId:current.versionId,kind:'turn'});
    const input = await t.store.call('getTurnInput',job.id);
    assert.equal(input.userText,currentUser);
    assert.equal(input.assistantText,falseBot);
    assert.ok(input.previous.includes(previousBot) && input.previous.includes(`id=${first.versionId} role=assistant`));
    assert.ok(input.text.includes(`id=${current.interactionId} role=user`) && input.text.includes(`id=${current.versionId} role=assistant`));
    assert.ok(!input.previous.includes('FUTURO_NÃO_ENTRAR'));
    assert.equal(sourceOf(input.recentSources,f.quote).id,first.versionId);
    const d = await t.store.call('getContextData',{convId:c.id,interactionId:current.interactionId});
    assert.deepEqual(d.pairs.map(p=>p.id),[first.interactionId]);
    const branch = await turn(t.store,c.id,'Outro caminho','Mira: VERSÃO_OUTRO_RAMO',{mode:'edit',targetId:first.interactionId});
    const selected = await t.store.call('getContextData',{convId:c.id,pendingUserText:currentUser});
    assert.deepEqual(selected.pairs.map(p=>p.id),[branch.interactionId]);
    assert.equal(sourceOf(sourcesOf(selected.pairs),f.quote),undefined,'unselected original branch does not supply quote origins');
    assert.equal((await t.store.call('getTurnInput',job.id)).gone,true);
    assert.ok(branch.interactionId!==first.interactionId);
  } finally { await t.cleanup(); }
});

test('store proveniência: acima de oito continuações o histórico e todas as fontes permanecem no Store', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const first = await turn(t.store,c.id,'Na Diretoria.',previousBot);
    for (let i=0;i<9;i++) await turn(t.store,c.id,null,`Mira: Continuação ${i}.`,{mode:'continue'});
    const d = await t.store.call('getContextData',{convId:c.id,pendingUserText:currentUser});
    assert.equal(d.pairs[0].sources.length,11);
    assert.ok(d.pairs[0].assistant.startsWith(previousBot));
    assert.ok(d.pairs[0].sources.some(s=>s.id===first.versionId));
    // the budget of eight continuations for quote location is verified in provenance_contracts.rs
  } finally { await t.cleanup(); }
});
