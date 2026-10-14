// Frame and temperature measurement on the phone (adb), for the "perf" copy of the app (scripts/build-perf-android.ps1).
// Two scenarios, both with real touch input, then reads gfxinfo (frames, janky frames, percentiles) and the
// battery/AP/skin temperatures before and after. Open a chat in the app first and keep the screen on.
//   drawer (default): pulls the side menu from the right edge and closes it, over and over (the heating test)
//   bar: pulls the chat's top bar down from the status dot and closes it by tapping the chat
//   scroll: swipes the chat list up and down
// Usage: node scripts/measure-device-perf.mjs [label] [out.json]   (MC_PERF_MODE=drawer|bar|scroll)
//   env: ADB (adb path), MC_PERF_PACKAGE (default io.github.modularchat.pierogiedition.perf), MC_PERF_SWIPES (default 80),
//        MC_PERF_SWIPE_MS (swipe duration, default 280), MC_PERF_PAUSE_MS (pause between swipes, default 350)
//        MC_PERF_CYCLES (drawer open/close cycles, default 60), MC_PERF_OPEN_HOLD_MS (menu stays open, default 600)
// Notes: gfxinfo only counts frames the app drew; on an idle screen the numbers are tiny. Compare runs made with the
// same chat, same screen brightness and a similar starting temperature (let the phone cool between runs).
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const env = process.env;
const adb = env.ADB || path.join(os.homedir(), 'RoleplayChat-local', 'android-sdk', 'platform-tools', 'adb.exe');
const pkg = env.MC_PERF_PACKAGE || 'io.github.modularchat.pierogiedition.perf';
const mode = ['scroll', 'bar'].includes(env.MC_PERF_MODE) ? env.MC_PERF_MODE : 'drawer';
const swipes = Number(env.MC_PERF_SWIPES || 80);
const cycles = Number(env.MC_PERF_CYCLES || 60);
const holdMs = Number(env.MC_PERF_OPEN_HOLD_MS || 600);
const swipeMs = Number(env.MC_PERF_SWIPE_MS || 280);
const pauseMs = Number(env.MC_PERF_PAUSE_MS || 350);
const label = process.argv[2] || 'run';
const outFile = process.argv[3];

const sh = (cmd) => execFileSync(adb, ['shell', cmd], { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, windowsHide: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function temperatures() {
  const out = { batteryC: null, apC: null, skinC: null };
  const bat = sh('dumpsys battery').match(/temperature:\s*(\d+)/);
  if (bat) out.batteryC = Number(bat[1]) / 10;
  for (const m of sh('dumpsys thermalservice').matchAll(/Temperature\{mValue=([\d.]+), mType=\d+, mName=(AP|SKIN)/g)) {
    // the service lists current values first, then thresholds: keep the first reading of each sensor
    const key = m[2] === 'AP' ? 'apC' : 'skinC';
    if (out[key] == null) out[key] = Number(m[1]);
  }
  return out;
}

function gfx() {
  const text = sh(`dumpsys gfxinfo ${pkg}`);
  const num = (re) => { const m = text.match(re); return m ? Number(m[1]) : null; };
  const total = num(/Total frames rendered:\s*(\d+)/);
  const janky = num(/Janky frames:\s*(\d+)/);
  return {
    totalFrames: total,
    jankyFrames: janky,
    jankyPercent: total ? Math.round((janky / total) * 1000) / 10 : null,
    p50ms: num(/50th percentile:\s*(\d+)ms/),
    p90ms: num(/90th percentile:\s*(\d+)ms/),
    p95ms: num(/95th percentile:\s*(\d+)ms/),
    p99ms: num(/99th percentile:\s*(\d+)ms/),
    slowUiThread: num(/Number Slow UI thread:\s*(\d+)/),
    slowBitmapUploads: num(/Number Slow bitmap uploads:\s*(\d+)/),
    slowIssueDraw: num(/Number Slow issue draw commands:\s*(\d+)/),
    missedVsync: num(/Number Missed Vsync:\s*(\d+)/),
    frameDeadlineMissed: num(/Number Frame deadline missed:\s*(\d+)/),
  };
}

const size = sh('wm size').match(/(\d+)x(\d+)/);
const [w, h] = [Number(size[1]), Number(size[2])];
const x = Math.round(w / 2);
const top = Math.round(h * 0.3);
const bottom = Math.round(h * 0.7);
const focus = sh('dumpsys window').match(/mCurrentFocus=.*?(\S+)\}/)?.[1] || '';
if (!focus.includes(pkg)) throw new Error(`The app ${pkg} is not in the foreground (focus: ${focus || 'none'}). Open it on a long chat first.`);

sh(`dumpsys gfxinfo ${pkg} reset`);
const before = temperatures();
const started = Date.now();
if (mode === 'scroll') {
  for (let i = 0; i < swipes; i++) {
    // alternate direction so the chat keeps moving without hitting the end for long
    const [from, to] = i % 2 === 0 ? [bottom, top] : [top, bottom];
    sh(`input swipe ${x} ${from} ${x} ${to} ${swipeMs}`);
    await sleep(pauseMs);
  }
} else if (mode === 'bar') {
  // The status dot sits at the top center (just under the camera cut-out); pulling it down 130 css px opens the bar.
  // Tapping the chat closes it again with its own animation.
  const density = Number((sh('wm density').match(/(\d+)\s*$/m) || [0, 450])[1]) / 160;
  const dotY = Math.round(h * 0.06);
  for (let i = 0; i < cycles; i++) {
    sh(`input swipe ${x} ${dotY} ${x} ${dotY + Math.round(150 * density)} ${swipeMs}`);
    await sleep(holdMs);
    sh(`input tap ${x} ${Math.round(h * 0.6)}`);
    await sleep(pauseMs + 250);
  }
} else {
  // The drawer opens by dragging inwards from the strip along the right edge (20 css px); BACK closes it with its
  // own animation. Each cycle = one pull open + one close.
  const edge = w - 8;
  const mid = Math.round(h * 0.5);
  for (let i = 0; i < cycles; i++) {
    sh(`input swipe ${edge} ${mid} ${Math.round(w * 0.2)} ${mid} ${swipeMs}`);
    await sleep(holdMs);
    sh('input keyevent KEYCODE_BACK');
    await sleep(pauseMs + 250);
  }
}
const seconds = Math.round((Date.now() - started) / 100) / 10;
const result = { label, mode, package: pkg, swipes: mode === 'scroll' ? swipes : cycles, seconds, before, after: temperatures(), frames: gfx(), at: new Date().toISOString() };
console.log(JSON.stringify(result, null, 2));
if (outFile) fs.writeFileSync(outFile, JSON.stringify(result, null, 2));
