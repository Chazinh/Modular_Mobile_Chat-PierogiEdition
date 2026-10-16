import test from 'node:test';
import assert from 'node:assert/strict';
import { TOP_BAR_OPEN_SPRING, TOP_BAR_POP_END, topBarPopProgress, TopBarMotion } from '../client/src/top-bar-motion.js';

test('topBarPopProgress: calcula o progresso de estouro da bolha entre 0.5 e 0.7', () => {
  assert.equal(topBarPopProgress(0), 0);
  assert.equal(topBarPopProgress(0.2), 0);
  assert.equal(topBarPopProgress(0.5), 0);

  // Ponto intermediário 0.6 -> (0.6 - 0.5) / 0.2 = 0.5
  assert.ok(Math.abs(topBarPopProgress(0.6) - 0.5) < 1e-6);

  // Limite superior 0.7 -> 1.0
  assert.equal(topBarPopProgress(TOP_BAR_POP_END), 1);
  assert.equal(topBarPopProgress(0.8), 1);
  assert.equal(topBarPopProgress(1.5), 1);
});

function mockElement(rect = { left: 0, top: 0, width: 40, height: 40 }) {
  const styles = {};
  return {
    style: styles,
    getBoundingClientRect: () => rect,
    querySelectorAll: () => [],
  };
}

test('TopBarMotion: ciclo de vida, medição, arrasto com rubber-band e transições de estado', () => {
  const originalRAF = globalThis.requestAnimationFrame;
  const originalCAF = globalThis.cancelAnimationFrame;
  const originalMatchMedia = globalThis.matchMedia;

  const callbacks = [];
  globalThis.requestAnimationFrame = (fn) => {
    callbacks.push(fn);
    return callbacks.length;
  };
  globalThis.cancelAnimationFrame = () => {};
  globalThis.matchMedia = () => ({ matches: false });

  try {
    const bar = mockElement({ left: 0, top: 0, width: 360, height: 50 });
    const handle = mockElement({ left: 160, top: 10, width: 40, height: 20 });
    const bubble = mockElement();
    const ring = mockElement();

    const item1 = mockElement({ left: 50, top: 10, width: 30, height: 30 });
    const item2 = mockElement({ left: 280, top: 10, width: 30, height: 30 });
    bar.querySelectorAll = () => [item1, item2];

    const states = [];
    const progresses = [];

    const motion = new TopBarMotion({
      bar,
      handle,
      bubble,
      ring,
      onState: (s) => states.push(s),
      onProgress: (p) => progresses.push(p),
    });

    assert.equal(motion.state, 'closed');
    assert.equal(motion.p, 0);

    // Medição posiciona itens relativos ao centro do handle
    motion.measure();
    assert.equal(motion.items.length, 2);
    assert.ok(motion.center.x > 0);

    // Inicia arrasto para baixo a partir de y = 100
    motion.begin(100);
    assert.equal(motion.state, 'dragging');
    assert.equal(states.at(-1), 'dragging');

    // Move 65px para baixo (metade do RANGE de 130px -> p ~ 0.5)
    motion.move(165);
    assert.ok(motion.p >= 0.49 && motion.p <= 0.51, `p esperado próximo de 0.5, obtido ${motion.p}`);

    // Arrasto além de 1 (y = 100 + 200 = 300) aplica rubber band
    motion.move(300);
    assert.ok(motion.p > 1.0, 'p deve ultrapassar 1 com resistência');
    assert.ok(motion.p < 2.0, 'p não deve crescer linearmente devido ao rubber band');

    // Soltar o arrasto dispara settle em direção a 1 (aberto)
    motion.end();
    assert.equal(motion.state, 'opening');

    // Executa os frames de animação simulados
    while (callbacks.length > 0) {
      const fn = callbacks.shift();
      fn(performance.now());
    }

    // Fechar a barra dispara settle em direção a 0 (fechado)
    motion.close();
    assert.equal(motion.state, 'closing');

    // Destruição cancela e marca como morto
    motion.destroy();
    assert.equal(motion.dead, true);
  } finally {
    globalThis.requestAnimationFrame = originalRAF;
    globalThis.cancelAnimationFrame = originalCAF;
    if (originalMatchMedia) {
      globalThis.matchMedia = originalMatchMedia;
    } else {
      delete globalThis.matchMedia;
    }
  }
});

test('TopBarMotion: modo de movimento reduzido aplica meta instantaneamente', () => {
  const originalMatchMedia = globalThis.matchMedia;
  const originalCAF = globalThis.cancelAnimationFrame;
  globalThis.cancelAnimationFrame = () => {};
  globalThis.matchMedia = (query) => ({
    matches: query.includes('prefers-reduced-motion: reduce'),
  });

  try {
    const bar = mockElement();
    const handle = mockElement();
    const bubble = mockElement();
    const ring = mockElement();

    const states = [];
    const motion = new TopBarMotion({
      bar,
      handle,
      bubble,
      ring,
      onState: (s) => states.push(s),
    });

    // Ao chamar open(), deve abrir instantaneamente
    motion.open();
    assert.equal(motion.state, 'open');
    assert.equal(motion.p, 1);
    assert.equal(states.at(-1), 'open');

    // Ao chamar close(), deve fechar instantaneamente
    motion.close();
    assert.equal(motion.state, 'closed');
    assert.equal(motion.p, 0);
    assert.equal(states.at(-1), 'closed');
  } finally {
    globalThis.cancelAnimationFrame = originalCAF;
    if (originalMatchMedia) {
      globalThis.matchMedia = originalMatchMedia;
    } else {
      delete globalThis.matchMedia;
    }
  }
});
