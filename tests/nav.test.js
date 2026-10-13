import test from 'node:test';
import assert from 'node:assert/strict';

// Simulated navigation history: asynchronous traversals as in the browser/WebView.
const listeners = [];
const fake = {
  entries: [{ state: null }],
  index: 0,
  swallow: false, // simulates a WebView that does not deliver popstate
  get state() {
    return this.entries[this.index].state;
  },
  pushState(state) {
    this.entries.splice(this.index + 1);
    this.entries.push({ state });
    this.index++;
  },
  replaceState(state) { this.entries[this.index].state = state; },
  go(n) {
    queueMicrotask(() => {
      const target = this.index + n;
      if (target < 0 || target >= this.entries.length) return;
      this.index = target;
      if (!this.swallow) for (const fn of listeners) fn({ state: this.state });
    });
  },
};
globalThis.window = { addEventListener: (type, fn) => type === 'popstate' && listeners.push(fn) };
globalThis.history = fake;

const { openLayer, closeTopLayer, layerCount } = await import('../client/src/nav.js');
const tick = () => new Promise((r) => setTimeout(r, 5));
const reset = async () => {
  while (closeTopLayer());
  await tick();
  fake.swallow = false;
  fake.entries = [{ state: null }];
  fake.index = 0;
};
const mk = (name, log) => openLayer(() => log.push(name));

test('trocar o menu pelo editor no mesmo commit não fecha o editor com um Voltar atrasado', async () => {
  await reset();
  const log = [];
  mk('chat', log);
  const dismissMenu = mk('menu', log);
  dismissMenu();
  mk('editor', log);
  await tick();
  assert.equal(layerCount(), 2);
  assert.equal(fake.index, 2);
  assert.deepEqual(log, []);
  closeTopLayer();
  await tick();
  assert.deepEqual(log, ['editor']);
  assert.equal(layerCount(), 1);
});

test('Voltar fecha a camada do topo, uma por vez, e o histórico volta ao início', async () => {
  await reset();
  const log = [];
  mk('a', log);
  mk('b', log);
  assert.equal(layerCount(), 2);
  assert.equal(fake.index, 2);
  assert.equal(closeTopLayer(), true);
  await tick();
  assert.deepEqual(log, ['b']);
  assert.equal(closeTopLayer(), true);
  await tick();
  assert.deepEqual(log, ['b', 'a']);
  assert.equal(closeTopLayer(), false, 'sem camadas: o APK minimiza o app');
  assert.equal(fake.index, 0);
});

test('Voltar pelo gesto/histórico (popstate) fecha o topo', async () => {
  await reset();
  const log = [];
  mk('a', log);
  mk('b', log);
  fake.go(-1);
  await tick();
  assert.deepEqual(log, ['b']);
  assert.equal(layerCount(), 1);
  fake.go(-1);
  await tick();
  assert.deepEqual(log, ['b', 'a']);
  assert.equal(layerCount(), 0);
});

test('fechar uma camada antiga por código fecha também as mais novas, sem engolir o próximo Voltar', async () => {
  await reset();
  const log = [];
  const dismissA = mk('a', log);
  mk('b', log);
  mk('c', log);
  dismissA(); // e.g. layer A's component unmounted with B and C still open
  await tick();
  assert.equal(layerCount(), 0);
  assert.deepEqual(log.sort(), ['b', 'c']);
  assert.equal(fake.index, 0, 'uma única navegação desfez as três entradas');
  // the next Back must not be "swallowed": a new layer + Back closes normally
  const log2 = [];
  mk('d', log2);
  fake.go(-1);
  await tick();
  assert.deepEqual(log2, ['d']);
});

test('se o WebView não entregar o popstate, o botão Voltar ainda fecha a camada (sem interface presa)', async () => {
  await reset();
  fake.swallow = true;
  const log = [];
  mk('a', log);
  mk('b', log);
  assert.equal(closeTopLayer(), true);
  assert.deepEqual(log, ['b'], 'fechou direto, sem depender do evento');
  assert.equal(closeTopLayer(), true);
  assert.deepEqual(log, ['b', 'a']);
  assert.equal(layerCount(), 0);
  await tick();
  fake.swallow = false;
  // and the following navigation stays normal
  const log2 = [];
  mk('c', log2);
  fake.go(-1);
  await tick();
  assert.deepEqual(log2, ['c']);
});

test('Voltar em rajada e callback que lança erro não deixam camadas abertas', async () => {
  await reset();
  const log = [];
  mk('a', log);
  openLayer(() => {
    throw new Error('falha ao fechar');
  });
  mk('c', log);
  closeTopLayer();
  closeTopLayer();
  closeTopLayer();
  await tick();
  assert.deepEqual(log, ['c', 'a']);
  assert.equal(layerCount(), 0);
  assert.equal(fake.index, 0);
});

test('nunca navega além da profundidade conhecida (não sai da página por engano)', async () => {
  await reset();
  const dismiss = openLayer(() => {});
  dismiss();
  dismiss(); // a repeated call is harmless
  await tick();
  assert.equal(fake.index, 0);
  assert.equal(closeTopLayer(), false);
  await tick();
  assert.equal(fake.index, 0);
});
