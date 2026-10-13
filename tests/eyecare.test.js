import test from 'node:test';
import assert from 'node:assert/strict';

// minimal simulated document (the module only uses documentElement.style, getElementById, createElement and body)
const vars = {};
const registry = {};
globalThis.document = {
  readyState: 'complete',
  documentElement: { style: { setProperty: (k, v) => (vars[k] = v) } },
  getElementById: (id) => registry[id] || null,
  createElement: () => {
    const el = { style: {}, setAttribute() {}, remove() { delete registry[this.id]; } };
    // registers the element when the module assigns the id right after creating it
    return new Proxy(el, { set(t, k, v) { t[k] = v; if (k === 'id') registry[v] = t; return true; } });
  },
  body: { appendChild: () => {} },
  addEventListener() {},
};
const store = new Map();
globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)) };
globalThis.window = {};

const { normalize, colorsFor, scaleColor, blueTint, tintColor, applyEyeCare, loadEyeCare, saveEyeCare, PRESETS, DEFAULTS, PAPER_MAX_OPACITY } = await import('../client/src/eyecare.js');
const chan = (hex, i) => parseInt(hex.slice(1 + 2 * i, 3 + 2 * i), 16);

test('normaliza valores inválidos para limites seguros', () => {
  assert.deepEqual(normalize({}), DEFAULTS);
  const n = normalize({ preset: 'inexistente', custom: 'vermelho', brightness: 10, blue: 999 });
  assert.equal(n.preset, 'neutro');
  assert.equal(n.custom, '#e7e7ea');
  assert.equal(n.brightness, 50, 'nunca escurece além de 50%');
  assert.equal(n.blue, 60, 'filtro de luz azul limitado a 60%');
  assert.equal(normalize({ paper: 500 }).paper, 100, 'textura de papel limitada a 100%');
  assert.equal(normalize({ paper: -5 }).paper, 0);
  assert.equal(normalize({ brightness: 'abc' }).brightness, 100);
});

test('brilho escurece as letras proporcionalmente e 100% mantém a cor original', () => {
  assert.equal(scaleColor('#ffffff', 0.5), '#808080');
  assert.equal(colorsFor({ preset: 'neutro', brightness: 100 }).text, PRESETS.neutro.text);
  const dim = colorsFor({ preset: 'neutro', brightness: 60 });
  assert.ok(chan(dim.text, 0) < chan(PRESETS.neutro.text, 0));
  assert.ok(chan(dim.white, 0) < 255, 'o branco puro também escurece');
});

test('texto sobre botão de destaque não segue o brilho', () => {
  assert.equal(colorsFor({ preset: 'ambar', brightness: 50 }).onAccent, '#ffffff');
  assert.equal(colorsFor({ preset: 'ambar', brightness: 100 }).onAccent, '#ffffff');
});

test('cor personalizada deriva nome/ação/tom mantendo a hierarquia de brilho', () => {
  const c = colorsFor({ preset: 'custom', custom: '#f0c78a', brightness: 100 });
  const lum = (hex) => hex.slice(1).match(/../g).reduce((a, h) => a + parseInt(h, 16), 0);
  assert.equal(c.text, '#f0c78a');
  assert.ok(lum(c.name) >= lum(c.text) && lum(c.text) > lum(c.action) && lum(c.action) >= lum(c.tone));
});

test('tingimento do filtro de luz azul: neutro em 0%, reduz azul mais que verde e nunca toca no vermelho', () => {
  assert.deepEqual(blueTint(0), { r: 1, g: 1, b: 1 });
  assert.deepEqual(blueTint(-3), { r: 1, g: 1, b: 1 });
  let prev = blueTint(0);
  for (let b = 10; b <= 60; b += 10) {
    const t = blueTint(b);
    assert.equal(t.r, 1);
    assert.ok(t.b < t.g && t.g < 1, `azul ${b}%: b < g < 1`);
    assert.ok(t.g < prev.g && t.b < prev.b, 'cresce com a intensidade');
    prev = t;
  }
  // same factors as the old multiplied overlay rgba(255, 170-40a, 80-40a, min(.75, 1.15a))
  const t60 = blueTint(60);
  assert.ok(Math.abs(t60.g - 0.705) < 0.002 && Math.abs(t60.b - 0.4615) < 0.002, JSON.stringify(t60));
  const t30 = blueTint(30);
  assert.ok(Math.abs(t30.g - 0.8688) < 0.002 && Math.abs(t30.b - 0.747) < 0.002, JSON.stringify(t30));
  assert.deepEqual(blueTint(999), blueTint(60), 'limitado a 60%');
});

test('tintColor aplica os fatores por canal e mantém a cor quando não há filtro', () => {
  assert.equal(tintColor('#e7e7ea', { r: 1, g: 1, b: 1 }), '#e7e7ea');
  assert.equal(tintColor('#e7e7ea'), '#e7e7ea');
  assert.equal(tintColor('#ffffff', { r: 1, g: 0.5, b: 0.25 }), '#ff8040');
});

test('as letras também ganham o tom do filtro de luz azul (mais quente, menos azul)', () => {
  const sem = colorsFor({ preset: 'neutro', brightness: 100, blue: 0 });
  const com = colorsFor({ preset: 'neutro', brightness: 100, blue: 60 });
  for (const k of ['text', 'name', 'action', 'tone', 'white', 'muted', 'onAccent']) {
    assert.equal(chan(com[k], 0), chan(sem[k] ?? '#ffffff', 0), `${k}: vermelho intacto`);
    assert.ok(chan(com[k], 2) < chan(sem[k] ?? '#ffffff', 2) * 0.6, `${k}: azul reduzido`);
  }
});

test('aplicar grava as variáveis CSS e o tingimento; não existe mais película de tela cheia', () => {
  applyEyeCare({ preset: 'ambar', brightness: 80, blue: 0 });
  assert.equal(vars['--mc-text'], colorsFor({ preset: 'ambar', brightness: 80 }).text);
  assert.equal(vars['--mc-onaccent'], '#ffffff');
  assert.deepEqual([vars['--mc-tr'], vars['--mc-tg'], vars['--mc-tb']], ['1', '1', '1']);
  assert.equal(vars['--mc-white-rgb'], '255, 255, 255');
  applyEyeCare({ preset: 'neutro', brightness: 100, blue: 40 });
  const t = blueTint(40);
  assert.deepEqual([vars['--mc-tr'], vars['--mc-tg'], vars['--mc-tb']], ['1', String(t.g), String(t.b)]);
  assert.match(vars['--mc-white-rgb'], /^255, \d+, \d+$/);
  assert.equal(vars['--mc-text'], colorsFor({ preset: 'neutro', blue: 40 }).text);
  assert.equal(registry['mc-bluelight'], undefined, 'sem overlay mix-blend-mode');
});

test('película antiga de versões anteriores é removida ao aplicar', () => {
  registry['mc-bluelight'] = { id: 'mc-bluelight', remove() { delete registry['mc-bluelight']; } };
  applyEyeCare({ blue: 20 });
  assert.equal(registry['mc-bluelight'], undefined);
});

test('textura de papel: camada estática só de opacidade, sem blend nem will-change, sem bloquear toques', () => {
  delete registry['mc-paper'];
  applyEyeCare({ paper: 0 });
  assert.equal(registry['mc-paper'], undefined, 'não cria a camada quando o papel está desligado');
  applyEyeCare({ paper: 50 });
  const paperStyle = registry['mc-paper'].style;
  assert.equal(paperStyle.display, 'block');
  assert.equal(paperStyle.pointerEvents, 'none', 'a camada nunca intercepta toques');
  assert.equal(paperStyle.mixBlendMode, undefined);
  assert.equal(paperStyle.willChange, undefined);
  assert.equal(paperStyle.backdropFilter, undefined);
  assert.match(paperStyle.backgroundImage, /^url\("data:image\/svg\+xml,/);
  assert.match(decodeURIComponent(paperStyle.backgroundImage), /feTurbulence/);
  const half = Number(paperStyle.opacity);
  applyEyeCare({ paper: 100 });
  assert.ok(Number(paperStyle.opacity) > half && Number(paperStyle.opacity) <= 0.24, 'no máximo 0,24 (grão suave) para o texto continuar legível');
  assert.equal(PAPER_MAX_OPACITY, 0.24, 'teto de intensidade preservado');
  assert.equal(paperStyle.opacity, '0.240');
  applyEyeCare({ paper: 0 });
  assert.equal(paperStyle.display, 'none');
});

test('preferências persistem e valores corrompidos no armazenamento não quebram o app', () => {
  saveEyeCare({ preset: 'sepia', brightness: 70, blue: 20 });
  assert.deepEqual(loadEyeCare(), { preset: 'sepia', custom: '#e7e7ea', brightness: 70, blue: 20, paper: 0 });
  store.set('mc.eyecare', '{quebrado');
  assert.deepEqual(loadEyeCare(), DEFAULTS);
});

test('Tailwind: letras mapeadas para --mc-*, cores da paleta tingidas por canal, sem exceções fixas de texto', async () => {
  const cfg = (await import('../client/tailwind.config.mjs')).default;
  const tc = cfg.theme.extend.textColor;
  assert.match(tc.white, /var\(--mc-white/);
  assert.match(tc.textMuted, /var\(--mc-muted/);
  assert.match(tc.gray[100], /var\(--mc-text/);
  assert.match(tc.gray[300], /var\(--mc-text/);
  assert.match(tc.onAccent, /var\(--mc-onaccent/);
  const bg = cfg.theme.colors.bgDark({ opacityValue: '0.5' });
  assert.equal(bg, 'rgb(calc(27 * var(--mc-tr, 1)) calc(27 * var(--mc-tg, 1)) calc(29 * var(--mc-tb, 1)) / 0.5)');
  assert.doesNotMatch(cfg.theme.colors.red[300](), /\/ /, 'sem opacidade explícita não emite alfa');
  assert.ok(typeof cfg.theme.colors.accentPurple === 'function' && typeof cfg.theme.colors.gray[500] === 'function');
});
