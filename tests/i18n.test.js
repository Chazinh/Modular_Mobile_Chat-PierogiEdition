import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const values = new Map([['mc.language', 'en']]);
globalThis.localStorage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
globalThis.document = { documentElement: { lang: '' } };
globalThis.window = new EventTarget();
const { tr, currentLanguage, setLanguage, syncNativeLanguage } = await import('../client/src/i18n.js');

test('idioma local inicia salvo, muda ao vivo e preserva interpolação/conteúdo', () => {
  assert.equal(currentLanguage(), 'en');
  assert.equal(document.documentElement.lang, 'en');
  assert.equal(tr('Configurações'), 'Settings');
  const authored = 'Mira: *Olá.* {1} “Configurações”';
  assert.equal(tr('Ver foto de {0}', [authored]), `View ${authored}'s photo`);
  assert.equal(tr(authored), authored);
  assert.equal(tr('\n  Configurações  '), '\n  Settings  ');
  setLanguage('pt-BR');
  assert.equal(tr('Configurações'), 'Configurações');
  assert.equal(values.get('mc.language'), 'pt-BR');
  assert.equal(document.documentElement.lang, 'pt-BR');
  setLanguage('unsupported');
  assert.equal(currentLanguage(), 'pt-BR');
  setLanguage('en');
});

test('alteração em outra aba e armazenamento indisponível mantêm interface utilizável', async () => {
  const storage = new Event('storage');
  Object.assign(storage, { key: 'mc.language', newValue: 'pt-BR' });
  window.dispatchEvent(storage);
  assert.equal(currentLanguage(), 'pt-BR');
  assert.equal(document.documentElement.lang, 'pt-BR');
  localStorage.setItem = () => { throw new Error('private mode'); };
  setLanguage('en');
  assert.equal(tr('Cancelar'), 'Cancel');
  await syncNativeLanguage();
});

test('traduções preservam todos os parâmetros e não contêm endpoint particular', async () => {
  const dictionary = JSON.parse(await fs.readFile(new URL('../client/src/locales/en.json', import.meta.url), 'utf8'));
  const placeholders = text => [...text.matchAll(/\{(\d+)\}/g)].map(match => match[1]).sort();
  for (const [key, value] of Object.entries(dictionary)) assert.deepEqual(placeholders(value), placeholders(key), key);
  assert.ok(!JSON.stringify(dictionary).includes('192.168.18.10'));
});

test('4B localhost: estados, opt-in e limitação OpenCode traduzidos ao vivo sem alterar IDs/endpoints', () => {
  setLanguage('en');
  const states = ['Disponível', 'Exige autenticação', 'Não detectado', 'Resposta incompatível', 'API própria detectada', 'Detectando…', 'Atualizar detecção'];
  for (const source of states) assert.notEqual(tr(source), source, source);
  assert.equal(tr('IDs detectados: {0}', ['mobile-api:9router-local:real-model']), 'Detected IDs: mobile-api:9router-local:real-model');
  const limit = 'OpenCode serve usa API própria, incompatível com OpenAI chat completions. Este host não implementa inferência nativa do OpenCode.';
  assert.match(tr(limit), /does not implement native OpenCode inference/);
  const label = 'Autorizar loopback neste aparelho (127/8, ::1 ou localhost exato)';
  assert.match(tr(label), /127\/8, ::1 or exact localhost/);
  setLanguage('pt-BR');
  assert.equal(tr(limit), limit); assert.equal(tr(label), label);
  setLanguage('en');
});
