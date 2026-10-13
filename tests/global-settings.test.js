import test from 'node:test';
import assert from 'node:assert/strict';
import { tempStore, newConv, turn } from './helpers.js';

// Global settings persisted in the Rust Store. The presence of the limit, the ratio and the content in the assembled prompt
// is verified in server-rust/tests/legacy_ports.rs (global_settings_reach_the_system_prompt_and_the_anchor).
const DEFAULTS = { botChars: 8000, dialoguePct: 65, content: '', lengthMode: 'maximum', monologueChars: 300, turnPacing: 'interactive' };

test('normalização: limites seguros, padrões e conteúdo aparado', async () => {
  const t = await tempStore();
  try {
    assert.deepEqual(await t.store.call('getGlobalSettings'), DEFAULTS);
    const n = await t.store.call('setGlobalSettings', { botChars: 99999, dialoguePct: -20, content: `  proibido +18\r\nsem violência gráfica  ` });
    assert.equal(n.botChars, 8000, 'o teto duro continua 8.000');
    assert.equal(n.dialoguePct, 0);
    assert.equal(n.content, 'proibido +18\nsem violência gráfica');
    assert.equal((await t.store.call('setGlobalSettings', { botChars: 10 })).botChars, 500, 'mínimo 500');
    const strings = await t.store.call('setGlobalSettings', { botChars: '3000', dialoguePct: '40' });
    assert.equal(strings.botChars, 3000);
    assert.equal(strings.dialoguePct, 40);
    const bad = await t.store.call('setGlobalSettings', { botChars: 'abc' });
    assert.equal(bad.botChars, 8000);
    assert.equal((await t.store.call('setGlobalSettings', { content: 'x'.repeat(5000) })).content.length, 1500);
  } finally {
    await t.cleanup();
  }
});

test('persistência no banco do PC: parcial mescla, valor inválido nunca grava lixo e o contexto carrega os ajustes', async () => {
  const t = await tempStore();
  try {
    assert.deepEqual(await t.store.call('getGlobalSettings'), DEFAULTS);
    const a = await t.store.call('setGlobalSettings', { dialoguePct: 40 });
    assert.deepEqual(a, { ...DEFAULTS, dialoguePct: 40 });
    const b = await t.store.call('setGlobalSettings', { botChars: 3000, content: 'Conteúdo +18 permitido' });
    assert.deepEqual(b, { ...DEFAULTS, botChars: 3000, dialoguePct: 40, content: 'Conteúdo +18 permitido' });
    assert.deepEqual(await t.store.call('getGlobalSettings'), b, 'persistiu');
    const c = await t.store.call('setGlobalSettings', { botChars: 'lixo', dialoguePct: 1000 });
    assert.equal(c.botChars, 8000, 'inválido volta ao padrão');
    assert.equal(c.dialoguePct, 100);
    // the context sent to the model carries the settings (read together with getContextData)
    const conv = await newConv(t.store);
    await turn(t.store, conv.id, 'Oi', 'Mira: Oi.');
    const data = await t.store.call('getContextData', { convId: conv.id, pendingUserText: 'E agora?' });
    assert.deepEqual(data.globalSettings, c);
  } finally {
    await t.cleanup();
  }
});

test('controle de turnos: configurações antigas recebem padrões e ajustes novos persistem com limites', async () => {
  const t = await tempStore();
  try {
    const selected = await t.store.call('setGlobalSettings', { lengthMode: 'target', monologueChars: '600', turnPacing: 'balanced' });
    assert.deepEqual(selected, { ...DEFAULTS, lengthMode: 'target', monologueChars: 600, turnPacing: 'balanced' });
    assert.deepEqual(await t.store.call('getGlobalSettings'), selected);
    const off = await t.store.call('setGlobalSettings', { monologueChars: 0, turnPacing: 'free' });
    assert.equal(off.monologueChars, 0);
    assert.equal(off.turnPacing, 'free');
    const invalid = await t.store.call('setGlobalSettings', { lengthMode: 'garbage', monologueChars: 99999, turnPacing: 'garbage' });
    assert.equal(invalid.lengthMode, 'maximum');
    assert.equal(invalid.monologueChars, 2000);
    assert.equal(invalid.turnPacing, 'interactive');
    assert.equal((await t.store.call('setGlobalSettings', { monologueChars: 1 })).monologueChars, 100);
    assert.equal((await t.store.call('setGlobalSettings', { monologueChars: -5 })).monologueChars, 300, 'negativo volta ao padrão, não a Livre');
  } finally { await t.cleanup(); }
});
