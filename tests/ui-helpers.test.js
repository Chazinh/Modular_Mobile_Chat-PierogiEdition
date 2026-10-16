import test from 'node:test';
import assert from 'node:assert/strict';
import { fold, findCast, timeAgo, copyText } from '../client/src/ui-helpers.js';

test('fold: normaliza acentuação Unicode (NFD), remove diacríticos e converte para minúsculas', () => {
  assert.equal(fold(''), '');
  assert.equal(fold(null), '');
  assert.equal(fold(undefined), '');
  assert.equal(fold('  Olá Mundo!  '), 'ola mundo!');
  assert.equal(fold('AÇÃO e REAÇÃO'), 'acao e reacao');
  assert.equal(fold('É âmbar à noite'), 'e ambar a noite');
  assert.equal(fold('Crème Brûlée'), 'creme brulee');
});

test('findCast: busca personagens por nome exato, apelidos e prefixos com insensibilidade a acentos', () => {
  const cast = [
    { name: 'Mira Arcanista', aliases: ['Maga', 'Arquimaga'] },
    { name: 'Nora Valente', aliases: ['Guerreira'] },
    { name: 'Lia' },
  ];

  // Casos nulos ou vazios
  assert.equal(findCast(null, 'Mira'), null);
  assert.equal(findCast(cast, ''), null);
  assert.equal(findCast(cast, null), null);

  // Busca por nome exato (mesmo com caixa ou acento diferente)
  const mira = findCast(cast, 'mira arcanista');
  assert.ok(mira);
  assert.equal(mira.name, 'Mira Arcanista');

  // Busca por apelido (alias)
  const porAlias = findCast(cast, 'maga');
  assert.ok(porAlias);
  assert.equal(porAlias.name, 'Mira Arcanista');

  // Busca por prefixo
  const porPrefixo = findCast(cast, 'Mira');
  assert.ok(porPrefixo);
  assert.equal(porPrefixo.name, 'Mira Arcanista');

  // Personagem inexistente
  assert.equal(findCast(cast, 'Desconhecido'), null);
});

test('timeAgo: formata tempos relativos em português e inglês corretamente', () => {
  const now = Date.now();

  // Menos de 60 segundos
  assert.equal(timeAgo(new Date(now - 10 * 1000).toISOString()), 'agora');

  // Minutos (1 a 59)
  assert.equal(timeAgo(new Date(now - 5 * 60 * 1000).toISOString()), '5 min');
  assert.equal(timeAgo(new Date(now - 59 * 60 * 1000).toISOString()), '59 min');

  // Horas (1 a 23)
  assert.equal(timeAgo(new Date(now - 2 * 3600 * 1000).toISOString()), '2 h');

  // Dias (1 a 29)
  assert.equal(timeAgo(new Date(now - 5 * 86400 * 1000).toISOString()), '5 d');

  // Meses e anos
  assert.ok(timeAgo(new Date(now - 35 * 86400 * 1000).toISOString()).includes('mês') || timeAgo(new Date(now - 35 * 86400 * 1000).toISOString()).includes('1'));

  // Nulo ou vazio
  assert.equal(timeAgo(''), '');
  assert.equal(timeAgo(null), '');
});

test('copyText: utiliza navigator.clipboard em contexto seguro ou textarea como fallback', async () => {
  const originalNavDesc = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const originalWin = globalThis.window;
  const originalDoc = globalThis.document;

  try {
    // 1. Contexto seguro com navigator.clipboard
    let written = '';
    Object.defineProperty(globalThis, 'navigator', {
      value: {
        clipboard: {
          writeText: async (t) => { written = t; },
        },
      },
      configurable: true,
      writable: true,
    });
    globalThis.window = { isSecureContext: true };

    const ok1 = await copyText('Texto copiado com sucesso');
    assert.equal(ok1, true);
    assert.equal(written, 'Texto copiado com sucesso');

    // 2. Contexto não seguro -> fallback com document.createElement('textarea')
    globalThis.window = { isSecureContext: false };
    let execCommandCalled = false;
    let appendedTa = null;

    globalThis.document = {
      body: {
        appendChild: (el) => { appendedTa = el; },
        removeChild: (el) => { if (appendedTa === el) appendedTa = null; },
      },
      createElement: (tag) => ({
        tagName: tag,
        value: '',
        style: {},
        setAttribute: () => {},
        select: () => {},
        setSelectionRange: () => {},
      }),
      execCommand: (cmd) => {
        if (cmd === 'copy') {
          execCommandCalled = true;
          return true;
        }
        return false;
      },
    };

    const ok2 = await copyText('Fallback textarea');
    assert.equal(ok2, true);
    assert.equal(execCommandCalled, true);
  } finally {
    if (originalNavDesc) {
      Object.defineProperty(globalThis, 'navigator', originalNavDesc);
    } else {
      delete globalThis.navigator;
    }

    if (originalWin) globalThis.window = originalWin;
    else delete globalThis.window;

    if (originalDoc) globalThis.document = originalDoc;
    else delete globalThis.document;
  }
});
