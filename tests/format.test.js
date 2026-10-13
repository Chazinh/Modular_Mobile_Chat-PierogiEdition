import test from 'node:test';
import assert from 'node:assert/strict';
import { parseScene, parseLine, inlineSegments, plainText, speakersOf } from '../client/src/format.js';

const EXAMPLE = [
  '*Mira arruma o cabelo e olha para Nora com um sorriso divertido.*',
  '',
  'Mira: Nora... pelo amor de Deus, para. (Tom divertido)',
  '',
  '*Nora cruza os braços, tentando conter o riso.*',
  '',
  'Nora: Eu nem comecei.',
].join('\n');

test('formato do plano: ações em itálico, falas com nome e tom opcional', () => {
  const b = parseScene(EXAMPLE);
  assert.deepEqual(b.map((x) => x.kind), ['action', 'speech', 'action', 'speech']);
  assert.equal(b[0].segments[0].i, true);
  assert.equal(b[0].segments[0].t, 'Mira arruma o cabelo e olha para Nora com um sorriso divertido.');
  assert.equal(b[1].name, 'Mira');
  assert.equal(b[1].tone, 'Tom divertido');
  assert.equal(b[1].segments.map((s) => s.t).join(''), 'Nora... pelo amor de Deus, para.');
  assert.equal(b[3].name, 'Nora');
  assert.equal(b[3].tone, null);
  assert.deepEqual(speakersOf(EXAMPLE), ['Mira', 'Nora']);
});

test('ações intercaladas dentro da fala continuam em itálico', () => {
  const l = parseLine('Mira: Eu *respiro fundo* e continuo.');
  assert.equal(l.kind, 'speech');
  assert.deepEqual(l.segments, [{ i: false, t: 'Eu ' }, { i: true, t: 'respiro fundo' }, { i: false, t: ' e continuo.' }]);
});

test('streaming: marcador sem par não expõe asteriscos e abre itálico até o fim da linha', () => {
  const partial = parseScene('*Mira arruma o cab');
  assert.equal(partial[0].kind, 'action');
  assert.equal(partial[0].segments[0].t, 'Mira arruma o cab');
  assert.doesNotMatch(JSON.stringify(partial), /\*/);
  // when the pair arrives, the result has the same style (no jump)
  const full = parseScene('*Mira arruma o cabelo.*');
  assert.equal(full[0].kind, 'action');
  const mid = parseScene('Mira: Oi *ela sorri');
  assert.deepEqual(mid[0].segments.map((s) => s.i), [false, true]);
  assert.doesNotMatch(JSON.stringify(mid), /\*/);
  // a double asterisk becomes a single marker
  assert.deepEqual(inlineSegments('**forte** texto'), [{ i: true, t: 'forte' }, { i: false, t: ' texto' }]);
});

test('nome exposto em negrito/itálico também é reconhecido como fala', () => {
  assert.equal(parseLine('**Nora:** Oi!').name, 'Nora');
  assert.equal(parseLine('*Nora*: Oi!').name, 'Nora');
  assert.equal(parseLine('Dona Maria: Bom dia.').name, 'Dona Maria');
});

test('narração com dois pontos não vira fala; HTML não é interpretado (só dados)', () => {
  const l = parseLine('*Ele murmurou algo: não importa.*');
  assert.equal(l.kind, 'action');
  const html = parseLine('Eve: <img src=x onerror=alert(1)> **oi**');
  assert.equal(html.kind, 'speech');
  assert.ok(html.segments.some((s) => s.t.includes('<img src=x onerror=alert(1)>')), 'texto literal, escapado pelo React na renderização');
  assert.equal(plainText('*Ela sorri.* Eve: oi'), 'Ela sorri. Eve: oi');
});

test('parágrafos e linhas vazias são preservados na ordem', () => {
  const b = parseScene('*A.*\n\n\n*B.*\nVic: C');
  assert.deepEqual(b.map((x) => x.kind), ['action', 'action', 'speech']);
});
