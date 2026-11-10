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
  assert.ok(ids.has('glm-52'));
  for (const id of ['muse-spark-13', 'nemotron-35', 'minimax-m31', 'gemma-4']) {
    assert.ok(!ids.has(id), `${id} removido do catálogo narrativo`);
  }

  // Modelos mortos/desligados devem ser removidos da lista ativa
  assert.ok(!ids.has('kimi-k3'), 'kimi-k3 desligado deve ser removido');
  assert.ok(!ids.has('step-5-preview'), 'step-5-preview desligado deve ser removido');

  // Regressão: rotas ativas do DeepSeek
  const deepseek = DEFAULT_PRESETS.presets.find(p => p.id === 'deepseek-v41-flash');
  assert.ok(deepseek.preferredRoutes.includes('oc/deepseek-v4.1-flash-free'), 'DeepSeek deve incluir oc/deepseek-v4.1-flash-free');
  assert.ok(deepseek.preferredRoutes.includes('cbai/deepseek-v4.1-flash'), 'DeepSeek deve incluir cbai/deepseek-v4.1-flash');
  assert.ok(deepseek.preferredRoutes.includes('nvidia/deepseek-ai/deepseek-v4.1-flash'), 'DeepSeek deve incluir nvidia/deepseek-ai/deepseek-v4.1-flash');

  // GLM: rota cbai ativa, nvidia desligada removida
  const glm = DEFAULT_PRESETS.presets.find(p => p.id === 'glm-52');
  assert.ok(glm.preferredRoutes.includes('cbai/glm-5.2'), 'GLM deve incluir cbai/glm-5.2');
  assert.ok(glm.preferredRoutes.includes('openrouter/z-ai/glm-5.3-flash'), 'GLM deve incluir openrouter/z-ai/glm-5.3-flash');
  assert.ok(glm.preferredRoutes.includes('openrouter/z-ai/glm-5.2'), 'GLM deve incluir openrouter/z-ai/glm-5.2');
  assert.ok(glm.preferredRoutes.includes('oc/glm-5.3-flash-free'), 'GLM deve incluir oc/glm-5.3-flash-free');
  assert.ok(glm.routes.some(r => r.id === 'openrouter/z-ai/glm-5.3-flash'), 'GLM deve expor a rota openrouter/z-ai/glm-5.3-flash');
  assert.ok(glm.routes.some(r => r.id === 'openrouter/z-ai/glm-5.2'), 'GLM deve expor a rota openrouter/z-ai/glm-5.2');
  assert.ok(!glm.preferredRoutes.includes('nvidia/z-ai/glm-5.2'), 'GLM NÃO deve incluir rota desligada nvidia/z-ai/glm-5.2');

  const gemini = DEFAULT_PRESETS.presets.find(p => p.id === 'gemini-38-flash');
  assert.ok(gemini.preferredRoutes.includes('gemini/gemini-3.8-flash'), 'Gemini deve incluir gemini/gemini-3.8-flash');

  // Gemini 3.8 Flash deve permitir ligar/desligar pensamento profundo
  assert.equal(gemini.thinkingCanDisable, true, 'Gemini 3.8 Flash deve permitir ligar/desligar modo pensamento profundo');

  // Suporte aos grupos/combos com fallback automático no 9Router
  assert.equal(deepseek.groupRoute, 'Deepseek');
  assert.ok(deepseek.routes.some(r => r.id === 'Deepseek'), 'DeepSeek deve expor a rota do grupo Deepseek');
  assert.equal(gemini.groupRoute, 'Gemini');
  assert.ok(gemini.routes.some(r => r.id === 'Gemini'), 'Gemini deve expor a rota do grupo Gemini');
  assert.equal(glm.groupRoute, 'GLM');
  assert.ok(glm.routes.some(r => r.id === 'GLM'), 'GLM deve expor a rota do grupo GLM');

  // Perfil Argon: ag/gemini-4-argon-medium, esforço low somente, thinking level com toggle
  const argon = DEFAULT_PRESETS.presets.find(p => p.id === 'gemini-4-argon-medium');
  assert.ok(argon, 'Preset Argon deve existir nos DEFAULT_PRESETS');
  assert.equal(argon.label, 'Argon', 'Preset deve ter label Argon');
  assert.equal(argon.groupRoute, 'Argon', 'Argon deve ter groupRoute Argon');
  assert.ok(argon.preferredRoutes.includes('ag/gemini-4-argon-medium'), 'Argon deve preferir ag/gemini-4-argon-medium');
  assert.ok(argon.routes.some(r => r.id === 'ag/gemini-4-argon-medium'), 'Argon deve expor a rota ag/gemini-4-argon-medium');
  assert.deepEqual(argon.efforts, ['low'], 'Argon deve aceitar esforço low somente');
  assert.equal(argon.defaultEffort, 'low', 'Esforço padrão do Argon deve ser low');
  assert.equal(argon.thinkingCanDisable, true, 'Argon deve permitir alternar modo pensamento profundo');
});
