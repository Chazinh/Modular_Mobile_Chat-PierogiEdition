import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_PRESETS } from '../client/src/default-presets.js';

test('DEFAULT_PRESETS: valida estrutura, consistência de rotas e famílias', () => {
  assert.ok(DEFAULT_PRESETS && Array.isArray(DEFAULT_PRESETS.presets));
  assert.ok(DEFAULT_PRESETS.presets.length >= 5, 'Deve conter pelo menos 5 presets de modelo');

  const validEfforts = new Set(['low', 'medium', 'high']);
  const validThinking = new Set(['enabled', 'adaptive', 'level', 'always']);

  const ids = new Set();
  for (const preset of DEFAULT_PRESETS.presets) {
    assert.ok(preset.id && typeof preset.id === 'string', 'id deve ser string');
    assert.ok(!ids.has(preset.id), `id de preset duplicado: ${preset.id}`);
    ids.add(preset.id);

    assert.ok(preset.label && typeof preset.label === 'string', `${preset.id} deve ter label`);
    assert.ok(preset.family && typeof preset.family === 'string', `${preset.id} deve ter family`);
    assert.ok(Array.isArray(preset.preferredRoutes) && preset.preferredRoutes.length > 0, `${preset.id} deve ter preferredRoutes`);
    assert.ok(Array.isArray(preset.routes) && preset.routes.length > 0, `${preset.id} deve ter routes`);

    // Cada rota listada deve ter um id válido
    for (const r of preset.routes) {
      assert.ok(r.id && typeof r.id === 'string', `${preset.id}: rota deve ter id string`);
    }

    // efforts e defaultEffort
    assert.ok(Array.isArray(preset.efforts) && preset.efforts.length > 0, `${preset.id} deve ter lista de esforços`);
    for (const eff of preset.efforts) {
      assert.ok(validEfforts.has(eff), `${preset.id} possui esforço inválido: ${eff}`);
    }
    assert.ok(validEfforts.has(preset.defaultEffort), `${preset.id} defaultEffort inválido`);
    assert.ok(preset.efforts.includes(preset.defaultEffort), `${preset.id} defaultEffort deve estar em efforts`);

    // thinking mode
    assert.ok(validThinking.has(preset.thinking), `${preset.id} thinking mode inválido: ${preset.thinking}`);

    // Se possui effortByRoute, valida mapeamento
    if (preset.effortByRoute) {
      for (const [routeKey, mappedEffort] of Object.entries(preset.effortByRoute)) {
        assert.ok(preset.routes.some(r => r.id === routeKey), `${preset.id}: effortByRoute aponta para rota inexistente ${routeKey}`);
        assert.ok(validEfforts.has(mappedEffort), `${preset.id}: effortByRoute mapeia para esforço inválido ${mappedEffort}`);
      }
    }
  }

  // Verifica que os modelos essenciais estão presentes
  assert.ok(ids.has('deepseek-v41-flash'));
  assert.ok(ids.has('claude-opus-5-5'));
  assert.ok(ids.has('claude-sonnet-5-5'));
  assert.ok(ids.has('gemini-38-flash'));
});
