import test from 'node:test';
import assert from 'node:assert/strict';
import { readSSE, api, publicPresets } from '../client/src/api.js';
import { privateSet } from '../client/src/private-storage.js';
import { gestureAxis } from '../client/src/gestures.js';
import { queueStyleSettings, flushStyleSettings } from '../client/src/style-settings.js';

test('SSE: CRLF fragmentado, Unicode e terminal explícito; EOF incompleto exige reconexão', async () => {
  const bytes = new TextEncoder().encode('data: {"delta":"🚀 **fala**"}\r\n\r\ndata: {"done":true}\r\n\r\n');
  const events = [];
  const stream = new ReadableStream({ start(c) {
    for (let i = 0; i < bytes.length; i += 3) c.enqueue(bytes.slice(i, i + 3));
    c.close();
  } });
  await readSSE(new Response(stream), e => events.push(e), true);
  assert.deepEqual(events, [{ delta: '🚀 **fala**' }, { done: true }]);
  await assert.rejects(readSSE(new Response('data: {"delta":"parcial"}\n\n'), () => {}, true), /Fluxo interrompido/);
  await assert.rejects(readSSE(new Response('data: {"done":true}\n\n'), () => { throw Error('callback'); }, true), /callback/);
});

test('presets: mobile-local/mobile-api sobrevivem à whitelist e não expõem presets internos', () => {
  assert.deepEqual(publicPresets({ presets: [{ id: 'mobile-local' }, { id: 'mobile-api' }, { id: 'private-provider' }] }).presets,
    [{ id: 'mobile-local' }, { id: 'mobile-api' }]);
});

test('presets: cache nativo mobile não é substituído pelos defaults PC', async () => {
  const previous = globalThis.localStorage;
  const mobile = { presets: [{ id: 'mobile-local', label: 'Local' }, { id: 'mobile-api', label: 'API' }] };
  const saved = new Map([['mc.kv.presets', JSON.stringify(mobile)]]);
  globalThis.localStorage = { getItem: k => saved.get(k) || null, setItem: (k, v) => saved.set(k, v), removeItem: k => saved.delete(k) };
  try { assert.deepEqual(await api.cachedPresets(), mobile); }
  finally { if (previous === undefined) delete globalThis.localStorage; else globalThis.localStorage = previous; }
});

test('privacidade: checkpoint de sync nunca cai no localStorage plaintext quando proteção indisponível', async () => {
  await assert.rejects(privateSet('mc.sync.fixture', '{"hash":"marker"}'), /armazenamento protegido/);
});

test('gestos: diagonais com projeção vertical >= 50% pertencem à leitura, em ambos os sentidos', () => {
  for (const sign of [-1, 1]) {
    assert.equal(gestureAxis(80, sign * 50), 'vertical');
    assert.equal(gestureAxis(-80, sign * 50), 'vertical');
    assert.equal(gestureAxis(90, sign * 40), 'horizontal');
  }
  assert.equal(gestureAxis(2, 3), null);
});

test('salvamento de estilo: fila preserva a última posição, serializa PUTs e retém falhas', async () => {
  const original = api.saveGlobalSettings;
  const calls = [];
  let release;
  api.saveGlobalSettings = async body => {
    calls.push(body);
    if (calls.length === 1) await new Promise(r => { release = r; });
    return { settings: body };
  };
  try {
    queueStyleSettings({ dialoguePct: 10 });
    const first = flushStyleSettings();
    queueStyleSettings({ dialoguePct: 90, botChars: 3000 });
    const second = flushStyleSettings();
    release();
    await Promise.all([first, second]);
    assert.deepEqual(calls, [{ dialoguePct: 10 }, { dialoguePct: 90, botChars: 3000 }]);
    api.saveGlobalSettings = async () => { throw Error('offline'); };
    queueStyleSettings({ dialoguePct: 0 });
    await assert.rejects(flushStyleSettings(), /offline/);
    api.saveGlobalSettings = async body => ({ settings: body });
    assert.deepEqual((await flushStyleSettings()).settings, { dialoguePct: 0 });
  } finally { api.saveGlobalSettings = original; }
});
