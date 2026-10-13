import test from 'node:test';
import assert from 'node:assert/strict';
import { tempStore, newConv, turn } from './helpers.js';

const history = async (store, id) => (await store.call('getConversation', id)).history.map((m) => `${m.role}:${m.content}`);

async function chatWith(store, n = 4) {
  const c = await newConv(store);
  const turns = [];
  for (let i = 1; i <= n; i++) turns.push(await turn(store, c.id, `Pergunta ${i}`, `Mira: Resposta ${i}.`));
  return { c, turns };
}

test('lixeira: apagar mensagem guarda o conteúdo (com prévia) e restaurar devolve no mesmo lugar', async () => {
  const t = await tempStore();
  try {
    const { c, turns } = await chatWith(t.store);
    const before = await history(t.store, c.id);
    const del = await t.store.call('deleteMessage', { convId: c.id, messageId: turns[1].interactionId });
    assert.ok(del.batch);
    const afterDelete = await history(t.store, c.id);
    assert.ok(!afterDelete.some((m) => m.includes('Pergunta 2')));
    const trash = await t.store.call('listTrash');
    const item = trash.find((x) => x.type === 'interactions');
    assert.ok(item, 'item da lixeira existe');
    assert.equal(item.count, 1);
    assert.equal(item.batch, del.batch);
    assert.match(item.preview, /Pergunta 2/, 'prévia do conteúdo apagado');
    await t.store.call('restoreBatch', c.id, del.batch);
    assert.deepEqual(await history(t.store, c.id), before, 'mesma ordem e conteúdo');
    assert.equal((await t.store.call('listTrash')).length, 0);
  } finally {
    await t.cleanup();
  }
});

test('lixeira: mensagem do meio apagada, conversa continua e a restauração mantém a ordem', async () => {
  const t = await tempStore();
  try {
    const { c, turns } = await chatWith(t.store, 3);
    const del = await t.store.call('deleteMessage', { convId: c.id, messageId: turns[0].interactionId });
    await turn(t.store, c.id, 'Pergunta nova', 'Mira: Resposta nova.');
    await t.store.call('restoreBatch', c.id, del.batch);
    const h = await history(t.store, c.id);
    assert.ok(h.findIndex((m) => m.includes('Pergunta 1')) < h.findIndex((m) => m.includes('Pergunta 2')));
    assert.ok(h.findIndex((m) => m.includes('Pergunta 3')) < h.findIndex((m) => m.includes('Pergunta nova')));
    assert.equal(h.length, 8);
  } finally {
    await t.cleanup();
  }
});

test('lixeira: apagar só a resposta (parte) aparece como "responses" com prévia e restaura', async () => {
  const t = await tempStore();
  try {
    const { c } = await chatWith(t.store, 2);
    const msgs = (await t.store.call('getConversation', c.id)).history;
    const last = msgs.filter((m) => m.role === 'assistant').at(-1);
    const del = await t.store.call('deleteMessage', { convId: c.id, messageId: last.id });
    const item = (await t.store.call('listTrash')).find((x) => x.type === 'responses');
    assert.ok(item);
    assert.match(item.preview, /Resposta 2/);
    await t.store.call('restoreBatch', c.id, del.batch);
    assert.ok((await history(t.store, c.id)).some((m) => m.includes('Resposta 2')));
  } finally {
    await t.cleanup();
  }
});

test('lixeira: apagar definitivamente um item remove só ele e não dá para restaurar', async () => {
  const t = await tempStore();
  try {
    const { c, turns } = await chatWith(t.store, 4);
    const d1 = await t.store.call('deleteMessage', { convId: c.id, messageId: turns[0].interactionId });
    const d2 = await t.store.call('deleteMessage', { convId: c.id, messageId: turns[2].interactionId });
    assert.equal((await t.store.call('listTrash')).filter((x) => x.type === 'interactions').length, 2);
    const r = await t.store.call('purgeTrash', { type: 'interactions', convId: c.id, batch: d1.batch });
    assert.equal(r.interactions, 1);
    const left = (await t.store.call('listTrash')).filter((x) => x.type === 'interactions');
    assert.deepEqual(left.map((x) => x.batch), [d2.batch], 'o outro item continua');
    await assert.rejects(t.store.call('restoreBatch', c.id, d1.batch), (e) => e.status === 404, 'item apagado de vez: nada a restaurar');
    assert.ok(!(await history(t.store, c.id)).some((m) => m.includes('Pergunta 1')), 'apagado de vez');
    await t.store.call('restoreBatch', c.id, d2.batch);
    assert.ok((await history(t.store, c.id)).some((m) => m.includes('Pergunta 3')));
  } finally {
    await t.cleanup();
  }
});

test('lixeira: conversa apagada tem prévia/contagem, restaura inteira e só vai de vez quando pedido', async () => {
  const t = await tempStore();
  try {
    const { c } = await chatWith(t.store, 3);
    await assert.rejects(t.store.call('purgeTrash', { type: 'conversation', id: c.id }), (e) => e.status === 400, 'conversa ativa nunca é apagada de vez');
    await t.store.call('deleteConversation', c.id);
    const item = (await t.store.call('listTrash')).find((x) => x.type === 'conversation');
    assert.equal(item.id, c.id);
    assert.equal(item.count, 3);
    assert.match(item.preview, /Resposta 3/);
    await t.store.call('restoreConversation', c.id);
    assert.equal((await history(t.store, c.id)).length, 6);
    await t.store.call('deleteConversation', c.id);
    const r = await t.store.call('purgeTrash', { type: 'conversation', id: c.id });
    assert.equal(r.conversations, 1);
    assert.equal((await t.store.call('listTrash')).length, 0);
    assert.equal((await t.store.call('listConversations', {})).length, 0);
  } finally {
    await t.cleanup();
  }
});

test('lixeira: esvaziar tudo apaga conversas, mensagens e respostas já apagadas e preserva o resto', async () => {
  const t = await tempStore();
  try {
    const a = await chatWith(t.store, 3);
    const b = await chatWith(t.store, 2);
    await t.store.call('deleteMessage', { convId: a.c.id, messageId: a.turns[0].interactionId });
    await t.store.call('deleteConversation', b.c.id);
    assert.equal((await t.store.call('listTrash')).length, 2);
    const r = await t.store.call('purgeTrash', { all: true });
    assert.equal(r.conversations, 1);
    assert.ok(r.interactions >= 1);
    assert.equal((await t.store.call('listTrash')).length, 0);
    const kept = await history(t.store, a.c.id);
    assert.ok(kept.some((m) => m.includes('Pergunta 2')) && kept.some((m) => m.includes('Pergunta 3')), 'o que não foi apagado continua');
  } finally {
    await t.cleanup();
  }
});

test('lixeira: itens com menos de 30 dias sobrevivem à limpeza automática e os mais antigos saem', async () => {
  const t = await tempStore();
  try {
    const { c, turns } = await chatWith(t.store, 3);
    const del = await t.store.call('deleteMessage', { convId: c.id, messageId: turns[0].interactionId });
    const day = 86_400_000;
    const soon = await t.store.call('purgeExpired', { now: Date.now() + 29 * day });
    assert.equal(soon.interactions, 0, 'aos 29 dias ainda é recuperável');
    assert.equal((await t.store.call('listTrash')).length, 1);
    await t.store.call('restoreBatch', c.id, del.batch);
    const again = await t.store.call('deleteMessage', { convId: c.id, messageId: turns[1].interactionId });
    assert.ok(again.batch);
    const late = await t.store.call('purgeExpired', { now: Date.now() + 31 * day });
    assert.equal(late.interactions, 1, 'passados 30 dias é apagado de vez');
    assert.equal((await t.store.call('listTrash')).length, 0);
  } finally {
    await t.cleanup();
  }
});
