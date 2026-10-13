// Vertical elasticity: pulling past the start/end of any scrollable list stretches the content with rubber-band
// resistance and it springs back; a flung scroll that hits the edge does a small bounce. Scrolling itself stays
// NATIVE: we only listen to touches (passive, never cancelled) and move the content with transform.
// Touch events keep arriving during native scrolling (pointer events do not), which is why touch is used.
import { rubber, springStep, clamp } from './drawer-motion.js';

const SCROLLER = '.overflow-y-auto, .mc-scroll, .mc-sheet-body';
const LIMIT = 48; // maximum soft offset (px) while pulling
const anim = new WeakMap(); // scroller -> { raf, x }

const movers = (el) => {
  const wrapper = el.querySelector(':scope > .mc-elastic-wrapper');
  if (wrapper) return [wrapper];
  return Array.from(el.children).filter((c) => getComputedStyle(c).position !== 'fixed');
};
function apply(el, x) {
  el.dataset.elasticY = x === 0 ? '' : x.toFixed(2); // reading measurements subtract the stretch in progress
  const t = x === 0 ? '' : `translate3d(0, ${x.toFixed(2)}px, 0)`;
  for (const c of movers(el)) {
    c.style.transform = t;
    c.style.willChange = x === 0 ? '' : 'transform';
  }
}
function stop(el) {
  const a = anim.get(el);
  if (a) cancelAnimationFrame(a.raf);
  anim.delete(el);
}
// Spring back to 0 from x (px) with velocity v (px/s).
function settle(el, x, v) {
  stop(el);
  const state = { raf: 0, x };
  anim.set(el, state);
  let last = performance.now();
  let s = { x, v };
  const tick = (now) => {
    const dt = (now - last) / 1000;
    last = now;
    s = springStep(s, 0, dt, 340, 36);
    if (Math.abs(s.x) < 0.3 && Math.abs(s.v) < 8) {
      apply(el, 0);
      anim.delete(el);
      return;
    }
    state.x = s.x;
    apply(el, clamp(s.x, -LIMIT * 1.2, LIMIT * 1.2));
    state.raf = requestAnimationFrame(tick);
  };
  state.raf = requestAnimationFrame(tick);
}

export function installElastic() {
  if (typeof document === 'undefined' || window.__mcElastic) return;
  if (typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  window.__mcElastic = true;
  let g = null;
  let touching = false;
  let releasedAt = -Infinity; // end of the last touch: only a scroll flung by it may bounce

  const edges = (el) => ({ top: el.scrollTop <= 0, bottom: el.scrollTop + el.clientHeight >= el.scrollHeight - 1 });

  document.addEventListener('touchstart', (e) => {
    touching = true;
    g = null;
    if (e.touches?.length !== 1) return;
    const el = e.target?.closest?.(SCROLLER);
    if (!el) return;
    const t = e.touches[0];
    stop(el);
    apply(el, 0);
    g = { el, x0: t.clientX, y: t.clientY, anchor: t.clientY, over: 0, dir: 0, samples: [[performance.now(), t.clientY]], moved: false };
  }, { passive: true, capture: true });

  document.addEventListener('touchmove', (e) => {
    if (!g || e.touches?.length !== 1) return;
    const t = e.touches[0];
    if (!g.moved) {
      // only gestures more vertical than horizontal (the drawer and sheets handle horizontal ones)
      if (Math.abs(t.clientX - g.x0) > Math.abs(t.clientY - g.anchor) * 1.2 && Math.abs(t.clientX - g.x0) > 8) { g = null; return; }
      g.moved = true;
    }
    const y = t.clientY;
    const now = performance.now();
    g.samples.push([now, y]);
    while (g.samples.length > 2 && now - g.samples[0][0] > 90) g.samples.shift();
    const { top, bottom } = edges(g.el);
    let over = 0, dir = 0;
    if (top && y > g.anchor && (!bottom || g.dir >= 0)) { over = y - g.anchor; dir = 1; }
    else if (bottom && y < g.anchor) { over = g.anchor - y; dir = -1; }
    else g.anchor = y; // native scrolling in progress (or returning from the stretch): the anchor follows the finger
    g.over = over;
    g.dir = dir;
    apply(g.el, over > 0 ? dir * rubber(over, LIMIT) : 0);
  }, { passive: true, capture: true });

  const end = () => {
    touching = false;
    releasedAt = performance.now();
    if (!g) return;
    const { el, over, dir, samples } = g;
    g = null;
    if (over <= 0) return;
    const [t0, y0] = samples[0];
    const [t1, y1] = samples[samples.length - 1];
    const v = t1 > t0 ? ((y1 - y0) / (t1 - t0)) * 1000 : 0; // finger px/s
    settle(el, dir * rubber(over, LIMIT), v * 0.25);
  };
  document.addEventListener('touchend', end, { passive: true, capture: true });
  document.addEventListener('touchcancel', end, { passive: true, capture: true });

  // Bounce at the end of a flung scroll (no finger on screen) that reaches the edge. Jumps made by the app
  // (reading anchor, "go to end", switching chats) do not come from a recent touch and do not bounce.
  const track = new WeakMap(); // scroller -> { top, t }
  document.addEventListener('scroll', (e) => {
    const el = e.target;
    if (!(el instanceof Element) || touching || !el.matches?.(SCROLLER)) return;
    const now = performance.now();
    const prev = track.get(el);
    track.set(el, { top: el.scrollTop, t: now });
    if (!prev || now - prev.t > 80 || anim.has(el) || now - releasedAt > 2500) return;
    // real momentum moves at most ~240 px per frame; a larger jump is scrolling done by code
    // (reading anchor, "go to end" button), which does not bounce
    if (Math.abs(el.scrollTop - prev.top) > Math.min(320, el.clientHeight * 0.9)) return;
    const v = (el.scrollTop - prev.top) / Math.max(1, now - prev.t); // px/ms (+ = scrolling down)
    const { top, bottom } = edges(el);
    if (Math.abs(v) < 1.2) return;
    if ((v < 0 && top) || (v > 0 && bottom)) settle(el, 0, -v * 1000 * 0.3);
  }, { passive: true, capture: true });
}
