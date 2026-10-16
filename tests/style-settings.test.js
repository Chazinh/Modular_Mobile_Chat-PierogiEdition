import test from 'node:test';
import assert from 'node:assert/strict';
import { queueStyleSettings, flushStyleSettings } from '../client/src/style-settings.js';
import { api } from '../client/src/api.js';

test('style-settings: enfileira alterações, mescla propriedades e envia no flush', async () => {
  const calls = [];
  const origSave = api.saveGlobalSettings;
  api.saveGlobalSettings = async (patch) => {
    calls.push(patch);
    return { ok: true, saved: patch };
  };

  try {
    // Flush inicial sem alterações retorna null
    assert.equal(await flushStyleSettings(), null);
    assert.equal(calls.length, 0);

    // Enfileira patch 1 e patch 2
    queueStyleSettings({ botChars: 2000, theme: 'dark' });
    queueStyleSettings({ dialoguePct: 40, theme: 'iridescent' });

    // Flush deve mesclar os patches
    const res = await flushStyleSettings();
    assert.equal(calls.length, 1);
    assert.deepEqual(calls[0], {
      botChars: 2000,
      dialoguePct: 40,
      theme: 'iridescent',
    });
    assert.equal(res.ok, true);

    // Próximo flush sem nada enfileirado retorna null
    assert.equal(await flushStyleSettings(), null);
    assert.equal(calls.length, 1);
  } finally {
    api.saveGlobalSettings = origSave;
  }
});

test('style-settings: concorrência e preservação de pendências em caso de falha', async () => {
  const calls = [];
  const origSave = api.saveGlobalSettings;
  let shouldFail = true;

  api.saveGlobalSettings = async (patch) => {
    calls.push(patch);
    if (shouldFail) throw new Error('Falha de rede ao salvar configurações de estilo');
    return { ok: true, saved: patch };
  };

  try {
    queueStyleSettings({ fontSize: 16 });

    // Primeiro flush falha
    await assert.rejects(
      () => flushStyleSettings(),
      /Falha de rede ao salvar configurações de estilo/
    );
    assert.equal(calls.length, 1);

    // Como falhou, a pendência deve ter sido preservada
    // Adicionamos nova alteração
    queueStyleSettings({ lineHeight: 1.5 });

    // Permitir sucesso agora
    shouldFail = false;
    const res = await flushStyleSettings();
    assert.equal(calls.length, 2);
    assert.deepEqual(calls[1], {
      fontSize: 16,
      lineHeight: 1.5,
    });
    assert.equal(res.ok, true);
  } finally {
    api.saveGlobalSettings = origSave;
  }
});
