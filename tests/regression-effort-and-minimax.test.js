import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_PRESETS } from '../client/src/default-presets.js';
import { mergeWithDefaults, normalizeChatPreset } from '../client/src/api.js';

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
        id: 'gemini-4-hydrogen',
        groupRoute: 'Hydrogen',
        efforts: ['low', 'medium', 'high'],
        routes: [{ id: 'Hydrogen' }],
        preferredRoutes: ['Hydrogen']
      },
      {
        id: 'generic',
        label: 'Outro modelo (sem controle de esforço)',
        routes: []
      }
    ]
  };

  const merged = mergeWithDefaults(staleCached);
  assert.ok(!merged.presets.some((p) => p.id === 'minimax-m31'), 'cache não deve reinjetar MiniMax removido');
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

  assert.equal(merged.presets.findIndex(p => p.id === 'minimax-m31'), -1);
});

test('Regressão: normalizeChatPreset normaliza chats com rota minimax mesmo com preset genérico', () => {
  const genericChat = { id: 'c1', name: 'Chat', presetId: 'generic', routeId: 'MiniMaxM3.1' };
  const normalized = normalizeChatPreset(genericChat);
  assert.equal(normalized.presetId, 'minimax-m31', 'Deve normalizar para minimax-m31');

  const openrouterChat = { id: 'c2', name: 'Chat 2', presetId: null, routeId: 'openrouter/minimax/minimax-m3.1-flash:free' };
  const normalized2 = normalizeChatPreset(openrouterChat);
  assert.equal(normalized2.presetId, 'minimax-m31', 'Deve normalizar rota openrouter minimax');
});

