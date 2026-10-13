import test from 'node:test';
import assert from 'node:assert/strict';
import { tempStore, newConv } from './helpers.js';

// Curation grouping and sanitizing: server-rust/tests/legacy_ports.rs.
// The job with the local model (one call, safe merges only) runs in the Rust supervisor: tests/rust-runtime.mjs.
const CURATION_MIN_ACTIVE = 60;
const add = (store, convId, m) => store.call('addMemory', convId, { visibility: 'public', ...m });

async function seed(store, convId, { distinct = 66, pairs = 4 } = {}) {
  const words = ['jardim', 'biblioteca', 'torre', 'lago', 'cozinha', 'estufa', 'ponte', 'porão', 'varanda', 'altar', 'galpão', 'mercado', 'oficina', 'cripta'];
  const ids = [];
  for (let i = 0; i < distinct; i++) {
    const r = await add(store, convId, { kind: 'WORLD_FACT', subject: `Lugar ${i}`, content: `${words[i % words.length]} número ${i} tem a marca ${i * 7} entalhada na porta ${i + 100}`, importance: 0.5 });
    ids.push(r.id);
  }
  const dup = [];
  for (let p = 0; p < pairs; p++) {
    const base = `Laura${p} escondeu o frasco${p} verde de poção${p} atrás do armário${p} da cozinha${p}`;
    const x = await add(store, convId, { kind: 'OBJECT', subject: `Laura${p}`, content: base, importance: 0.4 });
    const y = await add(store, convId, { kind: 'OBJECT', subject: `Laura${p}`, content: `${base} ontem`, importance: 0.8 });
    dup.push([x.id, y.id]);
  }
  return { ids, dup };
}

test('loja: só agenda curadoria com memórias demais e novas; grupos ignoram fixadas e privadas; aplicar é recuperável', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    assert.equal(await t.store.call('maybeQueueCuration', c.id), false, 'poucas memórias: nada a fazer');
    const { dup } = await seed(t.store, c.id);
    assert.ok(66 + dup.length * 2 >= CURATION_MIN_ACTIVE);
    // PINNED and PRIVATE duplicates never join groups
    const pin = await add(t.store, c.id, { kind: 'OBJECT', subject: 'Laura0', content: 'Laura0 escondeu o frasco0 verde de poção0 atrás do armário0 da cozinha0 hoje', importance: 0.9 });
    await t.store.call('updateMemory', c.id, pin.id, { pinned: true });
    const priv = await add(t.store, c.id, { kind: 'OBJECT', subject: 'Laura1', content: 'Laura1 escondeu o frasco1 verde de poção1 atrás do armário1 da cozinha1 de noite', importance: 0.9, visibility: 'private', knowers: ['Mira'] });

    assert.equal(await t.store.call('maybeQueueCuration', c.id), true);
    assert.equal(await t.store.call('maybeQueueCuration', c.id), false, 'já há uma curadoria pendente');
    const { clusters } = await t.store.call('curationCandidates', c.id);
    assert.equal(clusters.length, 4);
    const inClusters = clusters.flat().map((m) => m.id);
    assert.ok(!inClusters.includes(priv.id), 'privada/com conhecimento fica de fora');
    assert.ok(dup.every(([x, y]) => inClusters.includes(x) && inClusters.includes(y)));

    const [x0, y0] = dup[0];
    const merged = 'Laura0 escondeu o frasco0 verde de poção0 atrás do armário0 da cozinha0.';
    const out = await t.store.call('applyCuration', { convId: c.id, decisions: [{ keepId: y0, dropIds: [x0], merged }] });
    assert.equal(out.merged, 1);
    assert.equal(out.retired, 1);
    const overview = await t.store.call('memoryOverview', c.id);
    const ids = overview.memories.map((m) => m.id);
    assert.ok(ids.includes(y0) && !ids.includes(x0), 'a antiga saiu das memórias correntes');
    const kept = overview.memories.find((m) => m.id === y0);
    assert.equal(kept.content, merged);
    assert.ok(kept.importance >= 0.8, 'mantém a maior importância');
    assert.ok(!clusters.flat().some((m) => m.id === pin.id), 'fixada nunca entra em grupo');
    // after curating, it only schedules again with enough NEW memories
    await t.store.call('finishJob', (await t.store.call('claimNextJob')).id, 'done');
    assert.equal(await t.store.call('maybeQueueCuration', c.id), false);
    // recoverable: deleting the kept one brings the old one back as the current memory
    await t.store.call('deleteMemory', c.id, y0);
    assert.ok((await t.store.call('memoryOverview', c.id)).memories.some((m) => m.id === x0), 'substituída voltou a ser corrente');
  } finally {
    await t.cleanup();
  }
});
