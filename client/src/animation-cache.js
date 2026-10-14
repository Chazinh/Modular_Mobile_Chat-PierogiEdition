// Animation Cache System for ModularChat (Mobile / Capacitor).
// Caches animation trajectories, physics tables, and layout metrics in the application cache (IndexedDB / localStorage).
// On startup after closing the app, loads the cache directly into RAM and pre-warms GPU compositor layers and V8 JIT
// so animations are instantly fluid from the very first gesture.
//
// STRICT PERSISTENCE RULE:
// Saves to disk/cache ONLY ONCE (when the cache is absent or updated).
// Never saves repeatedly or during frame loops, gestures, or animations.

import { kv } from './kv.js';
import { rubber, unrubber, deformation, springStep } from './drawer-motion.js';

export const ANIM_CACHE_KEY = 'mc.anim_cache.v1';
export const ANIM_CACHE_VERSION = 1;

let memoryCache = null;
let isInitialized = false;
let hasSavedToDisk = false;

// Generates physics tables, trajectories, and curves for all app animations.
export function generateAnimationTables() {
  const dpr = typeof window !== 'undefined' ? (window.devicePixelRatio || 1) : 1;
  const screenW = typeof window !== 'undefined' ? (window.innerWidth || 390) : 390;
  const screenH = typeof window !== 'undefined' ? (window.innerHeight || 844) : 844;

  // 1. Rubber-band lookup tables for common limits (84 for drawer, 48 for elastic, 40 for top bar)
  const rubberLimits = [84, 48, 40];
  const rubberLUT = {};
  for (const lim of rubberLimits) {
    const table = [];
    for (let d = 0; d <= 200; d += 2) {
      table.push({ d, val: Math.round(rubber(d, lim) * 1000) / 1000 });
    }
    rubberLUT[lim] = table;
  }

  // 2. Precalculated spring trajectories for standard transitions (at 60Hz and 120Hz)
  // Drawer open: 360 -> 0 (k = 180, c = 27)
  const drawerOpenSteps = [];
  let s = { x: 360, v: 0 };
  const dt60 = 1 / 60;
  for (let i = 0; i < 45; i++) {
    s = springStep(s, 0, dt60, 180, 27);
    drawerOpenSteps.push(Math.round(s.x * 100) / 100);
    if (Math.abs(s.x) < 0.3 && Math.abs(s.v) < 6) break;
  }

  // Drawer close: 0 -> 368 (k = 180, c = 27)
  const drawerCloseSteps = [];
  s = { x: 0, v: 0 };
  for (let i = 0; i < 45; i++) {
    s = springStep(s, 368, dt60, 180, 27);
    drawerCloseSteps.push(Math.round(s.x * 100) / 100);
    if (s.x >= 360) break;
  }

  // Top bar open: 0 -> 1 (k = 170, c = 15)
  const topBarOpenSteps = [];
  s = { x: 0, v: 0 };
  for (let i = 0; i < 45; i++) {
    s = springStep(s, 1, dt60, 170, 15);
    topBarOpenSteps.push(Math.round(s.x * 1000) / 1000);
    if (Math.abs(s.x - 1) < 0.002 && Math.abs(s.v) < 0.02) break;
  }

  // Top bar close: 1 -> 0 (k = 170, c = 22)
  const topBarCloseSteps = [];
  s = { x: 1, v: 0 };
  for (let i = 0; i < 45; i++) {
    s = springStep(s, 0, dt60, 170, 22);
    topBarCloseSteps.push(Math.round(s.x * 1000) / 1000);
    if (Math.abs(s.x) < 0.002 && Math.abs(s.v) < 0.02) break;
  }

  // Elastic return: 48 -> 0 (k = 340, c = 36)
  const elasticSteps = [];
  s = { x: 48, v: 0 };
  for (let i = 0; i < 35; i++) {
    s = springStep(s, 0, dt60, 340, 36);
    elasticSteps.push(Math.round(s.x * 100) / 100);
    if (Math.abs(s.x) < 0.3 && Math.abs(s.v) < 8) break;
  }

  return {
    version: ANIM_CACHE_VERSION,
    cachedAt: Date.now(),
    metrics: { dpr, screenW, screenH },
    rubberLUT,
    trajectories: {
      drawerOpen: drawerOpenSteps,
      drawerClose: drawerCloseSteps,
      topBarOpen: topBarOpenSteps,
      topBarClose: topBarCloseSteps,
      elastic: elasticSteps,
    },
  };
}

// Loads cached animation tables from persistent storage (IndexedDB / localStorage).
export async function loadAnimationCache() {
  // 1. Fast synchronous check in localStorage
  if (typeof localStorage !== 'undefined') {
    try {
      const raw = localStorage.getItem(ANIM_CACHE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw);
        if (parsed && parsed.version === ANIM_CACHE_VERSION) {
          return parsed;
        }
      }
    } catch {
      /* ignore */
    }
  }

  // 2. Persistent check in IndexedDB (kv)
  try {
    const data = await kv.get(ANIM_CACHE_KEY);
    if (data && data.version === ANIM_CACHE_VERSION) {
      return data;
    }
  } catch {
    /* ignore */
  }

  return null;
}

// Saves animation tables to cache ONCE.
// Guarded by hasSavedToDisk: strictly never executes multiple times.
export async function saveAnimationCache(data) {
  if (hasSavedToDisk) return;
  hasSavedToDisk = true;

  try {
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(ANIM_CACHE_KEY, JSON.stringify(data));
    }
  } catch {
    /* localStorage quota or disabled */
  }

  try {
    await kv.set(ANIM_CACHE_KEY, data);
  } catch {
    /* IndexedDB error */
  }
}

// Warms up V8 JIT compiler and GPU compositor layers in memory.
export function warmupAnimationMemory(cache) {
  // JIT compiler warmup: exercise spring and math functions
  springStep({ x: 360, v: 0 }, 0, 1 / 60, 180, 27);
  springStep({ x: 0, v: 0 }, 1, 1 / 60, 170, 15);
  springStep({ x: 48, v: 0 }, 0, 1 / 60, 340, 36);
  rubber(20, 84);
  rubber(20, 48);
  unrubber(15, 84);
  unrubber(15, 48);
  deformation(-40, 2000);

  // GPU compositor & shader warmup: render hidden sample layer
  if (typeof document !== 'undefined' && document.body) {
    try {
      const warm = document.createElement('div');
      warm.setAttribute('aria-hidden', 'true');
      warm.style.cssText =
        'position:fixed;left:-9999px;top:-9999px;width:10px;height:10px;opacity:0.001;pointer-events:none;contain:strict;z-index:-9999;';
      warm.innerHTML = `
        <div class="mc-glass mc-bar-handle"><span class="mc-bar-dot"></span></div>
        <div class="mc-bar-bubble"></div>
        <div class="mc-elastic-wrapper"><div class="mc-stair mc-row"></div></div>
        <div class="mc-sheet mc-glass"><div class="mc-grabber"></div></div>
      `;
      document.body.appendChild(warm);
      // Force initial layout
      void warm.offsetHeight;

      if (typeof requestAnimationFrame === 'function') {
        requestAnimationFrame(() => {
          try {
            warm.style.transform = 'translate3d(0,0,0)';
            requestAnimationFrame(() => {
              warm.remove();
            });
          } catch {
            warm.remove();
          }
        });
      } else {
        warm.remove();
      }
    } catch {
      /* ignore DOM errors in restricted contexts */
    }
  }
}

// Main initialization: loads from cache on startup, or saves ONCE if not present.
export async function initAnimationCache() {
  if (isInitialized && memoryCache) return memoryCache;
  isInitialized = true;

  // 1. Try to load from persistent cache (after app restart)
  let data = await loadAnimationCache();

  if (!data) {
    // Generated and saved ONLY ONCE
    data = generateAnimationTables();
    await saveAnimationCache(data);
  }

  // 2. Put into memory cache
  memoryCache = data;

  // 3. Pre-warm memory, JIT, and GPU compositor
  warmupAnimationMemory(memoryCache);

  return memoryCache;
}

// Access current in-memory cache without storage reads or writes.
export function getAnimationCache() {
  return memoryCache;
}

export function isAnimationCacheReady() {
  return memoryCache !== null;
}
