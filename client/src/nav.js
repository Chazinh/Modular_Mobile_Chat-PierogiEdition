// Layered navigation: the Back button (Android) and the back gesture ALWAYS close the most recent
// overlay before leaving the screen. Each layer pushes one history entry.
//
// Robustness (the UI must never get "stuck" refusing Back):
//  - the depth is stored in every history entry (history.state.mcDepth); on popstate the app
//    closes everything ABOVE the depth reached, with no "events to ignore" counter that
//    could drift out of sync and swallow the next Back;
//  - closing a layer from code also closes the newer ones and performs ONE navigation (history.go);
//  - the APK Back button and the in-app back button close the top layer directly, without relying on the
//    WebView delivering popstate.
import { useEffect, useRef } from 'react';
import { createSharedListener } from './connection-lifetime.js';

const stack = [];
let syncTimer;

const depthOf = (state) => (state && Number.isInteger(state.mcDepth) ? state.mcDepth : 0);

function safeClose(layer) {
  try {
    layer.close();
  } catch {
    /* a failure closing one layer must never prevent closing the others */
  }
}

function goBack(steps) {
  // React may swap the menu for the editor in the same commit. Reconcile after the swap,
  // without firing a stale traversal that would close the newly opened layer.
  clearTimeout(syncTimer);
  syncTimer = setTimeout(() => {
    const n = Math.min(steps, Math.max(0, depthOf(history.state) - stack.length));
    if (n > 0) history.go(-n);
  }, 0);
}

// Closes the layer and every layer opened after it (the newer ones are visually on top).
function dismissLayer(layer) {
  if (!layer.active) return;
  const i = stack.indexOf(layer);
  layer.active = false;
  if (i < 0) return;
  const removed = stack.splice(i);
  for (const l of removed.slice(1).reverse()) {
    l.active = false;
    safeClose(l);
  }
  goBack(depthOf(history.state));
}

export function openLayer(close) {
  const layer = { close, active: true };
  stack.push(layer);
  if (depthOf(history.state) === stack.length) {
    history.replaceState({ mcDepth: stack.length }, '');
  } else {
    history.pushState({ mcDepth: stack.length }, '');
  }
  return function dismiss() {
    dismissLayer(layer);
  };
}

export const layerCount = () => stack.length;

// Closes the top layer (in-app and APK Back button). Returns false when no layer is open.
export function closeTopLayer() {
  const layer = stack[stack.length - 1];
  if (!layer) return false;
  dismissLayer(layer);
  safeClose(layer);
  return true;
}

if (typeof window !== 'undefined') {
  window.addEventListener('popstate', (event) => {
    const target = depthOf(event.state ?? history.state);
    while (stack.length > target) {
      const layer = stack.pop();
      layer.active = false;
      safeClose(layer);
    }
  });
}

// Registers the APK hardware Back button: closes layers; with none, minimizes the app.
export const installNativeBack = createSharedListener(async () => {
  if (!window.Capacitor?.isNativePlatform?.()) return;
  const { App } = await import('@capacitor/app');
  return App.addListener('backButton', () => {
    if (!closeTopLayer()) App.minimizeApp();
  });
});

// While `open` is true, keeps one layer active; Back calls onClose.
export function useLayer(open, onClose) {
  const ref = useRef(onClose);
  ref.current = onClose;
  useEffect(() => {
    if (!open) return undefined;
    const dismiss = openLayer(() => ref.current());
    return dismiss;
  }, [open]);
}
