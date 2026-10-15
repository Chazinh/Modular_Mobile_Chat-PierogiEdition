import { flushSync } from 'react-dom';
import { TopBarMotion, TOP_BAR_POP_END, TOP_BAR_OPEN_SPRING, topBarPopProgress } from './top-bar-motion.js';
import { springStep } from './drawer-motion.js';

let current = null;
let revision = 0;

const boxCurve = [0];
let boxState = { x: 0, v: 0 };
for (let i = 0; i < 120; i++) {
  boxState = springStep(boxState, 1, 1 / 60, TOP_BAR_OPEN_SPRING.k, TOP_BAR_OPEN_SPRING.c);
  boxCurve.push(boxState.x);
  if (Math.abs(boxState.x - 1) < .002 && Math.abs(boxState.v) < .02) break;
}
boxCurve[boxCurve.length - 1] = 1;

function soapElements(origin) {
  const make = (className, faceClass) => {
    const el = document.createElement('span');
    el.className = className; el.setAttribute('aria-hidden', 'true');
    Object.assign(el.style, { left: `${origin.x}px`, top: `${origin.y}px`, zIndex: '120' });
    const face = document.createElement('span'); face.className = faceClass;
    el.append(face); document.body.append(el); return el;
  };
  return { bubble: make('mc-bar-bubble', 'mc-glass-pill mc-bar-bubble-face absolute inset-0'), ring: make('mc-bar-ring', 'mc-bar-ring-face') };
}

function expandChatBox(update, rect, source, token) {
  const width = window.innerWidth, height = window.innerHeight;
  const r = rect || { left: width / 2 - 34, top: 48, width: 68, height: 44 };
  const radius = source?.radius ?? (source ? parseFloat(getComputedStyle(source).borderTopLeftRadius) || 26 : 26);
  flushSync(update);
  const page = document.querySelector('.mc-chat');
  if (!page) return;
  const old = { transform: page.style.transform, transformOrigin: page.style.transformOrigin, overflow: page.style.overflow, borderRadius: page.style.borderRadius, boxShadow: page.style.boxShadow };
  const frames = boxCurve.map(p => {
    const sx = r.width / width + (1 - r.width / width) * p;
    const sy = r.height / height + (1 - r.height / height) * p;
    const pop = topBarPopProgress(p);
    const rim = Math.min(1, p * 2) * (1 - pop);
    const corner = radius * (1 - pop);
    // The expanding chat itself is the bubble. Its rim breaks at the shared pop
    // point; there is no separate circle, image or ring above the conversation.
    return {
      transform: `translate3d(${r.left * (1-p)}px,${r.top * (1-p)}px,0) scale(${sx},${sy})`,
      borderRadius: `${corner / sx}px / ${corner / sy}px`,
      boxShadow: `inset 0 0 0 ${1 + pop}px rgb(calc(219 * var(--mc-tr, 1)) calc(206 * var(--mc-tg, 1)) calc(251 * var(--mc-tb, 1)) / ${rim * .42}), inset 0 0 ${6 + pop * 8}px rgb(calc(170 * var(--mc-tr, 1)) calc(207 * var(--mc-tg, 1)) calc(227 * var(--mc-tb, 1)) / ${rim * .2})`,
    };
  });
  page.style.transformOrigin = '0 0';
  page.style.overflow = 'hidden';
  page.style.transform = frames[0].transform;
  page.style.borderRadius = frames[0].borderRadius;
  page.style.boxShadow = frames[0].boxShadow;
  const duration = (boxCurve.length - 1) * 1000 / 60;
  const reveal = Math.max(0, boxCurve.findIndex(p => p >= TOP_BAR_POP_END)) * 1000 / 60;
  const animation = page.animate(frames, { duration, easing: 'linear' });
  const children = [...page.querySelectorAll(':scope > .mc-scroll, .mc-composer-shell, :scope > .mc-bar-handle')];
  const content = children.map(el => el.animate([{ opacity: 0 }, { opacity: 1 }], { delay: reveal, duration: Math.max(1, duration - reveal), fill: 'backwards', easing: 'ease-out' }));
  const clean = () => {
    Object.assign(page.style, old);
    page.querySelector('.mc-scroll')?.dispatchEvent(new Event('scroll'));
    if (current?.token === token) current = null;
  };
  current = { token, cancel: () => { animation.cancel(); content.forEach(a => a.cancel()); clean(); } };
  animation.finished.catch(() => {}).finally(clean);
}

// The same bubble, pop ring and release springs used by the chat's top bar.
// The original button is the handle; no icon/image is cloned.
export function transitionTo(update, selector, source) {
  const token = ++revision;
  current?.cancel();
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) { update(); return; }
  const rect = source?.getBoundingClientRect?.();
  if (selector === '.mc-chat') { expandChatBox(update, rect, source, token); return; }
  const origin = { x: rect ? rect.left + rect.width / 2 : window.innerWidth / 2, y: rect ? rect.top + rect.height / 2 : 48 };
  const handle = source?.style ? source : { style: {}, getBoundingClientRect: () => ({ left: origin.x, top: origin.y, width: 0, height: 0 }) };
  const old = { transform: handle.style.transform, opacity: handle.style.opacity, transition: handle.style.transition };
  handle.style.transition = 'none';
  const { bubble, ring } = soapElements(origin);
  let groups = [], committed = false, motion;
  const bar = { style: {}, querySelectorAll: () => groups };
  const clean = () => {
    bubble.remove(); ring.remove();
    Object.assign(handle.style, old);
    for (const el of groups) { el.style.transform = ''; el.style.opacity = ''; }
    if (current?.token === token) current = null;
  };
  const collect = () => {
    const page = [...document.querySelectorAll(selector)].at(-1);
    if (!page) return [];
    if (selector === '.mc-chat') return [...page.querySelectorAll(':scope > .mc-scroll > .mc-col, .mc-composer-shell')];
    return [...page.querySelectorAll(':scope > header > .mc-col, :scope > .overflow-y-auto > .mc-col')];
  };
  motion = new TopBarMotion({
    bar, handle, bubble, ring, origin,
    onProgress: p => {
      if (!committed && p >= TOP_BAR_POP_END && token === revision) {
        committed = true;
        flushSync(update);
        groups = collect();
        motion.measure();
      }
    },
    onState: state => { if (state === 'open') { motion.destroy(); clean(); } },
  });
  current = { token, cancel: () => { motion.destroy(); clean(); } };
  motion.open();
}
