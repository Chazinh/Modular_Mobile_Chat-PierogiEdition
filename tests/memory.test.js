import test from 'node:test';
import assert from 'node:assert/strict';
import { tempStore, newConv, turn } from './helpers.js';

// Store memory rules (Rust). Context assembly itself is verified in
// server-rust/tests/legacy_ports.rs; extraction, cast and heuristic review in server-rust/tests/contracts.rs.
const baseExt = (over = {}) => ({ summary: 's', cast: [], scene: null, facts: [], relationships: [], openThreads: [], resolvedThreads: [], ...over });

test('descoberta exige trecho que a comprove: sem revelação registrada, ausente não passa a saber', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Conto a Laura que o freio foi cortado.', 'Laura: Meu Deus, quem fez isso?');
    const src = 'Alex conta a Laura que o freio foi cortado. Laura: Meu Deus, quem fez isso?';
    await t.store.call('applyExtraction', {
      convId: c.id, interactionId: a.interactionId, versionId: a.versionId, versionIds: [a.versionId], sourceText: src,
      extraction: baseExt({
        scene: { location: 'sala', time: null, mood: null, present: ['Laura'], objects: [], positions: {}, holding: {}, locations: {} },
        facts: [{ kind: 'SECRET', subject: 'Mark', content: 'O freio do carro foi cortado.', importance: 0.9, confidence: 0.9, visibility: 'private', knowledge: [
          { who: 'Laura', state: 'knows', via: 'told', evidence: 'conta a Laura que o freio foi cortado', belief: null }, // backed by the text
          { who: 'Eric', state: 'knows', via: 'heard', evidence: 'Eric ouviu tudo pela porta', belief: null }, // invented: not in the text
          { who: 'Sarah', state: 'knows', via: 'witnessed', evidence: '', belief: null }, // absent from the scene
        ] }],
      }),
    });
    const mem = await t.store.call('memoryOverview', c.id);
    const names = mem.memories[0].knowledge.map((k) => k.name).sort();
    assert.deepEqual(names, ['Laura']);
  } finally {
    await t.cleanup();
  }
});

test('estado de cena substitui o anterior mas o histórico das posições é mantido', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'a', 'x');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: a.interactionId, versionId: null, versionIds: [], sourceText: 'a x', extraction: baseExt({ scene: { location: 'cozinha', time: null, mood: null, present: ['Lara'], objects: [], positions: { Lara: 'sentada no sofá' }, holding: { Lara: ['telefone'] }, locations: {} } }) });
    const b = await turn(t.store, c.id, 'b', 'y');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: b.interactionId, versionId: null, versionIds: [], sourceText: 'b y', extraction: baseExt({ scene: { location: 'corredor', time: null, mood: null, present: ['Lara'], objects: [], positions: { Lara: 'junto à janela' }, holding: {}, locations: {} } }) });
    const scene = await t.store.call('currentScene', c.id);
    assert.equal(scene.location, 'corredor');
    assert.equal(scene.characters.find((x) => x.name === 'Lara').position, 'junto à janela');
    const hist = await t.store.call('sceneHistory', c.id, null, 10);
    assert.equal(hist.length, 2, 'estado anterior conservado como histórico');
  } finally {
    await t.cleanup();
  }
});

test('fatos substituídos viram históricos e voltam se o ramo que os substituiu for abandonado', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'a', 'x');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: a.interactionId, versionId: null, versionIds: [], sourceText: 'a x', extraction: baseExt({ facts: [{ kind: 'CHARACTER_FACT', subject: 'Lara', content: 'Lara mora em Recife.', importance: 0.7, confidence: 0.9, knowledge: [] }] }) });
    const b = await turn(t.store, c.id, 'b', 'y');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: b.interactionId, versionId: null, versionIds: [], sourceText: 'b y', extraction: baseExt({ facts: [{ kind: 'CHARACTER_FACT', subject: 'Lara', content: 'Lara mora em São Paulo.', importance: 0.7, confidence: 0.9, replaces: 'Lara mora em Recife.', knowledge: [] }] }) });
    let mem = await t.store.call('memoryOverview', c.id);
    assert.deepEqual(mem.memories.map((m) => m.content), ['Lara mora em São Paulo.']);
    // leaves the branch: edits the second message
    await turn(t.store, c.id, 'b editada', 'z', { mode: 'edit', targetId: b.interactionId });
    mem = await t.store.call('memoryOverview', c.id);
    assert.deepEqual(mem.memories.map((m) => m.content), ['Lara mora em Recife.']);
  } finally {
    await t.cleanup();
  }
});

test('fatos repetidos no mesmo turno (visto com o modelo local real) viram uma só memória', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Oi', 'Mira: Oi.');
    const fact = { kind: 'EVENT', subject: 'Mira', content: 'Mira foi interrompida no meio de uma demonstração impecável.', importance: 0.6, confidence: 0.9, knowledge: [] };
    await t.store.call('applyExtraction', { convId: c.id, interactionId: a.interactionId, versionId: null, versionIds: [], sourceText: 'x', extraction: baseExt({ facts: [fact, { ...fact }, { ...fact, content: 'Mira foi interrompida no meio de uma demonstração impecável!' }] }) });
    assert.equal((await t.store.call('memoryOverview', c.id)).memories.length, 1);
  } finally {
    await t.cleanup();
  }
});

test('fato banal e incerto não vira memória de longo prazo', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    const a = await turn(t.store, c.id, 'Oi', 'Mira: Oi.');
    await t.store.call('applyExtraction', { convId: c.id, interactionId: a.interactionId, versionId: null, versionIds: [], sourceText: 'x', extraction: baseExt({ facts: [
      { kind: 'EVENT', subject: 'Mira', content: 'Mira piscou os olhos duas vezes.', importance: 0.1, confidence: 0.4, knowledge: [] },
      { kind: 'PROMISE', subject: 'Mira', content: 'Mira prometeu ajudar Alex no exame.', importance: 0.9, confidence: 0.9, knowledge: [] },
    ] }) });
    assert.deepEqual((await t.store.call('memoryOverview', c.id)).memories.map((m) => m.content), ['Mira prometeu ajudar Alex no exame.']);
  } finally {
    await t.cleanup();
  }
});

test('reanálise da ficha remove registros errados não confirmados e preserva os confirmados', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store);
    await t.store.call('upsertCast', c.id, [
      { name: 'Aptidão', source: 'card' },
      { name: 'Perfil', source: 'card' },
      { name: 'Celestial', source: 'card', confirmed: true },
      { name: 'Mark', source: 'narrative' },
    ]);
    const r = await t.store.call('replaceCardCast', c.id, [{ name: 'Mira Solano', kind: 'main', aliases: ['Mira'], description: 'maga' }], { persona: { name: 'Alex', description: 'Eu' } });
    assert.equal(r.removed, 2);
    const names = (await t.store.call('listCast', c.id)).map((x) => x.name).sort();
    assert.deepEqual(names, ['Alex', 'Celestial', 'Mark', 'Mira Solano']);
  } finally {
    await t.cleanup();
  }
});

test('perfis de persona: criados a partir das conversas e trocáveis por conversa (a persona anterior fica inativa)', async () => {
  const t = await tempStore();
  try {
    const c = await newConv(t.store, { prompt: 'Você é Mira. {{char}} nunca age por {{user}}.' });
    await t.store.call('upsertCast', c.id, [{ name: 'Alex', kind: 'user', source: 'manual', confirmed: true, description: '30 anos, óculos redondos' }]);
    assert.equal(await t.store.call('seedPersonas'), 1);
    const [alex] = await t.store.call('listPersonas');
    assert.equal(alex.name, 'Alex');
    assert.equal(alex.isDefault, true);
    assert.match(alex.description, /óculos redondos/);
    // new profile, switched in this conversation
    const lucas = await t.store.call('savePersona', { name: 'Lucas', description: 'Mercenário de 30 anos, cicatriz no rosto.' });
    await t.store.call('patchConversation', c.id, { persona: 'Lucas', settings: { personaId: lucas.id } });
    await t.store.call('syncUserCast', c.id, lucas);
    const cast = await t.store.call('listCast', c.id);
    assert.equal(cast.find((x) => x.kind === 'user').name, 'Lucas');
    assert.equal(cast.find((x) => x.name === 'Alex').active, false, 'persona anterior fica inativa, não apagada');
    const data = await t.store.call('getContextData', { convId: c.id, pendingUserText: 'Oi' });
    assert.match(data.conv.personaProfile.description, /Mercenário de 30 anos/, 'o contexto carrega o perfil escolhido');
  } finally {
    await t.cleanup();
  }
});
