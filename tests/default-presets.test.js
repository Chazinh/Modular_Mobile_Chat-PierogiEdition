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
  assert.ok(ids.has('kimi-k3'));
  assert.ok(ids.has('muse-spark-13'));
  assert.ok(ids.has('glm-52'));
  assert.ok(ids.has('step-5-preview'));

  // Regressão: rotas das screenshots do usuário
  const deepseek = DEFAULT_PRESETS.presets.find(p => p.id === 'deepseek-v41-flash');
  assert.ok(deepseek.preferredRoutes.includes('oc/deepseek-v4.1-flash-free'), 'DeepSeek deve incluir oc/deepseek-v4.1-flash-free');
  assert.ok(deepseek.preferredRoutes.includes('cbai/deepseek-v4.1-flash'), 'DeepSeek deve incluir cbai/deepseek-v4.1-flash');
  assert.ok(deepseek.preferredRoutes.includes('nvidia/deepseek-ai/deepseek-v4.1-flash'), 'DeepSeek deve incluir nvidia/deepseek-ai/deepseek-v4.1-flash');

  const glm = DEFAULT_PRESETS.presets.find(p => p.id === 'glm-52');
  assert.ok(glm.preferredRoutes.includes('cbai/glm-5.2'), 'GLM deve incluir cbai/glm-5.2');
  assert.ok(glm.preferredRoutes.includes('nvidia/z-ai/glm-5.2'), 'GLM deve incluir nvidia/z-ai/glm-5.2');

  const gemini = DEFAULT_PRESETS.presets.find(p => p.id === 'gemini-38-flash');
  assert.ok(gemini.preferredRoutes.includes('gemini/gemini-3.8-flash'), 'Gemini deve incluir gemini/gemini-3.8-flash');

  // Regressão: NENHUM nome de combo do 9Router adicionado como rota falsa
  const forbiddenComboTitles = ['Deepseek', 'Opus', 'Sonnet', 'Gemini', 'Kimi', 'Muse', 'GLM', 'Step5-preview'];
  for (const preset of DEFAULT_PRESETS.presets) {
    for (const forbidden of forbiddenComboTitles) {
      assert.ok(!preset.preferredRoutes.includes(forbidden), `${preset.id} não deve conter título de combo '${forbidden}' em preferredRoutes`);
      assert.ok(!preset.routes.some(r => r.id === forbidden), `${preset.id} não deve conter título de combo '${forbidden}' em routes`);
    }
  }

  // Regressão: Argon EXPLICITAMENTE excluído ("menos o argon")
  for (const preset of DEFAULT_PRESETS.presets) {
    assert.ok(!preset.id.toLowerCase().includes('argon'), `Preset ${preset.id} não deve ser argon`);
    assert.ok(!preset.preferredRoutes.some(r => r.toLowerCase().includes('argon')), `Preset ${preset.id} não deve conter rota argon`);
    assert.ok(!preset.routes.some(r => r.id.toLowerCase().includes('argon')), `Preset ${preset.id} não deve conter rota argon`);
  }
});
