import test from 'node:test';
import assert from 'node:assert/strict';
import { rubber, unrubber, follow, springStep, deformation, releaseTarget, VelocityTracker, EDGE_PX, PanelMotion, Staircase, frameDt, pullEvent, attachMotion, detachMotion, resetPull } from '../client/src/drawer-motion.js';

test('borracha: cresce cada vez menos, nunca passa do limite e é invertível', () => {
  assert.equal(rubber(0, 84), 0);
  assert.equal(rubber(-5, 84), 0);
  let prev = 0;
  for (const d of [10, 40, 120, 400, 5000]) {
    const y = rubber(d, 84);
    assert.ok(y > prev && y < 84, `rubber(${d})=${y}`);
    prev = y;
  }
  for (const d of [1, 20, 90, 300]) assert.ok(Math.abs(unrubber(rubber(d, 84), 84) - d) < 1e-6, `inversa em ${d}`);
});

test('segue o dedo: dentro do curso é 1:1; além do aberto resiste; além do fechado para', () => {
  assert.equal(follow(120, 360), 120);
  assert.equal(follow(0, 360), 0);
  assert.equal(follow(900, 360), 360);
  const over = follow(-60, 360);
  assert.ok(over < 0 && over > -60, `resistência além do aberto: ${over}`);
  assert.ok(follow(-4000, 360) > -84, 'nunca passa do limite de overscroll');
});

test('mola: abre com leve overshoot e assenta; fechar crítico não oscila', () => {
  let s = { x: 360, v: 0 };
  let min = Infinity;
  for (let i = 0; i < 400; i++) { s = springStep(s, 0, 1 / 120); min = Math.min(min, s.x); }
  assert.ok(min < 0 && min > -40, `overshoot leve (${min.toFixed(1)}px)`);
  assert.ok(Math.abs(s.x) < 0.05 && Math.abs(s.v) < 1, 'assenta em 0');
  let c = { x: 0, v: 0 };
  let max = -Infinity;
  const critical = 2 * Math.sqrt(520);
  for (let i = 0; i < 400; i++) { c = springStep(c, 100, 1 / 120, 520, critical); max = Math.max(max, c.x); }
  assert.ok(max <= 100.5, `sem overshoot visível no amortecimento crítico (${max.toFixed(2)})`);
});

test('mola é estável com quadros longos (subpassos)', () => {
  let s = { x: 360, v: 0 };
  for (let i = 0; i < 100; i++) s = springStep(s, 0, 0.05);
  assert.ok(Number.isFinite(s.x) && Math.abs(s.x) < 1);
});

test('deformação leve: alonga ao puxar além do aberto e achata com a velocidade, sempre pequena', () => {
  const rest = deformation(0, 0);
  assert.deepEqual(rest, { along: 1, across: 1 });
  const pulled = deformation(-84, 0);
  assert.ok(pulled.along > 1 && pulled.along <= 1.06 && pulled.across < 1 && pulled.across >= 0.98);
  const fast = deformation(120, 6000);
  assert.ok(fast.along <= 1.035 && fast.across >= 0.985);
});

test('solta o dedo: arremesso decide; senão a posição projetada', () => {
  assert.equal(releaseTarget(300, 900, 360), 'closed');
  assert.equal(releaseTarget(40, -900, 360), 'open');
  assert.equal(releaseTarget(100, 0, 360), 'open');
  assert.equal(releaseTarget(260, 0, 360), 'closed');
  assert.equal(releaseTarget(150, 100, 360), 'open');
  assert.equal(releaseTarget(150, 200, 360), 'closed', 'a posição projetada (150 + 200*0,2 = 190) passa da metade');
});

test('velocidade pelas últimas amostras', () => {
  const t = new VelocityTracker();
  t.push(0, 0); t.push(10, 10); t.push(30, 20); t.push(60, 30);
  assert.ok(Math.abs(t.velocity() - 2000) < 1, `v=${t.velocity()}`); // 60 px in 30 ms (all samples fit in the 90 ms window)
  t.reset();
  assert.equal(t.velocity(), 0);
});

test('faixa de ativação rente à borda é estreita (alça do painel Edge)', () => {
  assert.ok(EDGE_PX >= 12 && EDGE_PX <= 28);
});

test('gaveta: evento atrasado não bloqueia o botão; gesto iniciado continua anexando', () => {
  resetPull();
  const calls = [], motion = { begin: x => calls.push(['begin', x]), move: x => calls.push(['move', x]), end: () => calls.push(['end']) };
  pullEvent('move', 200);
  pullEvent('end');
  assert.equal(attachMotion(motion), false);
  assert.deepEqual(calls, []);
  detachMotion(motion);
  pullEvent('begin', 360); pullEvent('move', 300); pullEvent('end');
  assert.equal(attachMotion(motion), true);
  assert.deepEqual(calls, [['begin', 360], ['move', 300], ['end']]);
  detachMotion(motion);
});

test('gaveta: fechar na entrada não herda impulso de abertura e destruir impede reanimação', () => {
  const originalRAF = globalThis.requestAnimationFrame, originalCancel = globalThis.cancelAnimationFrame;
  let scheduled = 0;
  globalThis.requestAnimationFrame = () => ++scheduled;
  globalThis.cancelAnimationFrame = () => {};
  const panel = { style: {}, classList: { add() {}, remove() {} } };
  try {
    const motion = new PanelMotion({ panel, closed: 360 });
    motion.open();
    motion.v = -800;
    motion.close();
    assert.equal(motion.v, 0, 'fechamento não volta na direção de abertura');
    const before = scheduled;
    motion.destroy();
    motion.begin(300); motion.move(200); motion.end(); motion.open(); motion.close();
    assert.equal(scheduled, before);
    assert.equal(motion.drag, null);
  } finally {
    if (originalRAF === undefined) delete globalThis.requestAnimationFrame; else globalThis.requestAnimationFrame = originalRAF;
    if (originalCancel === undefined) delete globalThis.cancelAnimationFrame; else globalThis.cancelAnimationFrame = originalCancel;
  }
});

test('primeiro quadro após soltar o dedo anda um quadro (sem dt negativo congelando a animação)', () => {
  assert.equal(frameDt(1000, 0), 1 / 60);
  assert.equal(frameDt(1000, 1005), 0, 'carimbo do rAF anterior ao agendamento não anda para trás');
  assert.equal(frameDt(1016, 1000), 0.016);
  assert.equal(frameDt(5000, 1000), 0.05, 'quadro longo limitado');
});

test('escadinha: ao abrir, cada linha de cima para baixo persegue o painel com mola mais macia e todas alcançam', () => {
  const g = globalThis;
  const saved = { window: g.window, raf: g.requestAnimationFrame };
  g.window = { innerHeight: 800 };
  g.requestAnimationFrame = undefined; // the test drives the frames
  try {
    const items = [0, 1, 2, 3].map((i) => ({ style: {}, getBoundingClientRect: () => ({ top: i * 60, bottom: i * 60 + 50, height: 50 }) }));
    const offscreen = { style: {}, getBoundingClientRect: () => ({ top: 900, bottom: 950, height: 50 }) };
    const stair = new Staircase(() => [...items, offscreen]);
    const shift = (el) => Number(/translate3d\(([\d.]+)px/.exec(el.style.transform || 'translate3d(0px')[1]);
    stair.push(0, 400, 1000);
    for (let t = 1016, p = 0.15; p <= 1; t += 16, p += 0.15) stair.push(Math.min(1, p), 400, t);
    for (let i = 1; i < items.length; i++) assert.ok(shift(items[i]) >= shift(items[i - 1]), `degrau ${i}: o de baixo nunca adianta`);
    assert.ok(shift(items[0]) < shift(items[1]) && shift(items[1]) < shift(items[2]), 'escada visível de cima para baixo');
    for (let i = 1; i < items.length; i++) assert.ok(Number(items[i].style.opacity || 1) <= Number(items[i - 1].style.opacity || 1), `degrau ${i}: surge depois do de cima`);
    assert.ok(Math.abs(shift(items[3]) - stair.p * 400) < 0.01, 'a última linha ainda espera a vez, no ponto de partida');
    assert.equal(items[3].style.opacity, '0.000', 'esperando a vez fica invisível');
    assert.ok(items.every((el) => el.style.willChange), 'camada só durante o movimento');
    assert.equal(offscreen.style.transform, undefined, 'fora da tela não é animado');
    // smooth: between consecutive frames no row jumps more than the panel itself moved
    let prev = items.map(shift);
    for (let t = 1200; t < 1260; t += 16) {
      stair.draw(t);
      const now = items.map(shift);
      assert.ok(now.every((x, i) => x <= prev[i] + 0.01), 'com o painel parado as linhas só se aproximam, sem saltos para trás');
      prev = now;
    }
    let t = 1260;
    while (stair.draw(t) && t < 4000) t += 16;
    assert.ok(t < 4000, 'todas as linhas alcançam o painel');
    assert.ok(items.every((el) => el.style.transform === '' && el.style.opacity === ''), 'aberta: posição natural');
    stair.push(0.4, 400, t + 16);
    assert.ok(items.every((el) => el.style.transform === ''), 'ao fechar tudo vai junto');
    stair.release();
    assert.ok(items.every((el) => el.style.willChange === '' && el.style.transition === ''), 'repouso sem camadas extras');
  } finally {
    g.window = saved.window;
    g.requestAnimationFrame = saved.raf;
  }
});
