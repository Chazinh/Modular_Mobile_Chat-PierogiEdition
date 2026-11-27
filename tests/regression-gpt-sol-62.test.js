import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeWithDefaults, normalizeChatPreset, publicPresets } from '../client/src/api.js';

test('Gpt-sol-6.2 preset exposes low, medium, and high efforts and normalizes routes', () => {
  const defaults = publicPresets(mergeWithDefaults({ presets: [] }));
  const gptSol = defaults.presets.find((p) => p.id === 'gpt-6-2-sol');
  assert.ok(gptSol, 'gpt-6-2-sol should be present in public defaults');
  assert.equal(gptSol.groupRoute, 'Gpt-sol-6.2');
  assert.deepEqual(gptSol.efforts, ['low', 'medium', 'high']);

  assert.equal(
    normalizeChatPreset({ id: 'g1', presetId: 'generic', routeId: 'Gpt-sol-6.2' }).presetId,
    'gpt-6-2-sol'
  );
  assert.equal(
    normalizeChatPreset({ id: 'g2', presetId: null, routeId: 'cx/gpt-6.2-sol(high)' }).presetId,
    'gpt-6-2-sol'
  );
});
