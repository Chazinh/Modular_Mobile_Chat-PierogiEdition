import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_PRESETS } from '../client/src/default-presets.js';
import { mergeWithDefaults, normalizeChatPreset } from '../client/src/api.js';

test('Regressão: MiniMax M3.1 deve estar completo com rotas 9Router e controle de esforço', () => {
  const minimax = DEFAULT_PRESETS.presets.find((p) => p.id === 'minimax-m31');
  assert.ok(minimax, 'minimax-m31 deve existir em DEFAULT_PRESETS');
  assert.equal(minimax.groupRoute, 'MiniMaxM3.1', 'MiniMax deve ter groupRoute MiniMaxM3.1');
  assert.ok(minimax.routes.some((r) => r.id === 'MiniMaxM3.1'), 'Deve ter rota de grupo MiniMaxM3.1');
  assert.ok(minimax.routes.some((r) => r.id === 'openrouter/minimax/minimax-m3.1-flash:free'), 'Deve ter rota openrouter');
  assert.ok(minimax.routes.some((r) => r.id === 'oc/minimax-m3.1-flash-free'), 'Deve ter rota oc');
  assert.ok(minimax.preferredRoutes.includes('oc/minimax-m3.1-flash-free'), 'preferredRoutes deve incluir oc');
  assert.deepEqual(minimax.efforts, ['low', 'medium', 'high'], 'MiniMax deve suportar low, medium, high');
  assert.equal(minimax.thinkingCanDisable, true, 'MiniMax deve permitir desligar pensamento');
  assert.notEqual(minimax.thinkingDefault, false, 'MiniMax não deve iniciar com pensamento desligado');
});

test('Regressão: Modelos com pensamento profundo devem ter esforços low, medium, high e não iniciar desligados', () => {
  const models = ['deepseek-v41-flash', 'muse-spark-13', 'nemotron-35', 'minimax-m31'];
  for (const id of models) {
    const p = DEFAULT_PRESETS.presets.find((x) => x.id === id);
    assert.ok(p, `${id} deve existir`);
    assert.ok(p.efforts.includes('low') && p.efforts.includes('medium') && p.efforts.includes('high'), `${id} deve ter low, medium, high`);
    assert.equal(p.thinkingCanDisable, true, `${id} deve ter thinkingCanDisable: true`);
    assert.notEqual(p.thinkingDefault, false, `${id} não deve ter thinkingDefault: false`);
  }
});

test('Regressão: Gemini 3.8 Flash deve suportar low, medium, high e rota de grupo', () => {
  const gemini = DEFAULT_PRESETS.presets.find((p) => p.id === 'gemini-38-flash');
  assert.ok(gemini, 'gemini-38-flash deve existir');
  assert.equal(gemini.groupRoute, 'Gemini');
  assert.deepEqual(gemini.efforts, ['low', 'medium', 'high']);
  assert.ok(gemini.routes.some((r) => r.id === 'Gemini'));
});

test('Regressão: mergeWithDefaults recupera modelos novos e atualiza cache desatualizado do IndexedDB', () => {
  // Simula cache antigo do IndexedDB sem minimax e com generic já no final
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
        id: 'gemini-38-flash',
        groupRoute: 'Gemini',
        efforts: ['low', 'medium', 'high'],
        routes: [{ id: 'Gemini' }],
        preferredRoutes: ['Gemini']
      },
      {
        id: 'generic',
        label: 'Outro modelo (sem controle de esforço)',
        routes: []
      }
    ]
  };

  const merged = mergeWithDefaults(staleCached);
  assert.ok(merged.presets.some((p) => p.id === 'minimax-m31'), 'mergeWithDefaults deve injetar minimax-m31');
  const ds = merged.presets.find((p) => p.id === 'deepseek-v41-flash');
  assert.deepEqual(ds.efforts, ['low', 'medium', 'high'], 'mergeWithDefaults deve atualizar lista de esforços');
  assert.ok(ds.routes.some((r) => r.id === 'oc/deepseek-v4.1-flash-free'), 'mergeWithDefaults deve sincronizar rotas');
});

test('Regressão: Outro modelo (generic) SEMPRE deve ficar em último lugar na lista de presets', () => {
  const staleServerData = {
    presets: [
      { id: 'deepseek-v41-flash', label: 'DeepSeek' },
      { id: 'generic', label: 'Outro modelo' }
    ]
  };

  const merged = mergeWithDefaults(staleServerData);
  const lastPreset = merged.presets[merged.presets.length - 1];
  assert.equal(lastPreset.id, 'generic', '"generic" (Outro modelo) deve ser obrigatoriamente o último item da lista');

  const minimaxIndex = merged.presets.findIndex(p => p.id === 'minimax-m31');
  const genericIndex = merged.presets.findIndex(p => p.id === 'generic');
  assert.ok(minimaxIndex !== -1, 'minimax-m31 deve estar presente');
  assert.ok(minimaxIndex < genericIndex, 'minimax-m31 deve aparecer antes de "generic" (Outro modelo)');
});

test('Regressão: normalizeChatPreset normaliza chats com rota minimax mesmo com preset genérico', () => {
  const genericChat = { id: 'c1', name: 'Chat', presetId: 'generic', routeId: 'MiniMaxM3.1' };
  const normalized = normalizeChatPreset(genericChat);
  assert.equal(normalized.presetId, 'minimax-m31', 'Deve normalizar para minimax-m31');

  const openrouterChat = { id: 'c2', name: 'Chat 2', presetId: null, routeId: 'openrouter/minimax/minimax-m3.1-flash:free' };
  const normalized2 = normalizeChatPreset(openrouterChat);
  assert.equal(normalized2.presetId, 'minimax-m31', 'Deve normalizar rota openrouter minimax');
});

