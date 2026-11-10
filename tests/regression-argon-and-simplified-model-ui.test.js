import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { DEFAULT_PRESETS } from '../client/src/default-presets.js';
import { publicPresets } from '../client/src/api.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const projectRoot = path.resolve(__dirname, '..');

test('Argon Profile: Preset definition, low effort only, and thinking controls', () => {
  // 1. DEFAULT_PRESETS no cliente
  const argon = DEFAULT_PRESETS.presets.find(p => p.id === 'gemini-4-argon-medium');
  assert.ok(argon, 'Preset Argon deve existir nos DEFAULT_PRESETS');
  assert.equal(argon.label, 'Argon', 'Label deve ser "Argon"');
  assert.equal(argon.groupRoute, 'Argon', 'groupRoute deve ser "Argon"');
  assert.ok(argon.preferredRoutes.includes('ag/gemini-4-argon-medium'), 'preferredRoutes deve incluir ag/gemini-4-argon-medium');
  assert.ok(argon.routes.some(r => r.id === 'ag/gemini-4-argon-medium'), 'routes deve incluir ag/gemini-4-argon-medium');
  assert.deepEqual(argon.efforts, ['low'], 'Argon deve aceitar apenas esforço "low"');
  assert.equal(argon.defaultEffort, 'low', 'defaultEffort deve ser "low"');
  assert.equal(argon.thinkingCanDisable, true, 'thinkingCanDisable deve ser true para permitir controle de pensamento profundo');

  // 2. presets.json do servidor Rust
  const presetsJsonPath = path.join(projectRoot, 'server-rust', 'src', 'presets.json');
  const presetsJson = JSON.parse(fs.readFileSync(presetsJsonPath, 'utf8'));
  const argonRust = presetsJson.find(p => p.id === 'gemini-4-argon-medium');
  assert.ok(argonRust, 'Preset Argon deve existir no server-rust/src/presets.json');
  assert.equal(argonRust.label, 'Argon');
  assert.equal(argonRust.contextWindow, 1000000, 'Context window deve ser 1.0M (1000000)');
  assert.equal(argonRust.maxOutput, 262144, 'Max output tokens deve ser 262k (262144)');
  assert.ok(argonRust.unlistedRoutes?.includes('ag/gemini-4-argon-medium'), 'unlistedRoutes deve incluir ag/gemini-4-argon-medium');
  assert.ok(argonRust.match?.some(m => m.includes('argon')), 'match deve incluir padrões para rota argon');
  assert.deepEqual(argonRust.efforts, ['low'], 'Server preset deve ter esforço low somente');
  assert.equal(argonRust.defaultEffort, 'low');
  assert.equal(argonRust.thinkingCanDisable, true);

  // 3. publicPresets exporta o perfil
  const pub = publicPresets(DEFAULT_PRESETS);
  const pubArgon = pub.presets.find(p => p.id === 'gemini-4-argon-medium');
  assert.ok(pubArgon, 'Argon deve estar visível em publicPresets');
  assert.equal(pubArgon.label, 'Argon');
});

test('Gemini 3.8 Flash: Suporte ao controle de Modo Pensamento Profundo', () => {
  const gemini = DEFAULT_PRESETS.presets.find(p => p.id === 'gemini-38-flash');
  assert.ok(gemini, 'Gemini 3.8 Flash deve existir nos presets padrão');
  assert.equal(gemini.thinkingCanDisable, true, 'Gemini 3.8 Flash deve ter thinkingCanDisable=true');

  const presetsJsonPath = path.join(projectRoot, 'server-rust', 'src', 'presets.json');
  const presetsJson = JSON.parse(fs.readFileSync(presetsJsonPath, 'utf8'));
  const geminiRust = presetsJson.find(p => p.id === 'gemini-38-flash');
  assert.ok(geminiRust);
  assert.equal(geminiRust.thinkingCanDisable, true);
});

test('SettingsDrawer UI: Simplificação sem seleção visual de rotas', () => {
  const settingsDrawerPath = path.join(projectRoot, 'client', 'src', 'components', 'SettingsDrawer.jsx');
  const src = fs.readFileSync(settingsDrawerPath, 'utf8');

  // Não deve renderizar a lista de rotas ou radio buttons com 'Rota ou grupo de modelos'
  assert.ok(!src.includes('tr("Rota ou grupo de modelos")'), 'Não deve exibir label de seleção de rotas');
  assert.ok(!src.includes('p.routes.map'), 'Não deve iterar e exibir botões de rota visualmente no ModelPage');

  // Deve exibir Modo pensamento profundo e Nível de esforço
  assert.ok(src.includes('tr("Modo pensamento profundo")'), 'Deve exibir chave de Modo pensamento profundo');
  assert.ok(src.includes('tr("Nível de esforço")'), 'Deve exibir label Nível de esforço');
});
