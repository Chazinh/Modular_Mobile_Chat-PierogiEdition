import test from 'node:test';
import assert from 'node:assert/strict';
import {
  generateAnimationTables,
  loadAnimationCache,
  saveAnimationCache,
  warmupAnimationMemory,
  initAnimationCache,
  getAnimationCache,
  isAnimationCacheReady,
  ANIM_CACHE_KEY,
  ANIM_CACHE_VERSION,
} from '../client/src/animation-cache.js';

test('tabelas de animação: gera dados de física, métricas e trajetórias válidas', () => {
  const tables = generateAnimationTables();
  assert.equal(tables.version, ANIM_CACHE_VERSION);
  assert.ok(tables.cachedAt > 0);
  assert.ok(tables.metrics.dpr >= 1);
  assert.ok(tables.metrics.screenW > 0);
  assert.ok(tables.metrics.screenH > 0);

  // Validação da tabela de borracha
  assert.ok(Array.isArray(tables.rubberLUT[84]));
  assert.ok(tables.rubberLUT[84].length > 10);
  assert.equal(tables.rubberLUT[84][0].d, 0);
  assert.equal(tables.rubberLUT[84][0].val, 0);

  // Validação das trajetórias de mola pré-calculadas
  const traj = tables.trajectories;
  assert.ok(traj.drawerOpen.length > 5, 'trajetória de abertura da gaveta gerada');
  assert.equal(traj.drawerOpen[0] < 360, true, 'mola de abertura começa a aproximar de 0');
  assert.ok(traj.drawerClose.length > 5, 'trajetória de fechamento da gaveta gerada');
  assert.ok(traj.topBarOpen.length > 5, 'trajetória da barra superior gerada');
  assert.ok(traj.topBarClose.length > 5, 'trajetória de fechamento da barra gerada');
  assert.ok(traj.elastic.length > 5, 'trajetória de elasticidade gerada');
});

test('cache de animação: warmup em memória executa sem erros em qualquer ambiente', () => {
  const tables = generateAnimationTables();
  assert.doesNotThrow(() => warmupAnimationMemory(tables));
});

test('cache de animação: inicialização carrega do cache persistente e salva apenas uma vez', async () => {
  // Mock localStorage
  const store = new Map();
  const originalStorage = globalThis.localStorage;
  globalThis.localStorage = {
    getItem: (k) => store.get(k) || null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };

  try {
    // 1ª inicialização (fria): deve gerar os dados e gravar no cache
    const firstRun = await initAnimationCache();
    assert.ok(firstRun);
    assert.equal(isAnimationCacheReady(), true);
    assert.equal(getAnimationCache(), firstRun);

    // Confere que gravou no storage
    const stored = store.get(ANIM_CACHE_KEY);
    assert.ok(stored, 'cache deve ter sido persistido no storage');
    const parsed = JSON.parse(stored);
    assert.equal(parsed.version, ANIM_CACHE_VERSION);

    // Modifica um valor no storage para testar se a 2ª inicialização lê sem reescrever
    parsed._marker = 'from_cache';
    store.set(ANIM_CACHE_KEY, JSON.stringify(parsed));

    // Carrega diretamente do storage
    const loaded = await loadAnimationCache();
    assert.equal(loaded._marker, 'from_cache', 'deve ler os dados do cache persistente');

    // Tentar salvar novamente não deve sobrescrever (regra de gravação única)
    await saveAnimationCache({ version: ANIM_CACHE_VERSION, _marker: 'should_not_overwrite' });
    const storedAfter = JSON.parse(store.get(ANIM_CACHE_KEY));
    assert.equal(storedAfter._marker, 'from_cache', 'não deve regravar no cache quando já salvo');
  } finally {
    globalThis.localStorage = originalStorage;
  }
});
