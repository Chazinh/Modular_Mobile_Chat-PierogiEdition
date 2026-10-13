// Vertical elasticity: pulling past the start/end of any scrollable list stretches the content with rubber-band
// resistance and it springs back; a flung scroll that hits the edge does a small bounce. Scrolling itself stays
// NATIVE: we only listen to touches (passive, never cancelled) and move the content with transform.
// Touch events keep arriving during native scrolling (pointer events do not), which is why touch is used.
//
// Smoothness: finger moves are coalesced into one write per display frame (requestAnimationFrame, so 90/120 Hz
// screens get every frame and never two writes in one), offsets snap to device pixels, the elements to move are
// measured once per gesture, and the compositor layer is kept for the whole gesture. Creating and dropping that
// layer every time the stretch crossed zero is what made the glass bubbles flicker.
import { rubber, unrubber, springStep, clamp, frameDt } from './drawer-motion.js';

const SCROLLER = '.overflow-y-auto, .mc-scroll, .mc-sheet-body';
const LIMIT = 48; // maximum soft offset (px) while pulling
const anim = new WeakMap(); // scroller -> { raf, x }
const state = new WeakMap(); // scroller -> { x, movers, layered }

const movers = (el) => {
  const wrapper = el.querySelector(':scope > .mc-elastic-wrapper');
  if (wrapper) return [wrapper];
  return Array.from(el.children).filter((c) => getComputedStyle(c).position !== 'fixed');
};
const snap = (x) => {
  const d = window.devicePixelRatio || 1;
  return Math.round(x * d) / d;
};
function info(el) {
  let s = state.get(el);
  if (!s) state.set(el, (s = { x: 0, movers: null, layered: false }));
  return s;
}
// The wrapper of the menu/sheet pages already lives on its own layer (CSS); plain lists get one only while moving.
function layer(el, on) {
  const s = info(el);
  if (s.layered === on) return;
  s.layered = on;
  s.movers ||= movers(el);
  for (const c of s.movers) if (!c.classList.contains('mc-elastic-wrapper')) c.style.willChange = on ? 'transform' : '';
}
function apply(el, x) {
  const s = info(el);
  const v = x === 0 ? 0 : snap(x);
  if (v === s.x) return;
  s.x = v;
  if (v !== 0) layer(el, true);
  s.movers ||= movers(el);
  el.dataset.elasticY = v === 0 ? '' : String(v); // reading measurements subtract the stretch in progress
  const t = v === 0 ? '' : `translate3d(0, ${v}px, 0)`;
  for (const c of s.movers) c.style.transform = t;
}
// Back at rest: drop the temporary layer and the cached elements (the list may re-render before the next gesture).
function rest(el) {
  apply(el, 0);
  layer(el, false);
  info(el).movers = null;
}
function stop(el) {
  const a = anim.get(el);
  if (a) cancelAnimationFrame(a.raf);
  anim.delete(el);
}
// Spring back to 0 from x (px) with velocity v (px/s).
function settle(el, x, v) {
  stop(el);
  const a = { raf: 0, x };
  anim.set(el, a);
  let last = 0;
  let s = { x, v };
  const tick = (now) => {
    const dt = frameDt(now, last);
    last = now;
    s = springStep(s, 0, dt, 340, 36);
    if (Math.abs(s.x) < 0.3 && Math.abs(s.v) < 8) {
      anim.delete(el);
      rest(el);
      return;
    }
    a.x = s.x;
    apply(el, clamp(s.x, -LIMIT * 1.2, LIMIT * 1.2));
    a.raf = requestAnimationFrame(tick);
  };
  a.raf = requestAnimationFrame(tick);
}

export function installElastic() {
  if (typeof document === 'undefined' || window.__mcElastic) return;
  if (typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  window.__mcElastic = true;
  let g = null;
  let touching = false;
  let releasedAt = -Infinity; // end of the last touch: only a scroll flung by it may bounce
  let frame = 0;
  let pending = null; // [scroller, offset] waiting for the next display frame

  const edges = (el) => ({ top: el.scrollTop <= 0, bottom: el.scrollTop + el.clientHeight >= el.scrollHeight - 1 });
  const flush = () => {
    frame = 0;
    if (pending) apply(...pending);
    pending = null;
  };
  const queue = (el, x) => {
    pending = [el, x];
    if (!frame) frame = requestAnimationFrame(flush);
  };
  const drop = () => {
    cancelAnimationFrame(frame);
    frame = 0;
    pending = null;
  };

  document.addEventListener('touchstart', (e) => {
    touching = true;
    g = null;
    if (e.touches?.length !== 1) return;
    const el = e.target?.closest?.(SCROLLER);
    if (!el) return;
    const t = e.touches[0];
    drop();
    // Touching during a bounce grabs the content where it is (no jump back to 0 under the finger).
    const current = anim.has(el) ? info(el).x : 0;
    stop(el);
    const dir = Math.sign(current);
    const over = dir ? unrubber(Math.abs(current), LIMIT) : 0;
    g = { el, x0: t.clientX, y: t.clientY, anchor: t.clientY - dir * over, over, dir, samples: [[performance.now(), t.clientY]], moved: false };
  }, { passive: true, capture: true });

  document.addEventListener('touchmove', (e) => {
    if (!g || e.touches?.length !== 1) return;
    const t = e.touches[0];
    if (!g.moved) {
      // only gestures more vertical than horizontal (the drawer and sheets handle horizontal ones)
      if (Math.abs(t.clientX - g.x0) > Math.abs(t.clientY - g.anchor) * 1.2 && Math.abs(t.clientX - g.x0) > 8) {
        if (g.dir) settle(g.el, info(g.el).x, 0);
        g = null;
        return;
      }
      g.moved = true;
    }
    const y = t.clientY;
    const now = performance.now();
    g.samples.push([now, y]);
    while (g.samples.length > 2 && now - g.samples[0][0] > 90) g.samples.shift();
    const { top, bottom } = edges(g.el);
    let over = 0, dir = 0;
    if (top && bottom) {
      // content shorter than the list: no native scroll at all, so the stretch is one continuous function of the
      // finger through zero (up and down repeatedly never re-anchors or jumps)
      over = Math.abs(y - g.anchor);
      dir = Math.sign(y - g.anchor);
    } else if (top && y > g.anchor) { over = y - g.anchor; dir = 1; }
    else if (bottom && y < g.anchor) { over = g.anchor - y; dir = -1; }
    const el = g.el;
    if (!dir) {
      // Native scrolling took over (the finger turned back while stretched and the browser started scrolling).
      // The old code wrote 0 in the same frame: a jump of up to LIMIT px, the vertical "flick". The remaining
      // stretch now springs back from where it is while the native scroll carries on.
      g.anchor = y;
      if (g.dir && info(el).x !== 0) {
        drop();
        settle(el, info(el).x, 0);
      }
      g.over = 0;
      g.dir = 0;
      if (!anim.has(el)) queue(el, 0);
      return;
    }
    if (anim.has(el)) {
      // back at the edge while that spring is still running: take over from the current offset, no jump
      const current = info(el).x;
      stop(el);
      if (Math.sign(current) === dir) {
        g.anchor = y - dir * unrubber(Math.abs(current), LIMIT);
        over = Math.abs(y - g.anchor);
      }
    }
    g.over = over;
    g.dir = dir;
    queue(el, dir * rubber(over, LIMIT));
  }, { passive: true, capture: true });

  const end = () => {
    touching = false;
    releasedAt = performance.now();
    if (!g) return;
    const { el, over, dir, samples } = g;
    g = null;
    drop();
    if (over <= 0) {
      // a spring already returning the content keeps going; cutting it to 0 here was another jump
      if (!anim.has(el)) rest(el);
      return;
    }
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
