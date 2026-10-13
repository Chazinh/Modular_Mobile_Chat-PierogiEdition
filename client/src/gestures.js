import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { EDGE_PX, pullEvent, resetPull } from './drawer-motion.js';
import { TopBarMotion } from './top-bar-motion.js';

// Both vertical semicircles accept diagonals: the vertical projection only needs to
// reach 50% of the path. The remaining band belongs to the side drawer.
export function gestureAxis(dx, dy) {
  const distance = Math.hypot(dx, dy);
  if (distance < 8) return null;
  return Math.abs(dy) / distance >= 0.5 ? 'vertical' : 'horizontal';
}

const isControl = target => target?.closest('input, textarea, select, button, a, [contenteditable="true"]');
const NO_DRAG = 'input, textarea, select, [contenteditable="true"], [data-no-drag]';

// Reading: scrolling is NATIVE (touch-action: pan-y, on the compositor thread, at the device refresh
// rate). The browser only takes gestures steeper than 45°; this function covers the 30°–45° band
// (vertical component ≥ 50% of the vector) by scrolling manually, and only when the browser is NOT scrolling by itself.
export function useReadingGesture(ref, enabled) {
  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;
    let g = null, frame = 0, velocity = 0;
    const stopGlide = () => { cancelAnimationFrame(frame); frame = 0; velocity = 0; };
    const down = e => {
      stopGlide();
      g = e.pointerType === 'touch' && e.isPrimary && !isControl(e.target)
        ? { id: e.pointerId, x: e.clientX, y: e.clientY, lastY: e.clientY, top: el.scrollTop, time: performance.now(), axis: null, stalled: 0, assist: false }
        : null;
    };
    const move = e => {
      if (!g || e.pointerId !== g.id) return;
      g.axis ||= gestureAxis(e.clientX - g.x, e.clientY - g.y);
      if (g.axis !== 'vertical') return;
      const now = performance.now();
      if (!g.assist) {
        if (el.scrollTop !== g.top) { g = null; return; } // the browser is already scrolling
        if (++g.stalled < 2) return; // waits for two moves without native scrolling
        g.assist = true;
      }
      const step = g.lastY - e.clientY;
      velocity = Math.max(-2.5, Math.min(2.5, step / Math.max(8, now - g.time)));
      el.scrollTop += step;
      g.lastY = e.clientY;
      g.time = now;
    };
    const end = () => {
      const assisted = g?.assist;
      const idle = g ? performance.now() - g.time : 0;
      g = null;
      if (!assisted) return;
      if (idle > 100) velocity = 0;
      let last = performance.now();
      const glide = now => {
        const dt = Math.min(32, now - last); last = now;
        const top = el.scrollTop;
        el.scrollTop += velocity * dt;
        velocity *= Math.pow(0.94, dt / 16);
        if (Math.abs(velocity) > 0.025 && el.scrollTop !== top) frame = requestAnimationFrame(glide);
      };
      frame = requestAnimationFrame(glide);
    };
    const cancel = () => { g = null; stopGlide(); };
    el.addEventListener('pointerdown', down, { passive: true });
    el.addEventListener('pointermove', move, { passive: true });
    el.addEventListener('pointerup', end);
    el.addEventListener('pointercancel', cancel);
    return () => {
      cancel();
      el.removeEventListener('pointerdown', down); el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', end); el.removeEventListener('pointercancel', cancel);
    };
  }, [ref, enabled]);
}

// Opening the side menu: ONLY by dragging from the dot along the right edge (EDGE_PX strip, like the Samsung
// Edge panel handle), inwards and more horizontally than vertically. The panel follows the finger.
export function useDrawerGesture(onOpen, enabled = true) {
  const ref = useRef(null), callback = useRef(onOpen), allowed = useRef(enabled);
  callback.current = onOpen;
  allowed.current = enabled; // the listeners stay attached: opening the drawer must not cut the drag in progress
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let g = null;
    const down = e => {
      g = allowed.current && e.pointerType === 'touch' && e.isPrimary && !isControl(e.target) && e.clientX >= window.innerWidth - EDGE_PX
        ? { id: e.pointerId, x: e.clientX, y: e.clientY, live: false } : null;
    };
    const move = e => {
      if (!g || e.pointerId !== g.id) return;
      if (!g.live) {
        const dx = e.clientX - g.x, dy = e.clientY - g.y;
        if (Math.hypot(dx, dy) < 8) return;
        if (!(dx < 0 && -dx > Math.abs(dy) * 1.2)) { g = null; return; } // vertical or outwards: not a drawer gesture
        if (-dx < 12) return;
        g.live = true;
        try { el.setPointerCapture(g.id); } catch { /* no capture: carry on without it */ }
        resetPull();
        pullEvent('begin', g.x); // the panel grabs where the finger touched
        callback.current();
      }
      pullEvent('move', e.clientX);
    };
    const up = e => {
      if (!g || e.pointerId !== g.id) return;
      const live = g.live;
      g = null;
      if (live) pullEvent('end');
      try { el.releasePointerCapture(e.pointerId); } catch { /* already released */ }
    };
    el.addEventListener('pointerdown', down, { passive: true });
    el.addEventListener('pointermove', move, { passive: true });
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
    return () => {
      resetPull();
      el.removeEventListener('pointerdown', down); el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up); el.removeEventListener('pointercancel', up);
    };
  }, []);
  return { ref };
}

// Dragging an already open panel (drawer: x axis; sheet: y axis): the panel follows the finger both ways,
// you can "play" moving back and forth with the finger held down; on release, the spring decides open or closed.
export function usePanelDrag(ref, motionRef, { axis = 'x', enabled = true } = {}) {
  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;
    let g = null;
    const pos = e => (axis === 'x' ? e.clientX : e.clientY);
    const down = e => {
      if (!e.isPrimary || (e.pointerType === 'mouse' && e.button !== 0) || e.target.closest?.(NO_DRAG)) { g = null; return; }
      g = { id: e.pointerId, x: e.clientX, y: e.clientY, live: false };
    };
    const move = e => {
      if (!g || e.pointerId !== g.id) return;
      if (!g.live) {
        const dx = e.clientX - g.x, dy = e.clientY - g.y;
        if (Math.hypot(dx, dy) < 8) return;
        const main = axis === 'x' ? Math.abs(dx) : Math.abs(dy), cross = axis === 'x' ? Math.abs(dy) : Math.abs(dx);
        const motion = motionRef.current;
        if (main < cross * 1.2 || !motion) { g = null; return; }
        g.live = true;
        try { el.setPointerCapture(g.id); } catch { /* no capture: carry on without it */ }
        motion.begin(axis === 'x' ? g.x : g.y);
      }
      motionRef.current?.move(pos(e));
    };
    const up = e => {
      if (!g || e.pointerId !== g.id) return;
      const live = g.live;
      g = null;
      if (live) motionRef.current?.end();
      try { el.releasePointerCapture(e.pointerId); } catch { /* already released */ }
    };
    el.addEventListener('pointerdown', down, { passive: true });
    el.addEventListener('pointermove', move, { passive: true });
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
    return () => {
      el.removeEventListener('pointerdown', down); el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up); el.removeEventListener('pointercancel', up);
      if (g?.live) motionRef.current?.end();
    };
  }, [ref, motionRef, axis, enabled]);
}


// Chat top bar as a bubble (top-bar-motion.js): collapsed, only the status dot remains. Tapping the
// dot opens it; dragging it down drives the bubble with the finger (swell, pop, release the buttons), and you
// can go back and forth while holding. Open, it closes by dragging the bar up, tapping the chat or opening
// a panel from it (see ChatView.jsx).
export function useTopBar() {
  const barRef = useRef(null), handleRef = useRef(null), bubbleRef = useRef(null), ringRef = useRef(null), motionRef = useRef(null);
  const [state, setState] = useState('closed');
  useLayoutEffect(() => {
    const motion = new TopBarMotion({ bar: barRef.current, handle: handleRef.current, bubble: bubbleRef.current, ring: ringRef.current, onState: setState });
    motionRef.current = motion;
    return () => { motion.destroy(); motionRef.current = null; };
  }, []);
  const show = useCallback(() => motionRef.current?.open(), []);
  const hide = useCallback(() => motionRef.current?.close(), []);
  useEffect(() => {
    const el = handleRef.current;
    if (!el) return undefined;
    let g = null;
    const down = e => {
      if (!e.isPrimary || (e.pointerType === 'mouse' && e.button !== 0)) return;
      g = { id: e.pointerId, y: e.clientY, live: false };
      try { el.setPointerCapture(g.id); } catch { /* no capture: carry on without it */ }
    };
    const move = e => {
      if (!g || e.pointerId !== g.id) return;
      if (!g.live) {
        if (Math.abs(e.clientY - g.y) < 6) return;
        g.live = true;
        motionRef.current?.begin(g.y);
      }
      motionRef.current?.move(e.clientY);
    };
    const up = e => {
      if (!g || e.pointerId !== g.id) return;
      const live = g.live;
      g = null;
      try { el.releasePointerCapture(e.pointerId); } catch { /* already released */ }
      if (live) { el.dataset.dragged = '1'; motionRef.current?.end(); }
    };
    el.addEventListener('pointerdown', down, { passive: true });
    el.addEventListener('pointermove', move, { passive: true });
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
    return () => {
      el.removeEventListener('pointerdown', down); el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up); el.removeEventListener('pointercancel', up);
    };
  }, []);
  const open = state !== 'closed';
  usePanelDrag(barRef, motionRef, { axis: 'y', enabled: state === 'open' });
  // a tap (no drag) on the dot opens it; the click left over from a drag is ignored
  const onHandleClick = useCallback(e => {
    if (e.currentTarget.dataset.dragged) { delete e.currentTarget.dataset.dragged; return; }
    show();
  }, [show]);
  return { barRef, handleRef, bubbleRef, ringRef, open, settled: state === 'open', show, hide, onHandleClick };
}
