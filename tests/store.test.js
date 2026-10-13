import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { tempStore, newConv, turn } from './helpers.js';

test('envio idempotente por opId não duplica mensagens', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Olá', '*Ela sorri.*\n\nMira: Oi.', { opId: 'op-1' });
    const again = await t.store.call('beginTurn', { convId: c.id, opId: 'op-1', mode: 'send', content: 'Olá', routeId: c.routeId, presetId: c.presetId });
    assert.equal(again.replay, true);
    assert.equal(again.generationId, a.generationId);
    const chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history.length, 2);
  } finally {
    await t.cleanup();
  }
});

test('refazer substitui só a resposta escolhida e mantém a anterior recuperável', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', 'Mira: Primeira.');
    const b = await t.store.call('beginTurn', { convId: c.id, mode: 'regenerate', routeId: c.routeId, presetId: c.presetId });
    // while generating, the previous version stays selected
    let chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history[1].content, 'Mira: Primeira.');
    await t.store.call('finishGeneration', { generationId: b.generationId, text: 'Mira: Segunda.', status: 'complete' });
    chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history.length, 2);
    assert.equal(chat.history[1].content, 'Mira: Segunda.');
    assert.equal(chat.history[1].versionCount, 2);
    const versions = await t.store.call('listVersions', { convId: c.id, partId: chat.history[1].id });
    assert.deepEqual(versions.map((v) => v.content), ['Mira: Primeira.', 'Mira: Segunda.']);
    // restaura a primeira
    await t.store.call('selectVersion', { convId: c.id, partId: chat.history[1].id, versionId: versions[0].id });
    chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history[1].content, 'Mira: Primeira.');
  } finally {
    await t.cleanup();
  }
});

test('refazer com erro mantém a resposta anterior escolhida', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', 'Mira: Primeira.');
    const b = await t.store.call('beginTurn', { convId: c.id, mode: 'regenerate', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('finishGeneration', { generationId: b.generationId, text: 'Mira: meio', status: 'error', error: 'queda' });
    const chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history[1].content, 'Mira: Primeira.');
    const vs = await t.store.call('listVersions', { convId: c.id, partId: chat.history[1].id });
    assert.equal(vs.length, 2, 'o parcial fica recuperável como alternativa');
  } finally {
    await t.cleanup();
  }
});

test('refazer uma continuação conserva a resposta que a precedeu', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', 'Mira: Parte um.');
    const k = await t.store.call('beginTurn', { convId: c.id, mode: 'continue', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('finishGeneration', { generationId: k.generationId, text: 'Mira: Parte dois.', status: 'complete' });
    let chat = await t.store.call('getConversation', c.id);
    assert.deepEqual(chat.history.map((m) => m.content), ['Oi', 'Mira: Parte um.', 'Mira: Parte dois.']);
    const r = await t.store.call('beginTurn', { convId: c.id, mode: 'regenerate', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('finishGeneration', { generationId: r.generationId, text: 'Mira: Parte dois nova.', status: 'complete' });
    chat = await t.store.call('getConversation', c.id);
    assert.deepEqual(chat.history.map((m) => m.content), ['Oi', 'Mira: Parte um.', 'Mira: Parte dois nova.']);
    assert.equal(chat.history[1].versionCount, 1, 'a resposta anterior não foi tocada');
    assert.equal(chat.history[2].origin, 'regenerate');
  } finally {
    await t.cleanup();
  }
});

test('lista de chats: texto parcial de geração em andamento nunca aparece na prévia nem na contagem; resposta pronta aparece', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const b = await t.store.call('beginTurn', { convId: c.id, mode: 'send', content: 'Oi, Mira', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('flushVersion', b.versionId, 'Mira: PARCIAL_NAO_PODE_APARECER');
    let list = await t.store.call('listConversations', {});
    let item = list.find((x) => x.id === c.id);
    assert.ok(!JSON.stringify(item).includes('PARCIAL_NAO_PODE_APARECER'), 'sem texto parcial na lista');
    assert.equal(item.lastMessage, 'Oi, Mira', 'a prévia cai para a mensagem do usuário');
    assert.equal(item.messageCount, 1);
    assert.equal(item.generating, true);
    for (const k of ['pendingJobs', 'openFlags']) assert.equal(typeof item[k], 'number', k);
    await t.store.call('finishGeneration', { generationId: b.generationId, text: 'Mira: Pronto.', status: 'complete' });
    list = await t.store.call('listConversations', {});
    item = list.find((x) => x.id === c.id);
    assert.equal(item.lastMessage, 'Mira: Pronto.');
    assert.equal(item.messageCount, 2);
    assert.equal(item.generating, false);
  } finally {
    await t.cleanup();
  }
});

test('falha antes de qualquer texto não deixa versão vazia e permite tentar de novo', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const b = await t.store.call('beginTurn', { convId: c.id, mode: 'send', content: 'Oi', routeId: c.routeId, presetId: c.presetId });
    const fin = await t.store.call('finishGeneration', { generationId: b.generationId, text: '', status: 'error', error: 'Provedor fora' });
    assert.equal(fin.versionKept, false);
    let chat = await t.store.call('getConversation', c.id);
    assert.deepEqual(chat.history.map((m) => m.role), ['user']); // the user message was preserved
    const retry = await t.store.call('beginTurn', { convId: c.id, mode: 'regenerate', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('finishGeneration', { generationId: retry.generationId, text: 'Mira: Agora sim.', status: 'complete' });
    chat = await t.store.call('getConversation', c.id);
    assert.deepEqual(chat.history.map((m) => m.role), ['user', 'assistant']);
  } finally {
    await t.cleanup();
  }
});

test('segunda geração simultânea na mesma conversa é recusada', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await t.store.call('beginTurn', { convId: c.id, mode: 'send', content: 'Oi', routeId: c.routeId, presetId: c.presetId });
    await assert.rejects(t.store.call('beginTurn', { convId: c.id, mode: 'send', content: 'Outra', routeId: c.routeId, presetId: c.presetId }), (e) => e.status === 409 && !!e.generationId);
    const busy = await t.store.call('busyConversations');
    assert.equal(busy.length, 1);
  } finally {
    await t.cleanup();
  }
});

test('configurações salvas durante a geração permanecem salvas', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const b = await t.store.call('beginTurn', { convId: c.id, mode: 'send', content: 'Oi', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('patchConversation', c.id, { name: 'Renomeada', persona: 'Alex R.', temperature: 1.1, pinned: true, effort: 'low', settings: { context: { preferredPairs: 40 } } });
    await t.store.call('finishGeneration', { generationId: b.generationId, text: 'Mira: Oi.', status: 'complete' });
    const after = await t.store.call('getConversationMeta', c.id);
    assert.equal(after.name, 'Renomeada');
    assert.equal(after.persona, 'Alex R.');
    assert.equal(after.temperature, 1.1);
    assert.equal(after.pinned, true);
    assert.equal(after.effort, 'low');
    assert.equal(after.settings.context.preferredPairs, 40);
  } finally {
    await t.cleanup();
  }
});

test('personalização: revisão nova só quando muda e conflito é detectado', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const same = await t.store.call('setCharacterText', c.id, c.prompt, 'file');
    assert.equal(same.changed, false);
    const ext = await t.store.call('setCharacterText', c.id, `${c.prompt}\nNova regra.`, 'file');
    assert.equal(ext.changed, true);
    assert.equal(ext.rev, 2);
    await assert.rejects(t.store.call('patchConversation', c.id, { prompt: 'x' }, { baseCharacterRev: 1 }), (e) => e.code === 'CHARACTER_CONFLICT' && e.current.characterRev === 2);
    const ok = await t.store.call('patchConversation', c.id, { prompt: 'novo texto' }, { baseCharacterRev: 2 });
    assert.equal(ok.characterRev, 3);
  } finally {
    await t.cleanup();
  }
});

test('editar mensagem antiga cria ramo, preserva o anterior e invalida memórias do ramo abandonado', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Primeira', 'Mira: Um.');
    const b = await turn(t.store, c.id, 'Segunda', 'Mira: Dois.');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: b.interactionId, versionId: null, versionIds: [], sourceText: 'Segunda Dois', extraction: { summary: 'resumo', cast: [], scene: null, facts: [{ kind: 'WORLD_FACT', subject: null, content: 'A academia fica numa ilha.', importance: 0.8, confidence: 0.9, knowledge: [] }], relationships: [], openThreads: [], resolvedThreads: [] } });
    let mem = await t.store.call('memoryOverview', c.id);
    assert.equal(mem.memories.length, 1);
    // edits the first message -> new branch
    const e = await turn(t.store, c.id, 'Primeira EDITADA', 'Mira: Novo caminho.', { mode: 'edit', targetId: a.interactionId });
    let chat = await t.store.call('getConversation', c.id);
    assert.deepEqual(chat.history.map((m) => m.content), ['Primeira EDITADA', 'Mira: Novo caminho.']);
    assert.equal(chat.history[0].branch.count, 2);
    mem = await t.store.call('memoryOverview', c.id);
    assert.equal(mem.memories.length, 0, 'memória do ramo abandonado não vale mais');
    // o ramo antigo continua guardado e pode voltar
    const branches = await t.store.call('listBranches', c.id);
    assert.equal(branches.length, 1);
    const old = branches[0].siblings.find((s) => !s.active);
    await t.store.call('activateBranch', { convId: c.id, interactionId: old.id });
    chat = await t.store.call('getConversation', c.id);
    assert.deepEqual(chat.history.map((m) => m.content), ['Primeira', 'Mira: Um.', 'Segunda', 'Mira: Dois.']);
    mem = await t.store.call('memoryOverview', c.id);
    assert.equal(mem.memories.length, 1, 'ao restaurar o ramo, a memória volta');
    void e;
  } finally {
    await t.cleanup();
  }
});

test('editar a resposta reextrai: memórias derivadas da versão antiga são removidas', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Oi', 'Mira: Eu sabotei o carro.');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: a.interactionId, versionId: a.versionId, versionIds: [a.versionId], sourceText: 'x', extraction: { summary: 's', cast: [], scene: null, facts: [{ kind: 'SECRET', subject: 'Mira', content: 'Mira sabotou o carro.', importance: 0.9, confidence: 0.9, knowledge: [] }], relationships: [], openThreads: [], resolvedThreads: [] } });
    assert.equal((await t.store.call('memoryOverview', c.id)).memories.length, 1);
    const chat = await t.store.call('getConversation', c.id);
    await t.store.call('editMessage', { convId: c.id, messageId: chat.history[1].id, content: 'Mira: Eu não fiz nada.' });
    assert.equal((await t.store.call('memoryOverview', c.id)).memories.length, 0);
    const jobs = await t.store.call('jobSummary', c.id);
    assert.ok(jobs.pending >= 1, 'um novo trabalho de extração foi agendado');
  } finally {
    await t.cleanup();
  }
});

test('limpar histórico invalida a memória e a lixeira restaura por 30 dias', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Oi', 'Mira: Oi.');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: a.interactionId, versionId: null, versionIds: [], sourceText: 'Oi', extraction: { summary: 's', cast: [], scene: null, facts: [{ kind: 'WORLD_FACT', subject: null, content: 'O mundo tem magia.', importance: 0.7, confidence: 0.9, knowledge: [] }], relationships: [], openThreads: [], resolvedThreads: [] } });
    const r = await t.store.call('clearHistory', c.id);
    let chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history.length, 0);
    assert.equal((await t.store.call('memoryOverview', c.id)).memories.length, 0);
    const trash = await t.store.call('listTrash');
    assert.equal(trash[0].type, 'interactions');
    await t.store.call('restoreBatch', c.id, r.batch);
    chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history.length, 2);
    assert.equal((await t.store.call('memoryOverview', c.id)).memories.length, 1);
  } finally {
    await t.cleanup();
  }
});

test('exclusão de conversa é recuperável e só some depois de 30 dias', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', 'Mira: Oi.');
    await t.store.call('deleteConversation', c.id);
    assert.equal((await t.store.call('listConversations', {})).length, 0);
    await t.store.call('restoreConversation', c.id);
    assert.equal((await t.store.call('listConversations', {})).length, 1);
    await t.store.call('deleteConversation', c.id);
    const early = await t.store.call('purgeExpired', { now: Date.now() + 10 * 86_400_000 });
    assert.equal(early.conversations, 0);
    const late = await t.store.call('purgeExpired', { now: Date.now() + 31 * 86_400_000 });
    assert.equal(late.conversations, 1);
    await assert.rejects(t.store.call('getConversationMeta', c.id), (e) => e.status === 404);
  } finally {
    await t.cleanup();
  }
});

test('geração interrompida por reinício vira parcial preservado', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const b = await t.store.call('beginTurn', { convId: c.id, mode: 'send', content: 'Oi', routeId: c.routeId, presetId: c.presetId });
    await t.store.call('flushVersion', b.versionId, 'Mira: meio da fra');
    assert.equal(await t.store.call('recoverInterrupted'), 1);
    const chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history[1].content, 'Mira: meio da fra');
    assert.equal(chat.history[1].status, 'interrupted');
  } finally {
    await t.cleanup();
  }
});

test('busca FTS encontra mensagens com acentos e ignora o que foi apagado', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Vamos até a catedral antiga', 'Mira: A catedral está em ruínas.');
    let r = await t.store.call('searchAll', { q: 'catedral ruinas' });
    assert.ok(r.length >= 1);
    const chat = await t.store.call('getConversation', c.id);
    await t.store.call('deleteMessage', { convId: c.id, messageId: chat.history[0].id });
    r = await t.store.call('searchAll', { q: 'catedral' });
    assert.equal(r.length, 0);
  } finally {
    await t.cleanup();
  }
});

test('exportar e importar cria nova conversa sem tocar na original', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', 'Mira: Oi.');
    const data = await t.store.call('exportAll');
    const out = await t.store.call('importAll', JSON.parse(JSON.stringify(data)));
    assert.equal(out.imported.length, 1);
    assert.notEqual(out.imported[0], c.id);
    const copy = await t.store.call('getConversation', out.imported[0]);
    assert.deepEqual(copy.history.map((m) => m.content), ['Oi', 'Mira: Oi.']);
    assert.equal(copy.prompt, c.prompt);
    assert.equal((await t.store.call('getConversation', c.id)).history.length, 2);
  } finally {
    await t.cleanup();
  }
});

test('duplicar personagem cria conversa limpa com elenco e personalização', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await t.store.call('upsertCast', c.id, [{ name: 'Mira', kind: 'main', confirmed: true }]);
    await turn(t.store, c.id, 'Oi', 'Mira: Oi.');
    const d = await t.store.call('duplicateConversation', c.id);
    const chat = await t.store.call('getConversation', d.id);
    assert.equal(chat.history.length, 0);
    assert.equal(chat.prompt, c.prompt);
    assert.equal((await t.store.call('listCast', d.id)).some((x) => x.name === 'Mira'), true);
  } finally {
    await t.cleanup();
  }
});

test('migração dos arquivos antigos: backup, IDs/ordem preservados, originais intactos e falha isolada', async () => {
  const t = await tempStore();
  try {
    const chats = path.join(t.dir, 'chats');
    const good = 'aaaaaaaa-1111-2222-3333-444444444444';
    const bad = 'bbbbbbbb-1111-2222-3333-444444444444';
    fs.mkdirSync(path.join(chats, good), { recursive: true });
    fs.mkdirSync(path.join(chats, bad), { recursive: true });
    const meta = { id: good, name: 'Antigo', persona: 'Alex', model: 'cbai/deepseek-v4.1-flash', temperature: 0.9, createdAt: '2026-10-01T10:00:00.000Z', updatedAt: '2026-10-02T10:00:00.000Z' };
    const history = [
      { id: 'm1', role: 'user', content: 'Oi', ts: '2026-10-01T10:00:01.000Z' },
      { id: 'm2', role: 'assistant', content: 'Mira: Olá.', ts: '2026-10-01T10:00:05.000Z', model: 'cbai/deepseek-v4.1-flash' },
      { id: 'm3', role: 'assistant', content: 'Mira: Continuando.', ts: '2026-10-01T10:00:09.000Z' },
      { id: 'm4', role: 'user', content: 'E agora?', ts: '2026-10-01T10:01:00.000Z', edited: true },
    ];
    fs.writeFileSync(path.join(chats, good, 'metadata.json'), JSON.stringify(meta));
    fs.writeFileSync(path.join(chats, good, 'history.json'), JSON.stringify(history));
    fs.writeFileSync(path.join(chats, good, 'character.txt'), 'Mira é maga.');
    fs.writeFileSync(path.join(chats, bad, 'metadata.json'), '{ isto não é json');
    const before = fs.readFileSync(path.join(chats, good, 'history.json'), 'utf8');

    const rep = await t.store.call('importLegacy', { chatsDir: chats, backupDir: path.join(t.dir, 'backup') });
    assert.equal(rep.imported, 1);
    assert.equal(rep.failed.length, 1);
    assert.ok(fs.existsSync(path.join(rep.backup, good, 'history.json')), 'backup feito');
    assert.equal(fs.readFileSync(path.join(chats, good, 'history.json'), 'utf8'), before, 'original intacto');

    const chat = await t.store.call('getConversation', good);
    assert.deepEqual(chat.history.map((m) => [m.id, m.role, m.content]), history.map((m) => [m.id, m.role, m.content]));
    assert.equal(chat.prompt, 'Mira é maga.');
    assert.equal(chat.temperature, 0.9);
    assert.equal(chat.history[3].edited, true);
    // a second run does not re-import
    const again = await t.store.call('importLegacy', { chatsDir: chats, backupDir: path.join(t.dir, 'backup') });
    assert.equal(again.imported, 0);
    // the old history created extraction jobs (one per answered interaction)
    const js = await t.store.call('jobSummary', good);
    assert.ok(js.pending >= 1);
  } finally {
    await t.cleanup();
  }
});

test('limpeza de registros técnicos preserva mensagens e o resumo das gerações', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await turn(t.store, c.id, 'Oi', 'Mira: Oi.', { opId: 'op-velho' });
    const later = Date.now() + 8 * 86_400_000;
    const r = await t.store.call('pruneLogs', { now: later });
    assert.ok(r.generationsTrimmed >= 1);
    const chat = await t.store.call('getConversation', c.id);
    assert.equal(chat.history.length, 2, 'mensagens intactas');
    assert.ok(chat.history[1].usage !== undefined);
    assert.equal(await t.store.call('compactDatabase'), true);
  } finally {
    await t.cleanup();
  }
});
