import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { keepsKeyboard } from '../client/src/ui-helpers.js';

const read = path => fs.readFileSync(new URL(`../client/src/${path}`, import.meta.url), 'utf8');

test('regressão: tocar no histórico para rolar não fecha o teclado do chat', () => {
  const history = { closest: sel => (sel === '[data-keep-keyboard]' ? {} : null) };
  const outside = { closest: () => null };
  assert.equal(keepsKeyboard(history), true);
  assert.equal(keepsKeyboard(outside), false);
  assert.equal(keepsKeyboard(null), false);
  const chat = read('components/ChatView.jsx');
  assert.match(chat, /ref=\{scrollRef\}\s*data-keep-keyboard=""/, 'histórico marcado');
  assert.match(chat, /shellRef\.current\?\.contains\(event\.target\) \|\| keepsKeyboard\(event\.target\)\) return;/);
});

test('regressão: tela de IP/pareamento rola por inteiro com o teclado aberto', () => {
  const setup = read('components/ServerSetup.jsx');
  const root = setup.slice(setup.indexOf('  return (')).match(/<div className="([^"]+)"/)[1];
  assert.match(root, /overflow-y-auto/);
  // justify-center/items-center num container com rolagem esconde o topo quando o teclado encolhe a tela.
  assert.doesNotMatch(root, /justify-center|items-center/);
  assert.match(setup, /<form onSubmit=\{connect\} className="m-auto /);
});
