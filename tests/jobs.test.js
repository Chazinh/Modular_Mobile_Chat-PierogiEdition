import test from 'node:test';
import assert from 'node:assert/strict';
import { tempStore, newConv, turn } from './helpers.js';

// The job supervisor (review, extraction, episodes, failure/retry, local model unavailable) runs inside the
// Rust server and is verified in tests/rust-runtime.mjs and tests/provenance-runtime.test.js.
// Here are the Store guarantees the supervisor relies on (episodes: rust-runtime.mjs, "episodes with full sources").
test('sinalização heurística (fala de Alex) bloqueia a extração desde o início', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', '*Mira sorri.*\n\nAlex: Oi, Mira.', { flags: [{ kind: 'agency', detail: 'Fala atribuída a Alex', evidence: 'Alex: Oi, Mira.' }] });
    const mem = await t.store.call('memoryOverview', c.id);
    assert.equal(mem.jobs.blocked, 1);
    const chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history[1].flags.length, 1);
    assert.equal(chat.history[1].flags[0].kind, 'agency');
  } finally {
    await t.cleanup();
  }
});

test('referência explícita ao passado reinjeta o texto original, não só o resumo', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Naquela noite você me disse algo sobre o farol abandonado.', 'Mira: O farol abandonado guarda o diário de meu pai, Alex.');
    for (let i = 0; i < 6; i++) await turn(t.store, c.id, `Assunto ${i}`, `Resposta ${i}`);
    const r = await t.store.call('retrieve', { convId: c.id, queryText: 'Você lembra exatamente o que me disse sobre o farol abandonado?', queryVec: null, entityKeys: [], presentKeys: [], limit: 8, windowStartSeq: 4 });
    assert.ok(r.raw.length >= 1);
    assert.match(r.raw[0].text, /farol abandonado/);
  } finally {
    await t.cleanup();
  }
});
