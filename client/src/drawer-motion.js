// Motion of the side menu and sheets: the panel follows the finger, settles with a spring (slight overshoot),
// resists like rubber past its limits and deforms a little with speed, like iOS/One UI liquid glass.
// Only transform and opacity, written straight to the DOM (no React state per frame), so the
// compositor keeps the device refresh rate (90/120 Hz).

export const EDGE_PX = 20; // activation strip along the right edge (width of the Samsung Edge panel handle)

export const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

// Rubber-band resistance (iOS curve): grows ever slower and never exceeds `limit`.
export function rubber(distance, limit, c = 0.55) {
  if (!(distance > 0)) return 0;
  return (1 - 1 / ((distance * c) / limit + 1)) * limit;
}

// Inverse of rubber(): how far the finger moved for the panel to be at `y`.
export function unrubber(y, limit, c = 0.55) {
  const r = clamp(y / limit, 0, 0.999);
  return (limit / c) * (1 / (1 - r) - 1);
}

// Panel position (px; 0 = open, closed = closed) for the raw finger offset.
// Past open (raw < 0) the panel resists and stretches; past closed, it stops.
export function follow(raw, closed, overscroll = 84) {
  if (raw < 0) return -rubber(-raw, overscroll);
  return Math.min(raw, closed);
}

// Spring integrator (semi-implicit Euler with substeps): stable even with long frames.
export function springStep(state, goal, dt, k = 520, c = 34) {
  dt = clamp(dt, 0, 0.05);
  const n = Math.max(1, Math.ceil(dt * 240));
  const h = dt / n;
  let { x, v } = state;
  for (let i = 0; i < n; i++) {
    v += (-k * (x - goal) - c * v) * h;
    x += v * h;
  }
  return { x, v };
}

// Subtle constant-volume deformation (along/across)
export function deformation(x, v) {
  const over = Math.max(0, -x);
  const alongPull = 1 + clamp(over / 84, 0, 1) * 0.035;
  const speed = Math.abs(v);
  const alongSpeed = 1 + clamp(speed / 8000, 0, 1) * 0.03;
  const along = Math.max(alongPull, alongSpeed);
  const across = 1 / Math.sqrt(along);
  return { along, across };
}

// Finger released: open or close by velocity (fling) or by the projected position.
export function releaseTarget(x, v, closed) {
  if (v > 650) return 'closed';
  if (v < -650) return 'open';
  return x + v * 0.2 > closed / 2 ? 'closed' : 'open';
}

// Velocity (px/s) from the latest samples (~90 ms).
export class VelocityTracker {
  constructor() {
    this.s = [];
  }
  push(x, t = performance.now()) {
    this.s.push([t, x]);
    while (this.s.length > 2 && t - this.s[0][0] > 90) this.s.shift();
  }
  velocity() {
    const s = this.s;
    if (s.length < 2) return 0;
    const [t0, x0] = s[0];
    const [t1, x1] = s[s.length - 1];
    return t1 > t0 ? ((x1 - x0) / (t1 - t0)) * 1000 : 0;
  }
  reset() {
    this.s = [];
  }
}

const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

// First frame of an animation: the requestAnimationFrame timestamp is the frame start and can be EARLIER than the
// performance.now() read when the animation was scheduled. A negative dt froze the first frame (a visible hitch right
// after letting go); a missing previous timestamp counts as one frame instead.
export const frameDt = (now, last) => (last ? clamp((now - last) / 1000, 0, 0.05) : 1 / 60);

// "Staircase": the items of a side panel appear one after another, top to bottom. Each row waits its turn (`delay`
// ms after the row above it, counted from the start of the gesture/animation), hidden where the panel started, and
// then chases the panel with its own critically damped spring (continuous every frame: no replayed samples, no
// steps), so the rows clearly surface one by one instead of arriving together. Only while OPENING
// (a row never runs ahead of the panel): closing moves everything together. Only transform/opacity, only the rows
// on screen; at rest (fully open or closed) no inline styles or extra layers remain.
export class Staircase {
  constructor(getItems, { k = 210, falloff = 0.1, delay = 55, max = 14 } = {}) {
    Object.assign(this, { getItems, k, falloff, delay, max });
    this.t0 = 0;
    this.items = null;
    this.st = [];
    this.p = 0;
    this.closed = 1;
    this.last = 0;
    this.raf = 0;
  }
  collect() {
    const h = window.innerHeight;
    // only what is on screen moves: rows below the fold are not seen and would cost writes every frame
    this.items = Array.from(this.getItems() || []).filter((el) => {
      const r = el.getBoundingClientRect();
      return r.height > 0 && r.top < h && r.bottom > 0;
    }).slice(0, this.max);
    // every row starts where the panel was when the gesture/animation began
    this.st = this.items.map(() => ({ x: this.p, v: 0 }));
    for (const el of this.items) {
      el.style.willChange = 'transform, opacity';
      el.style.transition = 'none'; // .mc-row's press transition would smear the per-frame writes
    }
  }
  push(p, closed, now = performance.now()) {
    if (!this.items) {
      this.collect();
      this.t0 = now; // the turns are counted from here
    }
    this.p = p;
    this.closed = closed;
    this.draw(now);
    if (!this.raf && typeof requestAnimationFrame === 'function') this.raf = requestAnimationFrame(this.loop);
  }
  // advances every row's spring to `now`; returns true while some row is still behind the panel
  draw(now) {
    const dt = this.last ? clamp((now - this.last) / 1000, 0, 0.05) : 1 / 60;
    this.last = now;
    let behind = false;
    (this.items || []).forEach((el, i) => {
      const s = this.st[i];
      if (this.p <= s.x) {
        s.x = this.p; // closing (or caught up): the row goes with the panel
        s.v = 0;
      } else if (now - this.t0 < i * this.delay) {
        // not its turn yet: stays where the panel started, hidden
      } else if (dt > 0) {
        const k = this.k / (1 + i * this.falloff);
        const r = springStep(s, this.p, dt, k, 2 * Math.sqrt(k));
        s.x = Math.min(r.x, this.p);
        s.v = r.v;
      }
      const offset = (this.p - s.x) * this.closed; // px behind the panel
      if (offset < 0.5) {
        s.x = this.p;
        s.v = 0;
        el.style.transform = '';
        el.style.opacity = '';
      } else {
        behind = true;
        el.style.transform = `translate3d(${offset.toFixed(2)}px,0,0)`;
        el.style.opacity = (1 - clamp(offset / (this.closed * 0.45), 0, 1)).toFixed(3); // waiting rows are invisible
      }
    });
    return behind;
  }
  // keeps the rows gliding after the finger stops or the panel's spring settles
  loop = () => {
    this.raf = 0;
    if (!this.items) return;
    if (this.draw(performance.now())) {
      this.raf = requestAnimationFrame(this.loop);
    } else if (this.p >= 1 || this.p <= 0) {
      this.release();
    }
  };
  release() {
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
    for (const el of this.items || []) {
      el.style.transform = '';
      el.style.opacity = '';
      el.style.willChange = '';
      el.style.transition = '';
    }
    this.items = null;
    this.st = [];
    this.last = 0;
  }
}

// Panel that slides along one axis ('x' = side drawer, 'y' = bottom sheet).
export class PanelMotion {
  constructor({ panel, scrim, axis = 'x', closed, onClosed, onOpened, stagger }) {
    this.panel = panel;
    this.stagger = stagger ? new Staircase(stagger) : null;
    this.frame = 0;
    this.scrim = scrim;
    this.axis = axis;
    this.closed = closed;
    this.onClosed = onClosed;
    this.onOpened = onOpened;
    this.x = closed;
    this.v = 0;
    this.goal = closed;
    this.mode = null; // 'opening' | 'closing' | null
    this.raf = 0;
    this.last = 0;
    this.drag = null;
    this.tracker = new VelocityTracker();
    this.dead = false;
    this.render();
  }

  render() {
    const t = this.axis === 'x' ? `translate3d(${this.x.toFixed(2)}px,0,0)` : `translate3d(0,${this.x.toFixed(2)}px,0)`;
    this.panel.style.transform = t;
    const p = clamp(1 - this.x / this.closed, 0, 1);
    if (this.scrim) this.scrim.style.opacity = p.toFixed(3);
    this.stagger?.push(p, this.closed);
  }

  // Finger moves are coalesced into one write per display frame (never two layouts in the same frame).
  queueRender() {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      if (!this.dead) this.render();
    });
  }

  stop() {
    cancelAnimationFrame(this.raf);
    cancelAnimationFrame(this.frame);
    this.raf = 0;
    this.frame = 0;
    this.mode = null;
  }

  // ---- finger drag ----
  begin(finger) {
    if (this.dead) return;
    this.stop();
    const raw = this.x < 0 ? -unrubber(-this.x, 24) : this.x;
    this.drag = { finger, raw };
    this.tracker.reset();
    this.tracker.push(this.x);
    this.panel.classList.add('mc-dragging');
  }
  move(finger) {
    if (this.dead || !this.drag) return;
    const raw = this.drag.raw + (finger - this.drag.finger);
    this.x = follow(raw, this.closed, 24);
    this.tracker.push(this.x);
    this.v = this.tracker.velocity();
    this.queueRender();
  }
  end() {
    if (this.dead || !this.drag) return;
    this.drag = null;
    this.panel.classList.remove('mc-dragging');
    const v = this.tracker.velocity();
    this.v = v;
    this.settle(releaseTarget(this.x, v, this.closed), v);
  }

  // ---- animations ----
  open() {
    this.settle('open', 0);
  }
  close() {
    this.settle('closed', Math.max(0, this.v));
  }
  settle(target, v0) {
    if (this.dead) return;
    this.stop();
    this.v = this.axis === 'x' ? clamp(v0 || 0, -this.closed * 2, this.closed * 2) : v0 || 0;
    if (reduced()) {
      this.x = target === 'open' ? 0 : this.closed;
      this.v = 0;
      this.render();
      this.finish(target);
      return;
    }
    this.mode = target === 'open' ? 'opening' : 'closing';
    this.goal = target === 'open' ? 0 : this.closed + 8;
    this.k = this.axis === 'x' ? 180 : 360;
    this.c = this.axis === 'x' ? 27 : 38;
    this.last = 0;
    this.raf = requestAnimationFrame(this.tick);
  }
  tick = (t) => {
    if (this.dead || !this.mode) return;
    const dt = frameDt(t, this.last);
    this.last = t;
    const s = springStep({ x: this.x, v: this.v }, this.goal, dt, this.k, this.c);
    this.x = s.x;
    this.v = s.v;
    const closing = this.mode === 'closing';
    const arrived = closing ? this.x >= this.closed : Math.abs(this.x - this.goal) < 0.3 && Math.abs(this.v) < 6;
    if (arrived) {
      this.x = closing ? this.closed : 0;
      this.v = 0;
      this.render();
      const target = closing ? 'closed' : 'open';
      this.mode = null;
      this.finish(target);
      return;
    }
    this.render();
    this.raf = requestAnimationFrame(this.tick);
  };
  finish(target) {
    if (target === 'closed') this.onClosed?.();
    else this.onOpened?.();
  }

  destroy() {
    this.dead = true;
    this.stop();
    this.stagger?.release();
  }
}

// Bridge between whoever detects the gesture (chat edge) and the panel that mounts after it:
// events arrive before React mounts the drawer, so they are queued and replayed on attach.
let current = null;
let queue = [];

export function pullEvent(type, finger) {
  if (current) apply(current, type, finger);
  else queue.push([type, finger]);
}
function apply(m, type, finger) {
  if (type === 'begin') m.begin(finger);
  else if (type === 'move') m.move(finger);
  else m.end();
}
export function attachMotion(m) {
  current = m;
  const pending = queue;
  queue = [];
  if (pending[0]?.[0] !== 'begin') return false;
  for (const [type, finger] of pending) apply(m, type, finger);
  return pending.length > 0;
}
export function detachMotion(m) {
  if (current === m) current = null;
  queue = [];
}
export function resetPull() {
  queue = [];
}
