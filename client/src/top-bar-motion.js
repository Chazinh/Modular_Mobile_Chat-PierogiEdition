// Chat top bar as a soap bubble: collapsed, it is only the status dot at the top center. When opening,
// the dot swells into a glass bubble, pops and releases the buttons, which fly from the center to their places.
// Everything is a function of a single progress p (0 = dot only, 1 = bar open) that follows the finger: you can
// hold it halfway, go back, pull past the end (rubber band) and let go; the spring decides open or closed. Like the side
// menu, only transform/opacity written straight to the DOM (no React state per frame).
import { clamp, frameDt, rubber, springStep, VelocityTracker } from './drawer-motion.js';

const RANGE = 130; // px of drag to go from 0 to 1
const BUBBLE = 64; // bubble diameter (px) at its peak, before popping
const GROW_END = 0.5; // up to here the dot only swells
const POP_END = 0.7; // pop: the bubble expands and fades, the ring spreads
const ease = (t) => 1 - Math.pow(1 - t, 3);
const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

export class TopBarMotion {
  constructor({ bar, handle, bubble, ring, onState }) {
    Object.assign(this, { bar, handle, bubble, ring, onState });
    this.p = 0;
    this.v = 0;
    this.raf = 0;
    this.frame = 0;
    this.drag = null;
    this.items = [];
    this.tracker = new VelocityTracker();
    this.dead = false;
    this.state = 'closed';
    this.render();
  }

  // Final button positions relative to the dot (measured without transforms, once per gesture).
  measure() {
    const items = Array.from(this.bar.querySelectorAll('[data-bar-item]'));
    for (const el of items) el.style.transform = '';
    const h = this.handle.getBoundingClientRect();
    const cx = h.left + h.width / 2, cy = h.top + h.height / 2;
    this.center = { x: cx, y: cy };
    const spread = Math.max(1, ...items.map((el) => { const r = el.getBoundingClientRect(); return Math.abs(r.left + r.width / 2 - cx); }));
    this.items = items.map((el) => {
      const r = el.getBoundingClientRect();
      const dx = cx - (r.left + r.width / 2), dy = cy - (r.top + r.height / 2);
      // whoever is closest to the center leaves first: the burst opens from the middle to the edges
      return { el, dx, dy, delay: (Math.abs(dx) / spread) * 0.14 };
    });
  }

  render() {
    const p = this.p;
    const grow = clamp(p / GROW_END, 0, 1);
    const pop = clamp((p - GROW_END) / (POP_END - GROW_END), 0, 1);
    // bubble: swells (with a slight spring breathing from the velocity), then pops by expanding and fading
    const wobble = 1 + clamp(this.v / 40, -0.08, 0.08);
    const size = (9 + (BUBBLE - 9) * ease(grow)) * (1 + pop * 0.55);
    this.bubble.style.transform = `translate3d(-50%,-50%,0) scale(${(size / BUBBLE * wobble).toFixed(4)},${(size / BUBBLE / wobble).toFixed(4)})`;
    this.bubble.style.opacity = (p <= 0.001 ? 0 : clamp(grow * 3, 0, 1) * (1 - pop)).toFixed(3);
    this.ring.style.transform = `translate3d(-50%,-50%,0) scale(${(1 + pop * 1.6).toFixed(4)})`;
    this.ring.style.opacity = (pop > 0 && pop < 1 ? (1 - pop) * 0.9 : 0).toFixed(3);
    // the dot becomes the bubble's glowing core and disappears when it pops
    this.handle.style.opacity = (1 - pop).toFixed(3);
    this.handle.style.transform = `scale(${(1 + grow * 0.6).toFixed(3)})`;
    // buttons: leave the bubble center for their places, growing; past 1 (rubber band) they stretch a little
    const over = Math.max(0, p - 1);
    for (const it of this.items) {
      const q = ease(clamp((p - POP_END + 0.08 - it.delay) / (1 - POP_END), 0, 1));
      const k = 1 - q;
      const s = 0.25 + 0.75 * q;
      const stretch = 1 + over * 0.25;
      it.el.style.transform = q >= 1 && over === 0 ? '' : `translate3d(${(it.dx * k).toFixed(2)}px,${(it.dy * k + over * 14).toFixed(2)}px,0) scale(${(s * stretch).toFixed(4)},${(s / stretch).toFixed(4)})`;
      it.el.style.opacity = q.toFixed(3);
    }
    this.bar.style.visibility = p <= 0.001 ? 'hidden' : '';
  }

  setState(state) {
    if (this.state === state) return;
    this.state = state;
    this.onState?.(state);
  }

  stop() {
    cancelAnimationFrame(this.raf);
    cancelAnimationFrame(this.frame);
    this.raf = 0;
    this.frame = 0;
  }

  // Finger moves are coalesced into one write per display frame.
  queueRender() {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      if (!this.dead) this.render();
    });
  }

  // ---- drag (same interface as PanelMotion, y axis) ----
  begin(finger) {
    this.stop();
    if (!this.items.length || this.p <= 0.001) this.measure();
    this.drag = { finger, p: this.p };
    this.tracker.reset();
    this.tracker.push(this.p * RANGE);
    this.setState('dragging');
  }
  move(finger) {
    if (!this.drag) return;
    const raw = this.drag.p + (finger - this.drag.finger) / RANGE;
    // past open it resists like rubber; below closed it stops
    this.p = raw > 1 ? 1 + rubber((raw - 1) * RANGE, 40) / RANGE : Math.max(0, raw);
    this.tracker.push(this.p * RANGE);
    this.v = this.tracker.velocity() / RANGE;
    this.queueRender();
  }
  end() {
    if (!this.drag) return;
    this.drag = null;
    const v = this.tracker.velocity() / RANGE; // progresso por segundo
    const target = this.p + v * 0.18 > 0.5 ? 1 : 0;
    this.settle(target, v);
  }

  open() {
    if (this.p <= 0.001) this.measure();
    this.settle(1, this.v);
  }
  close() {
    this.settle(0, this.v);
  }
  settle(goal, v0 = 0) {
    this.stop();
    if (reduced()) {
      this.p = goal;
      this.v = 0;
      this.render();
      this.setState(goal ? 'open' : 'closed');
      return;
    }
    this.goal = goal;
    this.v = v0;
    this.setState(goal ? 'opening' : 'closing');
    let last = 0;
    const tick = (now) => {
      if (this.dead) return;
      const dt = frameDt(now, last);
      last = now;
      // a slightly loose spring: overshoots and comes back, like a bubble
      const s = springStep({ x: this.p, v: this.v }, goal, dt, 170, goal ? 15 : 22);
      this.p = Math.max(0, s.x);
      this.v = s.v;
      if (Math.abs(this.p - goal) < 0.002 && Math.abs(this.v) < 0.02) {
        this.p = goal;
        this.v = 0;
        this.render();
        this.raf = 0;
        this.setState(goal ? 'open' : 'closed');
        return;
      }
      this.render();
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }

  destroy() {
    this.dead = true;
    this.stop();
  }
}
