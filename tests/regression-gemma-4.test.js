import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_PRESETS } from '../client/src/default-presets.js';
import { mergeWithDefaults, normalizeChatPreset, publicPresets } from '../client/src/api.js';

test('Regressão: Gemma 4 deve estar presente em DEFAULT_PRESETS com rota de grupo e openrouter', () => {
  const gemma = DEFAULT_PRESETS.presets.find((p) => p.id === 'gemma-4');
  assert.ok(gemma, 'gemma-4 deve existir em DEFAULT_PRESETS');
  assert.equal(gemma.groupRoute, 'Gemma4', 'Gemma 4 deve ter groupRoute Gemma4');
  assert.equal(gemma.label, 'Gemma 4');
  assert.equal(gemma.selectable, true);
  assert.equal(gemma.family, 'gemma');
  assert.ok(gemma.routes.some((r) => r.id === 'Gemma4'), 'Deve ter rota de grupo Gemma4');
  assert.ok(gemma.routes.some((r) => r.id === 'openrouter/google/gemma-4-31b-it:free'), 'Deve ter rota openrouter');
  assert.ok(gemma.preferredRoutes.includes('Gemma4'), 'preferredRoutes deve incluir Gemma4');
  assert.ok(gemma.preferredRoutes.includes('openrouter/google/gemma-4-31b-it:free'), 'preferredRoutes deve incluir rota openrouter');
  assert.deepEqual(gemma.efforts, ['low', 'medium', 'high'], 'Gemma 4 deve suportar low, medium, high');
  assert.equal(gemma.thinkingCanDisable, true, 'Gemma 4 deve permitir controle de pensamento');
});

test('Regressão: mergeWithDefaults recupera gemma-4 e preserva generic no fim', () => {
  const staleCached = {
    presets: [
      {
        id: 'deepseek-v41-flash',
        groupRoute: 'Deepseek',
        efforts: ['high'],
        routes: [{ id: 'Deepseek' }],
        preferredRoutes: ['Deepseek']
      },
      {
        id: 'generic',
        label: 'Outro modelo (sem controle de esforço)',
        routes: []
      }
    ]
  };

  const merged = mergeWithDefaults(staleCached);
  assert.ok(merged.presets.some((p) => p.id === 'gemma-4'), 'mergeWithDefaults deve injetar gemma-4');
  const lastPreset = merged.presets[merged.presets.length - 1];
  assert.equal(lastPreset.id, 'generic', '"generic" deve ser o último item');
  const gemmaIndex = merged.presets.findIndex((p) => p.id === 'gemma-4');
  const genericIndex = merged.presets.findIndex((p) => p.id === 'generic');
  assert.ok(gemmaIndex < genericIndex, 'gemma-4 deve aparecer antes de generic');
});

test('Regressão: normalizeChatPreset normaliza rota Gemma4 e openrouter para preset gemma-4', () => {
  const groupChat = { id: 'c1', name: 'Chat', presetId: 'generic', routeId: 'Gemma4' };
  assert.equal(normalizeChatPreset(groupChat).presetId, 'gemma-4');

  const openrouterChat = { id: 'c2', name: 'Chat 2', presetId: null, routeId: 'openrouter/google/gemma-4-31b-it:free' };
  assert.equal(normalizeChatPreset(openrouterChat).presetId, 'gemma-4');

  const directChat = { id: 'c3', name: 'Chat 3', presetId: null, routeId: 'google/gemma-4-31b-it' };
  assert.equal(normalizeChatPreset(directChat).presetId, 'gemma-4');

  // Não deve interferir com embeddinggemma
  const embedChat = { id: 'c4', name: 'Chat 4', presetId: 'generic', routeId: 'embeddinggemma-300m' };
  assert.equal(normalizeChatPreset(embedChat).presetId, 'generic');
});

test('Regressão: publicPresets expõe gemma-4 no catálogo público', () => {
  const raw = {
    presets: [
      { id: 'gemma-4', selectable: true, label: 'Gemma 4' }
    ]
  };
  const pub = publicPresets(raw);
  assert.ok(pub.presets.some((p) => p.id === 'gemma-4'), 'gemma-4 deve estar em publicPresets');
});
