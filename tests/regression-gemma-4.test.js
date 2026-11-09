import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeWithDefaults, normalizeChatPreset } from '../client/src/api.js';

test('mergeWithDefaults atualiza cache sem reinjetar modelos removidos e preserva generic no fim', () => {
  const merged = mergeWithDefaults({ presets: [
    { id: 'deepseek-v41-flash', routes: [{ id: 'Deepseek' }], preferredRoutes: ['Deepseek'] },
    { id: 'gemma-4', selectable: true },
    { id: 'generic', label: 'Outro modelo', routes: [] },
  ] });
  assert.ok(!merged.presets.some(p => p.id === 'gemma-4'));
  assert.equal(merged.presets.at(-1).id, 'generic');
});

test('normalizeChatPreset preserva compatibilidade com conversas Gemma existentes', () => {
  for (const routeId of ['Gemma4', 'openrouter/google/gemma-4-31b-it:free', 'google/gemma-4-31b-it']) {
    assert.equal(normalizeChatPreset({ presetId: 'generic', routeId }).presetId, 'gemma-4');
  }
  assert.equal(normalizeChatPreset({ presetId: 'generic', routeId: 'embeddinggemma-300m' }).presetId, 'generic');
});
