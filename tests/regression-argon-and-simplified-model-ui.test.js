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
